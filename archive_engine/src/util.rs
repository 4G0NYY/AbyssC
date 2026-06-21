//! Small filesystem helpers shared across the engine.

use std::fs;
use std::path::{Path, PathBuf};

/// Total size in bytes of every input, recursing into directories. Symlinks are
/// not followed (their own metadata size is counted). Unreadable paths count 0.
pub fn total_size(paths: &[PathBuf]) -> u64 {
    paths.iter().map(|p| path_size(p)).sum()
}

/// Size in bytes of a single path, recursing into directories.
pub fn path_size(path: &Path) -> u64 {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => {
            let mut total = 0;
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    total += path_size(&entry.path());
                }
            }
            total
        }
        Ok(meta) => meta.len(),
        Err(_) => 0,
    }
}
