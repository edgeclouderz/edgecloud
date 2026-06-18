//! POSIX pattern definitions and WASI equivalents.
//!
//! Defines all POSIX patterns that edge-migrate can detect, their
//! transformability classification, and WASI equivalents.

use serde::{Deserialize, Serialize};

/// Classification of how transformable a POSIX pattern is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transformability {
    /// Can be automatically transformed to WASI with no manual intervention.
    AutoTransformable,
    /// Transformable but may require developer review (e.g., poll loops).
    BestEffort,
    /// Cannot be auto-transformed — requires manual rewrite.
    NotTransformable,
}

/// A detected POSIX pattern in source code.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PatternMatch {
    /// 1-based line number where the pattern was detected.
    ///
    /// When the analyzer runs with a preprocessor attached and macro
    /// expansion is successful, this is the **original** source line
    /// (1-based), remapped from the expanded source via the
    /// preprocessor's `line_map`. When the preprocessor is not
    /// attached, fails, or yields no useful mapping, the value is
    /// the 1-based line in the (possibly expanded) source actually
    /// fed to tree-sitter.
    pub line: usize,
    /// 0-based column (byte offset within the line) where the pattern
    /// starts. Currently always `None`; populated in M2 by the
    /// multi-file tree walker so reports include both line and column
    /// for editor integration. `#[serde(default)]` keeps older reports
    /// deserializable.
    #[serde(default)]
    pub column: Option<usize>,
    /// Start byte offset in the source (for replacement).
    pub start_byte: usize,
    /// End byte offset in the source (for replacement).
    pub end_byte: usize,
    /// The kind of POSIX pattern detected.
    pub pattern: PosixPattern,
    /// The original source code snippet.
    pub snippet: String,
    /// Raw text of each argument node from the AST (for accurate arg extraction).
    pub arg_nodes: Vec<String>,
    /// Whether this pattern can be auto-transformed.
    pub transformability: Transformability,
}

impl Default for PatternMatch {
    /// Used by struct literals that don't yet populate every field
    /// (e.g. `..Default::default()`). `line` defaults to 0 — callers
    /// that go through `analyze()` always get a real value.
    fn default() -> Self {
        Self {
            line: 0,
            column: None,
            start_byte: 0,
            end_byte: 0,
            pattern: PosixPattern::Unknown,
            snippet: String::new(),
            arg_nodes: Vec::new(),
            transformability: Transformability::NotTransformable,
        }
    }
}

/// All known POSIX patterns that edge-migrate can detect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PosixPattern {
    /// `socket(AF_INET, SOCK_STREAM, 0)` — TCP socket creation.
    SocketTcp,
    /// `socket(AF_INET, SOCK_DGRAM, 0)` — UDP socket creation.
    SocketUdp,
    /// `bind()` — bind socket to address.
    Bind,
    /// `listen()` — mark socket as listening.
    Listen,
    /// `accept()` — accept incoming connection.
    Accept,
    /// `connect()` — connect to remote address.
    Connect,
    /// `recv()` / `read()` on a socket.
    Recv,
    /// `send()` / `write()` on a socket.
    Send,
    /// `gethostbyname()` or `getaddrinfo()` — DNS resolution.
    GetHostByName,
    /// `close()` on a socket file descriptor.
    Close,
    /// `fopen()` — open a file.
    Fopen,
    /// `fread()` — read from file.
    Fread,
    /// `fwrite()` — write to file.
    Fwrite,
    /// `fclose()` — close a file.
    Fclose,
    /// `poll()` — event polling (not transformable).
    Poll,
    /// `select()` — file descriptor set polling (not transformable).
    Select,
    /// `fork()` — process forking (not transformable).
    Fork,
    /// `exec()` / `execve()` — process execution (not transformable).
    Exec,
    /// `socketpair()` — creates connected socket pair (not transformable).
    SocketPair,
    /// `shutdown()` — full-duplex shutdown (not in wasi-sockets).
    Shutdown,
    /// `O_NONBLOCK` flag usage (not applicable in WASI).
    NonBlocking,
    /// `SOCK_RAW` — raw socket (not supported in WASI).
    SockRaw,
    /// Unknown or user-defined pattern (treated as not transformable).
    Unknown,
}

