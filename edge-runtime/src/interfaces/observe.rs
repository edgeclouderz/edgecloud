//! `edge:observe` — metrics and logging.

/// Emit a log message at the given level.
pub fn emit_log(level: &str, message: &str) -> Result<(), String> {
    match level {
        "error" => tracing::error!(message),
        "warn" => tracing::warn!(message),
        "info" => tracing::info!(message),
        "debug" => tracing::debug!(message),
        _ => tracing::trace!(message),
    }
    Ok(())
}