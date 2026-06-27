//! `edge cluster status` / `edge cluster events` — operator-facing
//! cluster-admin subcommands (issue #85).
//!
//! Both routes require the `owner` role. The control plane rejects
//! any other role with 403; the CLI surfaces that as a flat anyhow
//! error from `ApiClient::cluster()` so the user sees "cluster view
//! request failed: 403 ..." rather than a stack trace.
//!
//! `cluster status` prints one row per worker grouped under the
//! region name, plus the average apps-per-worker per region. Useful
//! to spot fleet skew (one worker pinned at 100 apps while peers
//! sit at 5).
//!
//! `cluster events [--region REGION] [--limit N]` prints the most
//! recent autoscale_events rows newest-first, so operators can
//! answer "why did the fleet size change?".

use anyhow::{Context, Result};
use std::path::Path;

use crate::api::ApiClient;
use crate::config::EdgeToml;

/// `edge cluster status` — fetch and pretty-print the cluster view.
///
/// One line per worker, grouped under region headers. Regions are
/// printed in BTreeMap (alphabetical) order so the output is
/// deterministic across runs — operators reading logs or scripting
/// against the output can rely on the ordering.
#[cfg(feature = "network")]
pub fn status(path: &Path) -> Result<()> {
    let edge_toml = EdgeToml::from_path(path)?;
    let client = ApiClient::new(edge_toml.api_url("https://api.edgecloud.dev"))?;
    let view = client.cluster().view()?;

    println!("Cluster snapshot (generated {})", view.generated_at);
    if view.regions.is_empty() {
        println!("(no workers registered yet)");
        return Ok(());
    }

    for (region_name, region) in &view.regions {
        println!(
            "\nregion {} — {} worker(s), avg {} apps/worker",
            region_name,
            region.workers.len(),
            region.apps_per_worker_avg,
        );
        println!(
            "{:<24} {:<14} {:<10} {:<6} {:<8} LAST_SEEN",
            "WORKER_ID", "IP", "APPS", "MEM_MB", "AGE"
        );
        println!("{}", "-".repeat(80));
        for w in &region.workers {
            let ip = w.ip.as_deref().unwrap_or("(none)");
            println!(
                "{:<24} {:<14} {:<10} {:<6} {:<8} {}",
                w.worker_id,
                ip,
                w.app_count,
                w.memory_mb,
                "—", // age is operator-easy to compute; server already sends last_seen
                w.last_seen,
            );
        }
    }
    Ok(())
}

#[cfg(not(feature = "network"))]
pub fn status(_path: &Path) -> Result<()> {
    anyhow::bail!("cluster status requires network support; rebuild with --features network")
}

/// `edge cluster events [--region REGION] [--limit N]` — fetch and
/// pretty-print the most-recent autoscale_events rows.
///
/// Empty `region` (None) lists across all regions. `limit` is
/// clamped server-side to [1, 500] with a default of 50.
#[cfg(feature = "network")]
pub fn events(path: &Path, region: Option<&str>, limit: Option<u32>) -> Result<()> {
    let edge_toml = EdgeToml::from_path(path)?;
    let client = ApiClient::new(edge_toml.api_url("https://api.edgecloud.dev"))?;
    let list = client
        .cluster()
        .events(region, limit)
        .context("fetching cluster events")?;

    let applied_region = list.region.as_deref().unwrap_or("(all)");
    println!(
        "Autoscale events ({} item(s), limit {}, region {})",
        list.items.len(),
        list.limit,
        applied_region,
    );

    if list.items.is_empty() {
        println!("(no events recorded)");
        return Ok(());
    }

    println!(
        "\n{:<19} {:<8} {:<8} {:<6} {:<6} {:<10} REASON",
        "CREATED_AT", "REGION", "ACTION", "FROM", "TO", "PROVIDER"
    );
    println!("{}", "-".repeat(90));
    for ev in &list.items {
        // Truncate the timestamp to "YYYY-MM-DDTHH:MM:SS" for
        // readability — the trailing nanoseconds and timezone offset
        // are noise in a CLI table.
        let ts = short_ts(&ev.created_at);
        let status = if ev.succeeded { "ok" } else { "FAIL" };
        let reason = match &ev.error_message {
            Some(err) => format!("{} [{}: {}]", ev.reason, status, err),
            None => ev.reason.clone(),
        };
        println!(
            "{:<19} {:<8} {:<8} {:<6} {:<6} {:<10} {}",
            ts, ev.region, ev.action, ev.from_count, ev.to_count, ev.provider_kind, reason,
        );
    }
    Ok(())
}

#[cfg(not(feature = "network"))]
pub fn events(_path: &Path, _region: Option<&str>, _limit: Option<u32>) -> Result<()> {
    anyhow::bail!("cluster events requires network support; rebuild with --features network")
}

/// short_ts trims an RFC3339 timestamp down to "YYYY-MM-DDTHH:MM:SS"
/// for CLI table output. If the input doesn't parse as RFC3339 (the
/// server's timestamp should always be), it's returned unchanged so
/// the user sees what the server actually sent rather than a
/// silently-empty field.
fn short_ts(s: &str) -> &str {
    // RFC3339 starts with "YYYY-MM-DDTHH:MM:SS" — 19 chars. Anything
    // shorter than that is malformed and we return as-is.
    if s.len() >= 19 {
        &s[..19]
    } else {
        s
    }
}
