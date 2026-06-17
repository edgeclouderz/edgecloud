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
