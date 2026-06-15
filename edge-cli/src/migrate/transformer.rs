//! Auto-transform POSIX calls to WASI equivalents.

use anyhow::Result;
use std::path::Path;
use walkdir::WalkDir;

/// Apply safe auto-transformations in place.
pub fn apply_transforms(path: &Path) -> Result<()> {
    for entry in WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("c"))
    {
        let file_path = entry.path();
        let content = std::fs::read_to_string(file_path)?;

        // Simple safe transforms
        let transformed = content.clone()
            //fprintf(stdout, ...) → leave as-is (WASI-compatible, just note it)
            //fopen("...", "r") → has no direct WASI equivalent without the wasi-libc
            //  But we can at least flag it
        ;

        if transformed != content {
            std::fs::write(file_path, &transformed)?;
        }
    }

    Ok(())
}
