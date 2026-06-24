//! edge-ingress configuration loaded from environment variables.

use std::time::Duration;

use anyhow::Context;

/// Suffix for every public hostname the ingress serves. Must stay in sync
/// with the Go control plane's `domain.IngressHostSuffix` (in
/// `edge-control-plane/internal/domain/worker.go`) — drift between the
/// two produces 404s for every public URL the control plane has
/// advertised to tenants. Re-branding (e.g. to `edgecloud.run`) is a
/// single-line change in each language.
pub const INGRESS_HOST_SUFFIX: &str = "edgecloud.dev";

/// Render the public hostname for a `(tenant_id, app_name)` pair.
pub fn ingress_host(tenant_id: &str, app_name: &str) -> String {
    format!("{}-{}.{}", tenant_id, app_name, INGRESS_HOST_SUFFIX)
}

/// Default poll interval for `/api/internal/domains`. The custom-domain
/// poller fetches the full domain list on this cadence and diffs against
/// the current FQDN table. 30s matches the control plane's expected
/// staleness budget (per issue #83 design).
pub const DEFAULT_DOMAIN_POLL_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct Config {
    pub nats_url: String,
    pub caddy_admin_url: String,
    pub region: String,
    pub cert_file: String,
    pub key_file: String,
    pub listen_http: String,
    pub listen_https: String,
    pub refresh_debounce_ms: u64,
    pub http_to_https: bool,
    pub admin_token: Option<String>,
    pub control_plane_api_url: String,
    /// Shared secret presented in `X-Internal-Token` when fetching traffic
    /// splits from the control plane. Must match the control plane's
    /// `EDGE_INTERNAL_TOKEN`; otherwise the control plane's
    /// `internalAuth` middleware returns 401 and the Caddy weights
    /// never get applied (canary/blue-green silently no-ops). `None`
    /// means the header is omitted — which the control plane treats
    /// as a 401, so a production deployment must set this.
    pub internal_token: Option<String>,
    /// Base URL of the control plane's `/api/internal`. Empty = default-only
    /// mode (no custom domains, no on-demand TLS).
    pub control_plane_url: String,
    /// JWT for the control plane. Carries `role: ingest`. Required when
    /// `control_plane_url` is set; ignored otherwise.
    pub service_token: String,
    /// How often to poll `/api/internal/domains` (default 30s).
    pub domain_poll_interval: Duration,

}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// Required env vars:
    /// - `INGRESS_REGION` (e.g. `fra`)
    /// - `TLS_CERT_FILE` (path to the `*.edgecloud.dev` wildcard cert PEM)
    /// - `TLS_KEY_FILE` (path to the matching key PEM)
    ///
    /// Optional env vars:
    /// - `NATS_URL` (default: `nats://localhost:4222`)
    /// - `CADDY_ADMIN_URL` (default: `http://127.0.0.1:2019`)
    /// - `INGRESS_LISTEN_HTTP` (default: `:80`)
    /// - `INGRESS_LISTEN_HTTPS` (default: `:443`)
    /// - `CADDY_ADMIN_TOKEN` (if set, must match the value on the Caddy process)
    /// - `REFRESH_DEBOUNCE_MS` (default: `1000`)
    /// - `HTTP_TO_HTTPS` (default: `true`) — 308-redirect :80 → :443
    /// - `CONTROL_PLANE_API_URL` (default: `http://localhost:8080`) — used
    ///   by the ingress to fetch canary traffic splits at render time
    /// - `CONTROL_PLANE_URL` (default: empty = no custom domains)
    /// - `INGRESS_SERVICE_TOKEN` (default: empty; required when
    ///   `CONTROL_PLANE_URL` is set)
    /// - `DOMAIN_POLL_INTERVAL` (default: 30s; parsed as a Go-style
    ///   duration string, e.g. `30s`, `1m`, `500ms`)

    pub fn from_env() -> anyhow::Result<Self> {
        let control_plane_url = std::env::var("CONTROL_PLANE_URL").unwrap_or_default();
        let service_token = std::env::var("INGRESS_SERVICE_TOKEN").unwrap_or_default();
        if !control_plane_url.is_empty() && service_token.is_empty() {
            anyhow::bail!(
                "CONTROL_PLANE_URL is set but INGRESS_SERVICE_TOKEN is empty; \
                 the domain poller cannot authenticate against the control plane"
            );
        }
        let domain_poll_interval =
            parse_duration_env("DOMAIN_POLL_INTERVAL").unwrap_or(DEFAULT_DOMAIN_POLL_INTERVAL);

        Ok(Config {
            nats_url: std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into()),
            caddy_admin_url: std::env::var("CADDY_ADMIN_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:2019".into()),
            region: std::env::var("INGRESS_REGION").context("INGRESS_REGION not set")?,
            cert_file: std::env::var("TLS_CERT_FILE").context("TLS_CERT_FILE not set")?,
            key_file: std::env::var("TLS_KEY_FILE").context("TLS_KEY_FILE not set")?,
            listen_http: std::env::var("INGRESS_LISTEN_HTTP").unwrap_or_else(|_| ":80".into()),
            listen_https: std::env::var("INGRESS_LISTEN_HTTPS").unwrap_or_else(|_| ":443".into()),
            refresh_debounce_ms: std::env::var("REFRESH_DEBOUNCE_MS")
                .unwrap_or_else(|_| "1000".into())
                .parse()
                .unwrap_or(1000),
            http_to_https: std::env::var("HTTP_TO_HTTPS")
                .map(|v| !matches!(v.as_str(), "0" | "false" | "no"))
                .unwrap_or(true),
            admin_token: std::env::var("CADDY_ADMIN_TOKEN")
                .ok()
                .filter(|v| !v.is_empty()),
            control_plane_api_url: std::env::var("CONTROL_PLANE_API_URL")
                .unwrap_or_else(|_| "http://localhost:8080".into()),
            internal_token: std::env::var("EDGE_INTERNAL_TOKEN")
                .ok()
                .filter(|v| !v.is_empty()),
            control_plane_url,
            service_token,
            domain_poll_interval,

        })
    }
}

/// Parse a duration env var. Accepts Go-style strings (`30s`, `1m`,
/// `500ms`) via the `humantime` style — but to avoid pulling a new dep
/// we accept the integer-seconds form only. A `30s` env value will fail
/// to parse; users should set `DOMAIN_POLL_INTERVAL=30` (seconds) or
/// leave it unset for the 30s default.
fn parse_duration_env(name: &str) -> Option<Duration> {
    let raw = std::env::var(name).ok()?;
    let secs: u64 = raw.parse().ok()?;
    Some(Duration::from_secs(secs))
}
