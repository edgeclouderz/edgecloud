//! `edge:process` — environment variables, command-line args, and exit.

use std::env;

/// Get a single environment variable by name.
pub fn get_env_by_name(key: &str) -> Result<Option<String>, String> {
    Ok(env::var(key).ok())
}

/// Get all environment variables as a list of (key, value) tuples.
pub fn get_all_env() -> Result<Vec<(String, String)>, String> {
    Ok(env::vars().collect())
}

/// Get the command-line arguments.
pub fn get_args() -> Result<Vec<String>, String> {
    Ok(env::args().collect())
}

/// Exit the process with the given exit code.
pub fn exit(code: u32) -> ! {
    std::process::exit(code as i32)
}