impl PosixPattern {
    /// Returns the WASI equivalent description for this pattern.
    pub fn wasi_equivalent(&self) -> &'static str {
        match self {
            PosixPattern::SocketTcp => "create-tcp-socket(ipv4)",
            PosixPattern::SocketUdp => "create-udp-socket(ipv4)",
            PosixPattern::Bind => "start-bind() + finish-bind()",
            PosixPattern::Listen => "start-listen() + finish-listen()",
            PosixPattern::Accept => "accept() with poll loop",
            PosixPattern::Connect => "start-connect() + finish-connect()",
            PosixPattern::Recv => "input-stream read via wasi:io/streams",
            PosixPattern::Send => "output-stream write via wasi:io/streams",
            PosixPattern::GetHostByName => "wasi:ip-name-lookup",
            PosixPattern::Close => "drop() on socket resource",
            PosixPattern::Fopen => "wasi:filesystem open",
            PosixPattern::Fread => "wasi:filesystem read",
            PosixPattern::Fwrite => "wasi:filesystem write",
            PosixPattern::Fclose => "wasi:filesystem close",
            PosixPattern::Poll => "no WASI equivalent — restructure event loop",
            PosixPattern::Select => "no WASI equivalent — restructure event loop",
            PosixPattern::Fork => "no WASI equivalent — Wasm has no process model",
            PosixPattern::Exec => "no WASI equivalent — Wasm has no process model",
            PosixPattern::SocketPair => "no WASI equivalent",
            PosixPattern::Shutdown => "not in wasi-sockets",
            PosixPattern::NonBlocking => "WASI sockets are always non-blocking",
            PosixPattern::SockRaw => "raw sockets not supported in WASI",
            PosixPattern::Unknown => "unknown pattern",
        }
    }

    /// Returns the transformability classification for this pattern.
    pub fn transformability(&self) -> Transformability {
        match self {
            PosixPattern::SocketTcp
            | PosixPattern::SocketUdp
            | PosixPattern::Bind
            | PosixPattern::Listen
            | PosixPattern::Connect
            | PosixPattern::Recv
            | PosixPattern::Send
            | PosixPattern::GetHostByName
            | PosixPattern::Close
            | PosixPattern::Fopen
            | PosixPattern::Fread
            | PosixPattern::Fwrite
            | PosixPattern::Fclose => Transformability::AutoTransformable,
            PosixPattern::Accept => Transformability::BestEffort,
            PosixPattern::Poll
            | PosixPattern::Select
            | PosixPattern::Fork
            | PosixPattern::Exec
            | PosixPattern::SocketPair
            | PosixPattern::Shutdown
            | PosixPattern::NonBlocking
            | PosixPattern::SockRaw
            | PosixPattern::Unknown => Transformability::NotTransformable,
        }
    }
}

/// Validate a deployment app name against the public-facing format
/// `^[a-z0-9][a-z0-9-]{0,62}$`.
///
/// Distinct from path-safety checks (no `..`, no `/`). Used by the
/// `edge-migrate --tree` CLI and the Go control plane's
/// `IsValidDeploymentAppName` mirror. Keeping the regex in one place
/// (the shared design doc) — both sides are tested against the same
/// set of valid / invalid examples.
pub fn is_valid_deployment_app_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > 63 {
        return false;
    }
    // First char: lowercase letter or digit.
    let first = bytes[0];
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    // Remaining chars: lowercase letter, digit, or '-'.
    for &b in &bytes[1..] {
        if !b.is_ascii_lowercase() && !b.is_ascii_digit() && b != b'-' {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_deployment_app_name_accepts_valid() {
        assert!(is_valid_deployment_app_name("a"));
        assert!(is_valid_deployment_app_name("hello-world"));
        assert!(is_valid_deployment_app_name("my-app-123"));
        assert!(is_valid_deployment_app_name("0"));
        assert!(is_valid_deployment_app_name("a".repeat(63).as_str()));
    }

    #[test]
    fn test_is_valid_deployment_app_name_rejects_invalid() {
        // Empty
        assert!(!is_valid_deployment_app_name(""));
        // Too long (64 chars)
        assert!(!is_valid_deployment_app_name(&"a".repeat(64)));
        // Uppercase
        assert!(!is_valid_deployment_app_name("Hello"));
        assert!(!is_valid_deployment_app_name("HELLO"));
        // Starts with non-alnum
        assert!(!is_valid_deployment_app_name("-hello"));
        assert!(!is_valid_deployment_app_name("_hello"));
        // Contains invalid chars
        assert!(!is_valid_deployment_app_name("hello_world"));
        assert!(!is_valid_deployment_app_name("hello world"));
        assert!(!is_valid_deployment_app_name("hello.world"));
        assert!(!is_valid_deployment_app_name("hello/world"));
        // Path traversal
        assert!(!is_valid_deployment_app_name("../traversal"));
        assert!(!is_valid_deployment_app_name("a/../b"));
    }
}
