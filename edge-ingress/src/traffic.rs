//! Traffic split cache — fetched from the control plane API.
//!
//! The ingress periodically fetches traffic splits for all known
//! `(tenant_id, app_name)` pairs and caches them. The cache is consulted
//! at render time to override the heartbeat-derived weight with the
//! authoritative split from the control plane DB.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use tracing::{debug, warn};

/// A traffic split for one app: deployment_id → weight.
pub type DeploymentWeights = HashMap<String, u8>;

/// Key identifying a traffic split scope.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AppKey {
    pub tenant_id: String,
    pub app_name: String,
}

/// All cached traffic splits, keyed by (tenant_id, app_name).
#[derive(Default)]
pub struct TrafficSplitCache {
    /// Cached splits, updated periodically from the control plane.
    inner: HashMap<AppKey, DeploymentWeights>,
    /// When each entry was last fetched (for TTL eviction).
    fetched_at: HashMap<AppKey, Instant>,
}

/// TTL for cached splits before we re-fetch.
const CACHE_TTL: Duration = Duration::from_secs(30);

impl TrafficSplitCache {
    /// Get the weight for a specific deployment within an app's split.
    /// Returns `None` if the split is not cached or the deployment is not found.
    pub fn weight(&self, tenant_id: &str, app_name: &str, deployment_id: &str) -> Option<u8> {
        let key = AppKey {
            tenant_id: tenant_id.to_string(),
            app_name: app_name.to_string(),
        };
        self.inner.get(&key)?
            .get(deployment_id).copied()
    }

    /// Returns true if the cache has a split for this app and it's not stale.
    pub fn has_split(&self, tenant_id: &str, app_name: &str) -> bool {
        let key = AppKey {
            tenant_id: tenant_id.to_string(),
            app_name: app_name.to_string(),
        };
        matches!(
            self.fetched_at.get(&key),
            Some(instant) if instant.elapsed() < CACHE_TTL
        )
    }

    /// Update the cache with a new set of splits for an app.
    pub fn update(&mut self, tenant_id: String, app_name: String, weights: DeploymentWeights) {
        let key = AppKey { tenant_id, app_name };
        self.inner.insert(key.clone(), weights);
        self.fetched_at.insert(key, Instant::now());
    }

    /// Remove stale entries (TTL expired).
    pub fn evict_stale(&mut self) {
        let stale: Vec<AppKey> = self
            .fetched_at
            .iter()
            .filter(|(_, instant)| instant.elapsed() >= CACHE_TTL)
            .map(|(k, _)| k.clone())
            .collect();
        for key in stale {
            self.inner.remove(&key);
            self.fetched_at.remove(&key);
        }
    }

    /// Get the list of all known (tenant_id, app_name) pairs in the cache.
    pub fn known_apps(&self) -> Vec<(String, String)> {
        self.inner.keys().map(|k| (k.tenant_id.clone(), k.app_name.clone())).collect()
    }
}

/// Fetch traffic splits for a specific app from the control plane API.
async fn fetch_app_split(
    http: &reqwest::Client,
    api_url: &str,
    tenant_id: &str,
    app_name: &str,
) -> Option<DeploymentWeights> {
    let url = format!("{}/api/v1/apps/{}/traffic", api_url, app_name);
    #[derive(serde::Deserialize)]
    struct SplitEntry {
        deployment_id: String,
        weight: u8,
    }
    #[derive(serde::Deserialize)]
    struct TrafficResponse {
        splits: Vec<SplitEntry>,
    }

    let resp = match http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            warn!(tenant = %tenant_id, app = %app_name, err = %e, "failed to fetch traffic split");
            return None;
        }
    };
    if !resp.status().is_success() {
        warn!(tenant = %tenant_id, app = %app_name, status = %resp.status(), "traffic split fetch returned non-2xx");
        return None;
    }
    let body: TrafficResponse = match resp.json().await {
        Ok(b) => b,
        Err(e) => {
            warn!(tenant = %tenant_id, app = %app_name, err = %e, "failed to parse traffic split response");
            return None;
        }
    };
    let weights: DeploymentWeights = body
        .splits
        .into_iter()
        .map(|s| (s.deployment_id, s.weight))
        .collect();
    debug!(tenant = %tenant_id, app = %app_name, count = %weights.len(), "fetched traffic split");
    Some(weights)
}

/// Shared handle to the traffic split cache.
pub type SharedCache = Arc<RwLock<TrafficSplitCache>>;

/// Spawn a background task that periodically re-fetches traffic splits for
/// all known apps. It also periodically removes stale cache entries.
pub fn spawn_fetcher(http: reqwest::Client, api_url: String, cache: SharedCache) {
    tokio::spawn(async move {
        let fetch_interval = Duration::from_secs(30);
        let mut ticker = tokio::time::interval(fetch_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let apps: Vec<(String, String)> = {
                let mut cache = cache.write().await;
                cache.evict_stale();
                cache.known_apps()
            };
            for (tenant_id, app_name) in apps {
                let weights = fetch_app_split(&http, &api_url, &tenant_id, &app_name).await;
                if let Some(weights) = weights {
                    let mut cache = cache.write().await;
                    cache.update(tenant_id.clone(), app_name.clone(), weights);
                }
            }
        }
    });
}
