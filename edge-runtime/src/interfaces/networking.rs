//! `edge:networking` — DNS resolution.

#[cfg(feature = "networking")]
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};
#[cfg(feature = "networking")]
use trust_dns_resolver::TokioAsyncResolver;

#[cfg(feature = "networking")]
pub struct NetworkingState {
    resolver: TokioAsyncResolver,
    runtime_handle: tokio::runtime::Handle,
}

#[cfg(feature = "networking")]
impl NetworkingState {
    pub fn new() -> Self {
        Self::new_with_handle(tokio::runtime::Handle::current())
    }

    pub fn new_with_handle(handle: tokio::runtime::Handle) -> Self {
        let resolver =
            TokioAsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default());
        Self {
            resolver,
            runtime_handle: handle,
        }
    }

    /// Resolve a hostname to a list of IP addresses.
    pub fn resolve(&self, hostname: &str) -> Result<Vec<String>, String> {
        self.runtime_handle
            .block_on(self.resolve_async(hostname))
    }

    async fn resolve_async(&self, hostname: &str) -> Result<Vec<String>, String> {
        self.resolver
            .lookup_ip(hostname)
            .await
            .map(|lookup| lookup.iter().map(|ip| ip.to_string()).collect())
            .map_err(|e| format!("DNS resolution failed: {}", e))
    }
}

impl Default for NetworkingState {
    fn default() -> Self {
        Self::new()
    }
}
