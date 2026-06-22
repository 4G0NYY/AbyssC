//! RAR (`.rar`) — **read-only**. We list, browse, and extract; RAR creation is
//! proprietary, so nothing here forges one. Backed by the official unrar C source
//! through the [`unrar`] crate.
//!
//! RAR is a sequential format with a typestate API: each member is reached by
//! reading its header, then either extracting or skipping it before the next
//! header can be read. We honor that dance below.

use crate::listing::Entry;
use crate::progress::Progress;
use std::fs;
use std::io;
use std::path::Path;
use unrar::Archive;

fn to_io<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

/// Canonicalize an archive-internal path for comparison: forward slashes, no
/// trailing slash. RAR stores paths with platform separators; this levels them.
fn normalize(name: &str) -> String {
    name.replace('\\', "/").trim_end_matches('/').to_string()
}

/// Read the table of contents without extracting.
pub fn list(src: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for header in Archive::new(src).open_for_listing().map_err(to_io)? {
        let header = header.map_err(to_io)?;
        entries.push(Entry {
            name: header.filename.to_string_lossy().replace('\\', "/"),
            size: Some(header.unpacked_size),
            is_dir: header.is_directory(),
        });
    }
    Ok(entries)
}

/// Extract the whole archive into `out_dir`. `progress` counts uncompressed bytes
/// against the sum of the members' sizes (gathered in a cheap listing pass).
pub fn decompress(src: &Path, out_dir: &Path, progress: &Progress) -> io::Result<()> {
    fs::create_dir_all(out_dir)?;
    let total: u64 = Archive::new(src)
        .open_for_listing()
        .map_err(to_io)?
        .filter_map(|h| h.ok())
        .filter(|h| !h.is_directory())
        .map(|h| h.unpacked_size)
        .sum();
    progress.set_total(total);

    let mut archive = Archive::new(src).open_for_processing().map_err(to_io)?;
    while let Some(cursor) = archive.read_header().map_err(to_io)? {
        let size = cursor.entry().unpacked_size;
        let is_dir = cursor.entry().is_directory();
        archive = cursor.extract_with_base(out_dir).map_err(to_io)?;
        if !is_dir {
            progress.add(size);
        }
    }
    Ok(())
}

/// Extract a single member to `dest`, skipping every other member.
pub fn extract_member(src: &Path, inner: &str, dest: &Path) -> io::Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let want = normalize(inner);
    let mut archive = Archive::new(src).open_for_processing().map_err(to_io)?;
    while let Some(cursor) = archive.read_header().map_err(to_io)? {
        let name = normalize(&cursor.entry().filename.to_string_lossy());
        if !cursor.entry().is_directory() && name == want {
            cursor.extract_to(dest).map_err(to_io)?;
            return Ok(());
        }
        archive = cursor.skip().map_err(to_io)?;
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("no such member in archive: {inner}"),
    ))
}
