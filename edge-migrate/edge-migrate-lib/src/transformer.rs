//! POSIX → WASI transformation.
//!
//! Transforms detected POSIX patterns to WASI equivalents,
//! generating transformed source code and a transformation report.

use crate::patterns::{PatternMatch, PosixPattern, Transformability};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;

/// A single transformation applied to the source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transformation {
    /// 1-based line number where the transformation was applied.
    pub line: usize,
    /// The pattern that was transformed.
    pub pattern: PosixPattern,
    /// A description of what was changed.
    pub description: String,
}

/// An error that occurred during transformation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformError {
    /// 1-based line number where the error occurred.
    pub line: usize,
    /// The pattern that failed.
    pub pattern: PosixPattern,
    /// Human-readable error message.
    pub message: String,
}

/// The result of a transformation operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformResult {
    /// The WASI-compatible C source code.
    pub transformed_source: String,
    /// All transformations that were applied.
    pub transformations_applied: Vec<Transformation>,
    /// Patterns that require manual review.
    pub manual_review: Vec<PatternMatch>,
    /// Errors that occurred during transformation.
    pub errors: Vec<TransformError>,
}

/// WASI header includes to prepend to the transformed source.
const WASI_INCLUDES: &str = r#"#include <wasi/sockets.h>
#include <wasi/io/streams.h>
#include <wasi/filesystem.h>
#include <wasi/ip-name-lookup.h>

"#;

/// Transforms POSIX C source to WASI-compatible C source.
pub struct Transformer;

impl Transformer {
    /// Transform the given C source based on detected pattern matches.
    pub fn transform(source: &str, matches: Vec<PatternMatch>) -> TransformResult {
        let mut transformations_applied = Vec::new();
        let mut manual_review = Vec::new();
        let errors = Vec::new();

        // Partition into transformable and not-transformable
        let (transformable, not_transformable): (Vec<_>, Vec<_>) = matches
            .into_iter()
            .partition(|m| m.transformability != Transformability::NotTransformable);

        manual_review.extend(not_transformable);

        // Sort by start_byte descending — process from end of file backward
        // so earlier byte positions remain valid after replacements
        let mut sorted = transformable;
        sorted.sort_by_key(|m| Reverse(m.start_byte));

        let mut source_bytes = source.as_bytes().to_vec();
        let mut added_wasi_header = false;
        let header_len = WASI_INCLUDES.len();
        // Cumulative offset from all splices performed so far.
        // A positive value means the source has grown; all subsequent
        // positions must be adjusted by this amount.
        let mut running_offset = 0_isize;

        for m in &sorted {
            let wasi_code = Self::generate_wasi_code(m);
            if !wasi_code.is_empty() {
                // Mark that we need the WASI header
                if !added_wasi_header && m.start_byte > 0 {
                    // Prepend header at the top of the file
                    source_bytes = Self::prepend_wasi_header(&source_bytes);
                    added_wasi_header = true;
                    running_offset += header_len as isize;
                }

                // Apply running offset to get current position in source_bytes
                let current_start = (m.start_byte as isize + running_offset) as usize;
                let current_end = (m.end_byte as isize + running_offset) as usize;

                // Replace the matched bytes with WASI code
                let new_len = wasi_code.len();
                let old_len = current_end - current_start;
                source_bytes.splice(current_start..current_end, wasi_code.as_bytes().to_vec());

                // Update running offset: subsequent positions shift by (new_len - old_len)
                running_offset += (new_len as isize) - (old_len as isize);

                transformations_applied.push(Transformation {
                    line: m.line,
                    pattern: m.pattern.clone(),
                    description: format!(
                        "Transformed {} → {}",
                        m.snippet.split('(').next().unwrap_or(&m.snippet),
                        m.pattern.wasi_equivalent()
                    ),
                });
            }
        }

        let transformed_source = String::from_utf8(source_bytes)
            .expect("Transformed source is not valid UTF-8");

        TransformResult {
            transformed_source,
            transformations_applied,
            manual_review,
            errors,
        }
    }

