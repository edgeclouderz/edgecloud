//! HTTP client for the edgeCloud control plane API.

use anyhow::Result;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::config::ApiKey;

/// HTTP client for all control plane API calls.
pub struct ApiClient {
    http: Client,
    base_url: String,
    api_key: ApiKey,
}

#[derive(Debug, Deserialize)]
pub struct DeployResponse {
    pub id: String,
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct StatusResponse {
    pub id: String,
    pub status: String,
    pub created_at: String,
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct DeploymentSummary {
    pub id: String,
    pub status: String,
    pub created_at: String,
    pub url: Option<String>,
}

impl ApiClient {
    /// Create a new API client.
    pub fn new(base_url: String) -> Result<Self> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| anyhow::anyhow!("reqwest client failed: {}", e))?;
        let api_key = ApiKey::load()?;
        Ok(Self { http, base_url, api_key })
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.api_key.0)
    }

    /// Upload a deployment artifact.
    pub fn deploy(&self, app_name: &str, wasm_bytes: &[u8]) -> Result<DeployResponse> {
        use reqwest::blocking::multipart;

        let url = format!("{}/api/deploy/{}", self.base_url, app_name);
        let part = multipart::Part::bytes(wasm_bytes.to_vec())
            .file_name("payload");
        let form = multipart::Form::new().part("payload", part);

        let resp = self.http
            .post(&url)
            .header("Authorization", self.auth_header())
            .multipart(form)
            .send()?;

        if !resp.status().is_success() {
            anyhow::bail!("deploy failed: {} {}", resp.status(), resp.text()?);
        }

        let body: DeployResponse = serde_json::from_str(&resp.text()?)?;
        Ok(body)
    }

    /// Get deployment status.
    pub fn status(&self, deployment_id: &str) -> Result<StatusResponse> {
        let url = format!("{}/api/status/{}", self.base_url, deployment_id);
        let resp = self.http
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()?;

        if !resp.status().is_success() {
            anyhow::bail!("status failed: {} {}", resp.status(), resp.text()?);
        }

        serde_json::from_str(&resp.text()?).map_err(Into::into)
    }

    /// List environment variables for an app.
    pub fn list_env(&self, app_name: &str) -> Result<Vec<EnvVar>> {
        let url = format!("{}/api/apps/{}/env", self.base_url, app_name);
        let resp = self.http
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()?;

        if !resp.status().is_success() {
            anyhow::bail!("list env failed: {} {}", resp.status(), resp.text()?);
        }

        serde_json::from_str(&resp.text()?).map_err(Into::into)
    }

    /// Set an environment variable.
    pub fn set_env(&self, app_name: &str, key: &str, value: &str) -> Result<()> {
        let url = format!("{}/api/apps/{}/env", self.base_url, app_name);
        #[derive(Serialize)]
        struct Payload<'a> { key: &'a str, value: &'a str }
        let payload = Payload { key, value };
        let resp = self.http
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&payload)
            .send()?;

        if !resp.status().is_success() {
            anyhow::bail!("set env failed: {} {}", resp.status(), resp.text()?);
        }

        Ok(())
    }

    /// Activate a deployment.
    pub fn activate(&self, app_name: &str, deployment_id: &str) -> Result<()> {
        let url = format!(
            "{}/api/apps/{}/activate/{}",
            self.base_url, app_name, deployment_id
        );
        let resp = self.http
            .post(&url)
            .header("Authorization", self.auth_header())
            .send()?;

        if !resp.status().is_success() {
            anyhow::bail!("activate failed: {} {}", resp.status(), resp.text()?);
        }

        Ok(())
    }

    /// List all deployments for an app.
    pub fn list_deployments(&self, app_name: &str) -> Result<Vec<DeploymentSummary>> {
        let url = format!("{}/api/list/{}", self.base_url, app_name);
        let resp = self.http
            .get(&url)
            .header("Authorization", self.auth_header())
            .send()?;

        if !resp.status().is_success() {
            anyhow::bail!("list deployments failed: {} {}", resp.status(), resp.text()?);
        }

        serde_json::from_str(&resp.text()?).map_err(Into::into)
    }
}
