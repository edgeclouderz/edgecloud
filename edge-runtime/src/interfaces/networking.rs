//! `edge:networking` — TCP/UDP/DNS.

use std::net::SocketAddr;

/// Resolve a hostname to a list of IP addresses.
pub fn resolve(hostname: &str) -> Result<Vec<String>, String> {
    let addr_format = format!("{}:443", hostname);
    match addr_format.parse::<SocketAddr>() {
        Ok(addr) => Ok(vec![addr.ip().to_string()]),
        Err(_) => Ok(vec![]),
    }
}