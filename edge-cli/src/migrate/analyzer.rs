//! POSIX → WASI compatibility analyzer.

use anyhow::Result;
use regex::Regex;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    pub location: String,
    pub message: String,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Severity {
    Warning,
    Compatible,
}

pub struct Analyzer {
    socket_re: Regex,
    bind_re: Regex,
    open_re: Regex,
    fopen_re: Regex,
    read_write_re: Regex,
    fprintf_stdout_re: Regex,
    printf_re: Regex,
}

impl Analyzer {
    pub fn new() -> Self {
        Self {
            socket_re: Regex::new(r"socket\s*\(").unwrap(),
            bind_re: Regex::new(r"bind\s*\(").unwrap(),
            open_re: Regex::new(r"open\s*\(").unwrap(),
            fopen_re: Regex::new(r"fopen\s*\(").unwrap(),
            read_write_re: Regex::new(r"\b(read|write)\s*\(\s*(\d+|[a-zA-Z_][a-zA-Z0-9_]*)")
                .unwrap(),
            fprintf_stdout_re: Regex::new(r"fprintf\s*\(\s*stdout\s*,").unwrap(),
            printf_re: Regex::new(r"\bprintf\s*\(").unwrap(),
        }
    }

    pub fn analyze(&self, source: &str, path: &Path) -> Result<Vec<Finding>> {
        let ext = path.extension().and_then(|s| s.to_str());
        let mut findings = Vec::new();

        match ext {
            Some("c") => findings = self.analyze_c(source, path)?,
            Some("rs") => findings = self.analyze_rust(source, path)?,
            _ => {}
        }

        Ok(findings)
    }

    fn analyze_c(&self, source: &str, path: &Path) -> Result<Vec<Finding>> {
        let mut findings = Vec::new();
        let lines: Vec<&str> = source.lines().collect();

        for (lineno, line) in lines.iter().enumerate() {
            let lineno = lineno + 1;
            let location = format!("{}:{}", path.file_name().unwrap().to_string_lossy(), lineno);

            if self.socket_re.is_match(line) {
                findings.push(Finding {
                    severity: Severity::Warning,
                    location: location.clone(),
                    message: "POSIX socket detected".to_string(),
                    suggestion: Some("Replace with WASI udp-socket or tcp-socket".to_string()),
                });
            }

            if self.bind_re.is_match(line) {
                findings.push(Finding {
                    severity: Severity::Warning,
                    location: location.clone(),
                    message: "POSIX bind detected".to_string(),
                    suggestion: Some("Replace with WASI socks".to_string()),
                });
            }

            if self.open_re.is_match(line) {
                findings.push(Finding {
                    severity: Severity::Warning,
                    location: location.clone(),
                    message: "POSIX file open detected".to_string(),
                    suggestion: Some("Replace with WASI filesystem".to_string()),
                });
            }

            if self.fopen_re.is_match(line) {
                findings.push(Finding {
                    severity: Severity::Warning,
                    location: location.clone(),
                    message: "POSIX fopen detected".to_string(),
                    suggestion: Some("Replace with WASI filesystem".to_string()),
                });
            }

            if self.read_write_re.is_match(line) {
                findings.push(Finding {
                    severity: Severity::Warning,
                    location: location.clone(),
                    message: "POSIX read/write on file descriptor".to_string(),
                    suggestion: Some("Use WASI stream I/O for sockets".to_string()),
                });
            }

            if self.fprintf_stdout_re.is_match(line) || self.printf_re.is_match(line) {
                findings.push(Finding {
                    severity: Severity::Compatible,
                    location: location.clone(),
                    message: "WASI-compatible stdio output".to_string(),
                    suggestion: None,
                });
            }
        }

        Ok(findings)
    }

    fn analyze_rust(&self, source: &str, path: &Path) -> Result<Vec<Finding>> {
        let mut findings = Vec::new();
        let lines: Vec<&str> = source.lines().collect();

        let std_net_re = Regex::new(r"std::net").unwrap();
        let tcp_stream_re = Regex::new(r"TcpStream|UdpSocket").unwrap();

        for (lineno, line) in lines.iter().enumerate() {
            let lineno = lineno + 1;
            let location = format!("{}:{}", path.file_name().unwrap().to_string_lossy(), lineno);

            if std_net_re.is_match(line) {
                findings.push(Finding {
                    severity: Severity::Warning,
                    location: location.clone(),
                    message: "POSIX networking via std::net detected".to_string(),
                    suggestion: Some("Replace with WASI sockets interface".to_string()),
                });
            }

            if tcp_stream_re.is_match(line) {
                findings.push(Finding {
                    severity: Severity::Warning,
                    location: location.clone(),
                    message: "TCP/UDP socket type detected".to_string(),
                    suggestion: Some("Replace with WASI tcp-socket or udp-socket".to_string()),
                });
            }
        }

        Ok(findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_analyze_c_socket() {
        let a = Analyzer::new();
        let findings = a
            .analyze_c("socket(AF_INET, ...);", Path::new("test.c"))
            .unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warning);
        assert!(findings[0].message.contains("socket"));
    }

    #[test]
    fn test_analyze_c_bind() {
        let a = Analyzer::new();
        let findings = a
            .analyze_c("bind(sock, ...);", Path::new("test.c"))
            .unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("bind"));
    }