    /// Prepend WASI header includes to the source.
    fn prepend_wasi_header(source: &[u8]) -> Vec<u8> {
        let mut result = WASI_INCLUDES.as_bytes().to_vec();
        result.extend_from_slice(source);
        result
    }

    /// Generate WASI C code for a pattern match.
    fn generate_wasi_code(m: &PatternMatch) -> String {
        match m.pattern {
            PosixPattern::SocketTcp => {
                "wasi_socket_tcp_create(IP_ADDRESS_FAMILY_IPV4)".to_string()
            }
            PosixPattern::SocketUdp => {
                "wasi_socket_udp_create(IP_ADDRESS_FAMILY_IPV4)".to_string()
            }
            PosixPattern::Bind => {
                // Two-phase: start-bind + finish-bind
                // The original call is like: bind(fd, &addr, len)
                // We replace with a two-line block
                format!(
                    "// WASI: two-phase bind\n{{\n  wasi_socket_tcp_start_bind({}, {});\n  wasi_socket_tcp_finish_bind({});\n}}",
                    Self::extract_first_arg(&m.snippet),
                    Self::extract_first_arg(&m.snippet),
                    Self::extract_first_arg(&m.snippet)
                )
            }
            PosixPattern::Listen => {
                // Two-phase: start-listen + finish-listen
                format!(
                    "// WASI: two-phase listen\n{{\n  wasi_socket_tcp_start_listen({}, {});\n  wasi_socket_tcp_finish_listen({});\n}}",
                    Self::extract_first_arg(&m.snippet),
                    Self::extract_second_arg(&m.snippet),
                    Self::extract_first_arg(&m.snippet)
                )
            }
            PosixPattern::Connect => {
                // Two-phase: start-connect + finish-connect
                format!(
                    "// WASI: two-phase connect\n{{\n  wasi_socket_tcp_start_connect({}, {});\n  wasi_socket_tcp_finish_connect({});\n}}",
                    Self::extract_first_arg(&m.snippet),
                    Self::extract_second_arg(&m.snippet),
                    Self::extract_first_arg(&m.snippet)
                )
            }
            PosixPattern::Accept => {
                // Wrap in poll loop
                format!(
                    "// WASI: accept with poll loop\n{{\n  wasi_socket_tcp_accept_result_t result;\n  do {{\n    result = wasi_socket_tcp_accept({});\n    if (result.tag == WASI_SOCKET_TCP_ACCEPT_ERROR_WOULD_BLOCK) {{\n      wasi_poll_pollable_block(pollable);\n    }}\n  }} while (result.tag == WASI_SOCKET_TCP_ACCEPT_ERROR_WOULD_BLOCK);\n  /* accepted socket in result.val */\n}}",
                    Self::extract_first_arg(&m.snippet)
                )
            }
            PosixPattern::Recv => {
                format!(
                    "wasi_input_stream_read({}, {}, {})",
                    Self::extract_first_arg(&m.snippet),
                    Self::extract_second_arg(&m.snippet),
                    Self::extract_third_arg(&m.snippet)
                )
            }
            PosixPattern::Send => {
                format!(
                    "wasi_output_stream_write({}, {}, {})",
                    Self::extract_first_arg(&m.snippet),
                    Self::extract_second_arg(&m.snippet),
                    Self::extract_third_arg(&m.snippet)
                )
            }
            PosixPattern::GetHostByName => {
                format!(
                    "wasi_ip_name_lookup_resolve({})",
                    Self::extract_first_arg(&m.snippet)
                )
            }
            PosixPattern::Close => {
                format!("wasi_socket_close({})", Self::extract_first_arg(&m.snippet))
            }
            PosixPattern::Fopen => {
                format!(
                    "wasi_filesystem_open({}, {})",
                    Self::extract_first_arg(&m.snippet),
                    Self::extract_second_arg(&m.snippet)
                )
            }
            PosixPattern::Fread => {
                format!(
                    "wasi_filesystem_read({}, {}, {})",
                    Self::extract_first_arg(&m.snippet),
                    Self::extract_second_arg(&m.snippet),
                    Self::extract_third_arg(&m.snippet)
                )
            }
            PosixPattern::Fwrite => {
                format!(
                    "wasi_filesystem_write({}, {}, {})",
                    Self::extract_first_arg(&m.snippet),
                    Self::extract_second_arg(&m.snippet),
                    Self::extract_third_arg(&m.snippet)
                )
            }
            PosixPattern::Fclose => {
                format!(
                    "wasi_filesystem_close({})",
                    Self::extract_first_arg(&m.snippet)
                )
            }
            // These should not reach here (NotTransformable patterns)
            _ => String::new(),
        }
    }

