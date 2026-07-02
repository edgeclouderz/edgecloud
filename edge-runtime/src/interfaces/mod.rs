//! Host function implementations for edge:* WIT interfaces.

/// Returns `true` iff `id` is safe to use as a single directory component.
/// Rejects empty strings, path separators, `.`, `..`, null bytes, colons,
/// and Windows reserved device names (CON, NUL, PRN, AUX, COM1-9, LPT1-9).
pub fn is_safe_tenant_id(id: &str) -> bool {
    if id.is_empty() || id == "." || id == ".." {
        return false;
    }
    if id.contains('/') || id.contains('\\') || id.contains('\0') || id.contains(':') {
        return false;
    }
    let upper = id.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        return false;
    }
    true
}

// The http_client / http_server / networking / dns modules were dropped in
// v0.2 — components needing HTTP go through `wasi:http`, sockets through
// `wasi:sockets`, and DNS through `wasi:sockets/ip-name-lookup`.
// The async, host-provided `edge:cloud/*` interfaces retained here.
pub mod cache;
pub mod kv_store;
pub mod observe;
pub mod process;
pub mod scheduling;
pub mod time;