    #[test]
    fn test_analyze_c_open() {
        let a = Analyzer::new();
        let findings = a
            .analyze_c("int fd = open(\"/tmp/f\", 0);", Path::new("test.c"))
            .unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("file open"));
    }

    #[test]
    fn test_analyze_c_fopen() {
        let a = Analyzer::new();
        let findings = a
            .analyze_c("fopen(\"log.txt\", \"w\");", Path::new("test.c"))
            .unwrap();
        // fopen matches both open_re and fopen_re
        assert!(findings.iter().any(|f| f.message.contains("fopen")));
    }

    #[test]
    fn test_analyze_c_read_write() {
        let a = Analyzer::new();
        let findings = a
            .analyze_c("read(5, buf, 128);", Path::new("test.c"))
            .unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("read/write"));
    }

    #[test]
    fn test_analyze_c_printf() {
        let a = Analyzer::new();
        let findings = a
            .analyze_c("printf(\"hello\\n\");", Path::new("test.c"))
            .unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Compatible);
        assert!(findings[0].message.contains("stdio"));
    }

    #[test]
    fn test_analyze_c_fprintf_stdout() {
        let a = Analyzer::new();
        let findings = a
            .analyze_c("fprintf(stdout, \"x=%d\\n\", x);", Path::new("test.c"))
            .unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Compatible);
    }

    #[test]
    fn test_analyze_c_multiple_on_one_line() {
        let a = Analyzer::new();
        let findings = a
            .analyze_c("socket(AF_INET); bind(s, ...);", Path::new("test.c"))
            .unwrap();
        assert!(findings.len() >= 2);
    }

    #[test]
    fn test_analyze_c_no_patterns() {
        let a = Analyzer::new();
        let findings = a.analyze_c("int x = 42;", Path::new("test.c")).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn test_analyze_c_line_numbers() {
        let a = Analyzer::new();
        let src = "// line 1\nsocket(AF_INET); // line 2\n// line 3";
        let findings = a.analyze_c(src, Path::new("test.c")).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0].location.contains("test.c:2"),
            "expected line 2, got {}",
            findings[0].location
        );
    }

    #[test]
    fn test_analyze_rust_std_net() {
        let a = Analyzer::new();
        let findings = a
            .analyze_rust("use std::net::TcpStream;", Path::new("main.rs"))
            .unwrap();
        // "use std::net::TcpStream" matches both std::net AND TcpStream regexes
        assert!(findings.len() >= 1);
        assert!(findings.iter().any(|f| f.message.contains("std::net")));
    }

    #[test]
    fn test_analyze_rust_tcp_stream() {
        let a = Analyzer::new();
        let findings = a
            .analyze_rust("let s = TcpStream::connect(...);", Path::new("main.rs"))
            .unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("TCP/UDP socket"));
    }

    #[test]
    fn test_analyze_rust_udp_socket() {
        let a = Analyzer::new();
        let findings = a
            .analyze_rust("let s = UdpSocket::bind(...);", Path::new("main.rs"))
            .unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("UDP"));
    }

    #[test]
    fn test_analyze_rust_no_patterns() {
        let a = Analyzer::new();
        let findings = a
            .analyze_rust("fn main() { println!(\"hi\"); }", Path::new("main.rs"))
            .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn test_analyze_rust_line_numbers() {
        let a = Analyzer::new();
        let src = "// line 1\nTcpStream::connect(...); // line 2";
        let findings = a.analyze_rust(src, Path::new("main.rs")).unwrap();
        assert!(findings[0].location.contains("main.rs:2"));
    }

    #[test]
    fn test_analyze_dispatches_on_c_extension() {
        let a = Analyzer::new();
        let findings = a.analyze("socket(AF_INET);", Path::new("test.c")).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("socket"));
    }

    #[test]
    fn test_analyze_dispatches_on_rs_extension() {
        let a = Analyzer::new();
        let findings = a
            .analyze("TcpStream::connect();", Path::new("main.rs"))
            .unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("TCP/UDP"));
    }

    #[test]
    fn test_analyze_unknown_extension_returns_empty() {
        let a = Analyzer::new();
        let findings = a
            .analyze("socket(...); printf(...);", Path::new("script.py"))
            .unwrap();
        assert!(findings.is_empty());
    }
}
