//! Styled terminal output.

use console::style;

/// Print a success message in green.
pub fn success(msg: &str) {
    println!("{}", style(msg).green());
}

/// Print an error message in red.
pub fn error(msg: &str) {
    eprintln!("{}", style(msg).red());
}

/// Print a warning message in yellow.
pub fn warn(msg: &str) {
    eprintln!("{}", style(msg).yellow());
}

/// Print an info message in cyan.
pub fn info(msg: &str) {
    println!("{}", style(msg).cyan());
}

/// Print a section header.
pub fn section(label: &str) {
    println!("\n{} {}", style("›").cyan(), style(label).bold());
}