    /// Extract the first argument from a call snippet.
    fn extract_first_arg(snippet: &str) -> String {
        Self::extract_arg(snippet, 0)
    }

    /// Extract the second argument from a call snippet.
    fn extract_second_arg(snippet: &str) -> String {
        Self::extract_arg(snippet, 1)
    }

    /// Extract the third argument from a call snippet.
    fn extract_third_arg(snippet: &str) -> String {
        Self::extract_arg(snippet, 2)
    }

    /// Extract the nth argument from a call snippet like "func(arg1, arg2, arg3)".
    fn extract_arg(snippet: &str, n: usize) -> String {
        // Find the opening parenthesis
        if let Some(paren_idx) = snippet.find('(') {
            let args_part = &snippet[paren_idx + 1..];
            // Count commas to find the right argument
            let mut depth = 0;
            let mut arg_start = 0;
            let mut commas_found = 0;

            for (i, c) in args_part.char_indices() {
                if c == '(' {
                    depth += 1;
                } else if c == ')' {
                    if depth == 0 {
                        // End of arguments
                        let arg = args_part[arg_start..i].trim();
                        return arg.to_string();
                    }
                    depth -= 1;
                } else if c == ',' && depth == 0 {
                    if commas_found == n {
                        let arg = args_part[arg_start..i].trim();
                        return arg.to_string();
                    }
                    commas_found += 1;
                    arg_start = i + 1;
                }
            }
        }
        "/* unknown */".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transform_tcp_socket() {
        let source = r#"
int main() {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    return 0;
}
"#;
        let matches = vec![PatternMatch {
            line: 3,
            start_byte: 16,
            end_byte: 48,
            pattern: PosixPattern::SocketTcp,
            snippet: "socket(AF_INET, SOCK_STREAM, 0)".to_string(),
            transformability: Transformability::AutoTransformable,
        }];
        let result = Transformer::transform(source, matches);
        assert_eq!(result.transformations_applied.len(), 1);
        assert_eq!(result.manual_review.len(), 0);
        assert!(result.transformed_source.contains("wasi_socket_tcp_create"));
    }

    #[test]
    fn test_transform_poll_not_transformable() {
        let source = r#"
int main() {
    poll(fds, 2, timeout);
    return 0;
}
"#;
        let matches = vec![PatternMatch {
            line: 3,
            start_byte: 0,
            end_byte: 0,
            pattern: PosixPattern::Poll,
            snippet: "poll(fds, 2, timeout)".to_string(),
            transformability: Transformability::NotTransformable,
        }];
        let result = Transformer::transform(source, matches);
        assert_eq!(result.transformations_applied.len(), 0);
        assert_eq!(result.manual_review.len(), 1);
    }

    #[test]
    fn test_transform_bind_two_phase() {
        let source = r#"
int main() {
    bind(fd, (struct sockaddr*)&addr, sizeof(addr));
    return 0;
}
"#;
        let matches = vec![PatternMatch {
            line: 3,
            start_byte: 16,
            end_byte: 65,
            pattern: PosixPattern::Bind,
            snippet: "bind(fd, (struct sockaddr*)&addr, sizeof(addr))".to_string(),
            transformability: Transformability::AutoTransformable,
        }];
        let result = Transformer::transform(source, matches);
        assert_eq!(result.transformations_applied.len(), 1);
        assert!(result.transformed_source.contains("wasi_socket_tcp_start_bind"));
        assert!(result.transformed_source.contains("wasi_socket_tcp_finish_bind"));
    }

    #[test]
    fn test_transform_connect_two_phase() {
        let source = r#"
int main() {
    connect(fd, (struct sockaddr*)&addr, sizeof(addr));
    return 0;
}
"#;
        let matches = vec![PatternMatch {
            line: 3,
            start_byte: 16,
            end_byte: 68,
            pattern: PosixPattern::Connect,
            snippet: "connect(fd, (struct sockaddr*)&addr, sizeof(addr))".to_string(),
            transformability: Transformability::AutoTransformable,
        }];
        let result = Transformer::transform(source, matches);
        assert_eq!(result.transformations_applied.len(), 1);
        assert!(result.transformed_source.contains("wasi_socket_tcp_start_connect"));
        assert!(result.transformed_source.contains("wasi_socket_tcp_finish_connect"));
    }

    #[test]
    fn test_transform_recv() {
        let source = r#"
int main() {
    recv(fd, buf, len, 0);
    return 0;
}
"#;
        let matches = vec![PatternMatch {
            line: 3,
            start_byte: 16,
            end_byte: 35,
            pattern: PosixPattern::Recv,
            snippet: "recv(fd, buf, len, 0)".to_string(),
            transformability: Transformability::AutoTransformable,
        }];
        let result = Transformer::transform(source, matches);
        assert_eq!(result.transformations_applied.len(), 1);
        assert!(result.transformed_source.contains("wasi_input_stream_read"));
    }

    #[test]
    fn test_transform_send() {
        let source = r#"
int main() {
    send(fd, buf, len, 0);
    return 0;
}
"#;
        let matches = vec![PatternMatch {
            line: 3,
            start_byte: 16,
            end_byte: 35,
            pattern: PosixPattern::Send,
            snippet: "send(fd, buf, len, 0)".to_string(),
            transformability: Transformability::AutoTransformable,
        }];
        let result = Transformer::transform(source, matches);
        assert_eq!(result.transformations_applied.len(), 1);
        assert!(result.transformed_source.contains("wasi_output_stream_write"));
    }

    #[test]
    fn test_transform_accept_poll_loop() {
        let source = r#"
int main() {
    accept(fd, NULL, NULL);
    return 0;
}
"#;
        let matches = vec![PatternMatch {
            line: 3,
            start_byte: 16,
            end_byte: 38,
            pattern: PosixPattern::Accept,
            snippet: "accept(fd, NULL, NULL)".to_string(),
            transformability: Transformability::BestEffort,
        }];
        let result = Transformer::transform(source, matches);
        assert_eq!(result.transformations_applied.len(), 1);
        assert!(result.transformed_source.contains("wasi_socket_tcp_accept"));
        assert!(result.transformed_source.contains("wasi_poll_pollable_block"));
    }

    #[test]
    fn test_extract_first_arg() {
        assert_eq!(Transformer::extract_first_arg("socket(AF_INET, SOCK_STREAM, 0)"), "AF_INET");
        assert_eq!(Transformer::extract_first_arg("bind(fd, &addr, len)"), "fd");
        assert_eq!(Transformer::extract_first_arg("recv(fd, buf, len, 0)"), "fd");
    }

    #[test]
    fn test_extract_second_arg() {
        assert_eq!(Transformer::extract_second_arg("bind(fd, &addr, len)"), "&addr");
        assert_eq!(Transformer::extract_second_arg("listen(fd, 128)"), "128");
    }
}
