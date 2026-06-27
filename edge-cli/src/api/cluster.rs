//! Cluster-admin types and endpoints (issue #85).
//!
//! Two routes back this client:
//!   - `GET /api/v1/admin/cluster`        → `ClusterView`
//!   - `GET /api/v1/admin/cluster/events` → `AutoscaleEventList`
//!
//! The wire shapes mirror the Go types in
//! `edge-control-plane/internal/service/cluster.go` field-for-field,
//! and the OpenAPI spec at `docs/api/openapi.yaml:769-810`.
//!
//! `ClusterClient` borrows the parent `ApiClient` so the API-key +
//! base_url are shared with the rest of the CLI without cloning the
//! underlying HTTP client. Same pattern as `DomainClient`.

use anyhow::{Context, Result};
use std::collections::BTreeMap;

use super::client::{check_response, ApiClient};

/// Per-region snapshot of the cluster. Returned by
/// `GET /api/v1/admin/cluster`. Mirrors the Go `service.ClusterView`.
#[derive(Debug, serde::Deserialize)]
pub struct ClusterView {
    pub generated_at: String,
    /// Map of region name → region view. BTreeMap (not HashMap) so
    /// iteration order is deterministic — the CLI prints regions
    /// alphabetically and tests can assert a stable ordering without
    /// having to sort.
    pub regions: BTreeMap<String, RegionView>,
}

/// Per-region breakdown. Mirrors the Go `service.RegionView`.
#[derive(Debug, serde::Deserialize)]
pub struct RegionView {
    pub workers: Vec<WorkerStatus>,
    pub apps_per_worker_avg: i64,
}

/// Per-worker projection. Mirrors the Go `service.WorkerView`. `ip`
/// is `Option<String>` because the server omits the field for
/// workers that haven't reported an address yet.
#[derive(Debug, serde::Deserialize)]
pub struct WorkerStatus {
    pub worker_id: String,
    pub region: String,
    #[serde(default)]
    pub ip: Option<String>,
    pub last_seen: String,
    pub app_count: i64,
    pub memory_mb: i64,
}

/// One row of the `autoscale_events` table. Mirrors the Go
/// `domain.AutoscaleEvent`. The `action` field is one of
/// `scale_up` / `scale_down` / `noop` — the Go CHECK constraint
/// enforces the same set server-side, so a value outside this list
/// arriving here is a server bug.
#[derive(Debug, serde::Deserialize)]
pub struct AutoscaleEvent {
    pub id: i64,
    pub created_at: String,
    pub region: String,
    pub action: String,
    pub from_count: i64,
    pub to_count: i64,
    pub reason: String,
    pub provider_kind: String,
    pub succeeded: bool,
    #[serde(default)]
    pub error_message: Option<String>,
}

/// Envelope for `GET /api/v1/admin/cluster/events`. The `region`
/// field echoes back the filter (or is null when listing all
/// regions) so a CLI paginating across calls can verify what was
/// actually applied without re-parsing query state.
#[derive(Debug, serde::Deserialize)]
pub struct AutoscaleEventList {
    pub items: Vec<AutoscaleEvent>,
    pub limit: i64,
    #[serde(default)]
    pub region: Option<String>,
}

/// Borrowed accessor for the cluster-admin endpoints. Constructed via
/// `ApiClient::cluster()`.
pub struct ClusterClient<'a> {
    pub(crate) client: &'a ApiClient,
}

impl<'a> ClusterClient<'a> {
    /// GET `/api/v1/admin/cluster` — returns the current per-region
    /// snapshot. Both this call and the events endpoint require the
    /// caller to be authenticated with the `owner` role; the
    /// control plane rejects other roles with 403.
    pub fn view(&self) -> Result<ClusterView> {
        let url = format!("{}/api/v1/admin/cluster", self.client.base_url());
        let resp = self
            .client
            .http()
            .get(&url)
            .header("Authorization", self.client.auth_header())
            .send()
            .context("GET /api/v1/admin/cluster")?;
        let resp = check_response(resp).context("cluster view request failed")?;
        resp.json().context("decoding cluster view response")
    }

    /// GET `/api/v1/admin/cluster/events` — returns the most-recent
    /// `autoscale_events` rows, newest first. Both `region` and
    /// `limit` are optional:
    ///   - `region = None` → list across all regions
    ///   - `limit = None`  → server default (50, clamped to [1, 500])
    pub fn events(&self, region: Option<&str>, limit: Option<u32>) -> Result<AutoscaleEventList> {
        let mut parsed = reqwest::Url::parse(&format!(
            "{}/api/v1/admin/cluster/events",
            self.client.base_url()
        ))
        .context("invalid base url")?;
        if let Some(r) = region {
            if !r.is_empty() {
                parsed.query_pairs_mut().append_pair("region", r);
            }
        }
        if let Some(n) = limit {
            if n > 0 {
                parsed
                    .query_pairs_mut()
                    .append_pair("limit", &n.to_string());
            }
        }
        let url = parsed.to_string();
        let resp = self
            .client
            .http()
            .get(&url)
            .header("Authorization", self.client.auth_header())
            .send()
            .context("GET /api/v1/admin/cluster/events")?;
        let resp = check_response(resp).context("cluster events request failed")?;
        resp.json().context("decoding cluster events response")
    }
}
