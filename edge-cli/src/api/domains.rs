//! Custom-domain types and endpoints (issue #83).
//!
//! Five routes on the control plane back this client. The handler
//! contract is documented in `edge-control-plane/internal/handler/domain.go`;
//! the wire shape mirrors the Go `domain.Domain` struct exactly so a
//! serde Deserialize on this end cannot drift from the JSON the
//! server emits.
//!
//! `DomainClient` borrows the parent `ApiClient` so the API-key +
//! base_url are shared across all subcommands without cloning the
//! underlying HTTP client (which is already internally `Arc`-shared
//! by reqwest). The methods are the only path that hits
//! `/api/v1/apps/{app}/domains*`; tests stub the wiremock at those
//! paths.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// One row of the `domains` table as seen by the tenant. Mirrors the
/// Go `domain.Domain` struct field-for-field. The `verified_at` and
/// `last_error` fields are nullable on the server; both are
/// represented as `Option<…>` here so a missing field deserializes
/// cleanly instead of erroring.
///
/// `status` is server-driven (pending/active/failed). Tenants can
/// observe but cannot mutate it; the CLI's `check` subcommand is
/// the only place this struct is read.
#[derive(Debug, Deserialize, Serialize)]
pub struct Domain {
    pub id: String,
    pub tenant_id: String,
    pub app_name: String,
    pub fqdn: String,
    pub status: String,
    #[serde(default)]
    pub last_error: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub verified_at: Option<String>,
}

/// Borrowed accessor for the custom-domain endpoints. Constructed via
/// `ApiClient::domains()`.
pub struct DomainClient<'a> {
    pub(crate) client: &'a super::client::ApiClient,
}

impl<'a> DomainClient<'a> {
    /// Bind a custom FQDN to an existing app. Returns the new row.
    pub fn add(&self, app: &str, fqdn: &str) -> Result<Domain> {
        let url = format!("{}/api/v1/apps/{}/domains", self.client.base_url(), app);
        let resp = self
            .client
            .http()
            .post(&url)
            .header("Authorization", self.client.auth_header())
            .json(&json!({ "fqdn": fqdn }))
            .send()
            .context("POST /api/v1/apps/{app}/domains")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("add domain failed: {status} {body}");
        }

        let body = resp.text().context("reading add-domain response body")?;
        serde_json::from_str(&body).context("decoding add-domain response")
    }

    /// List all custom FQDNs bound to the app.
    pub fn list(&self, app: &str) -> Result<Vec<Domain>> {
        let url = format!("{}/api/v1/apps/{}/domains", self.client.base_url(), app);
        let resp = self
            .client
            .http()
            .get(&url)
            .header("Authorization", self.client.auth_header())
            .send()
            .context("GET /api/v1/apps/{app}/domains")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("list domains failed: {status} {body}");
        }

        let body = resp.text().context("reading list-domains response body")?;
        serde_json::from_str(&body).context("decoding list-domains response")
    }

    /// Fetch a single row by (app, fqdn).
    pub fn get(&self, app: &str, fqdn: &str) -> Result<Domain> {
        let url = format!(
            "{}/api/v1/apps/{}/domains/{}",
            self.client.base_url(),
            app,
            fqdn
        );
        let resp = self
            .client
            .http()
            .get(&url)
            .header("Authorization", self.client.auth_header())
            .send()
            .context("GET /api/v1/apps/{app}/domains/{fqdn}")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("get domain failed: {status} {body}");
        }

        let body = resp.text().context("reading get-domain response body")?;
        serde_json::from_str(&body).context("decoding get-domain response")
    }

    /// Unbind a custom FQDN from an app.
    pub fn remove(&self, app: &str, fqdn: &str) -> Result<()> {
        let url = format!(
            "{}/api/v1/apps/{}/domains/{}",
            self.client.base_url(),
            app,
            fqdn
        );
        let resp = self
            .client
            .http()
            .delete(&url)
            .header("Authorization", self.client.auth_header())
            .send()
            .context("DELETE /api/v1/apps/{app}/domains/{fqdn}")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("remove domain failed: {status} {body}");
        }

        Ok(())
    }
}
