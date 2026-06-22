//! ISO-9660 (`.iso`) disc images — **read-only**. We list, browse, and extract
//! files. Backed by the pure-Rust [`iso9660`] reader.
//!
//! ISO directory records carry a `;1` version suffix on file identifiers and the
//! special `.`/`..` entries; both are smoothed away so the listing matches what a
//! user expects to see.

use crate::listing::Entry;
use crate::progress::{CountWriter, Progress};
use iso9660::{DirectoryEntry, ISO9660, ISODirectory, ISOFile};
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

fn to_io<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

/// Strip an ISO-9660 file-version suffix (`README.TXT;1` -> `README.TXT`).
fn clean(identifier: &str) -> String {
    match identifier.rsplit_once(';') {
        Some((stem, ver)) if !ver.is_empty() && ver.bytes().all(|b| b.is_ascii_digit()) => {
            stem.to_string()
        }
        _ => identifier.to_string(),
    }
}

/// Read the table of contents (recursively) without extracting.
pub fn list(src: &Path) -> io::Result<Vec<Entry>> {
    let iso = ISO9660::new(File::open(src)?).map_err(to_io)?;
    let mut entries = Vec::new();
    walk(&iso.root, "", &mut entries)?;
    Ok(entries)
}

fn walk(dir: &ISODirectory<File>, prefix: &str, out: &mut Vec<Entry>) -> io::Result<()> {
    for entry in dir.contents() {
        let entry = entry.map_err(to_io)?;
        let name = clean(entry.identifier());
        if name == "." || name == ".." {
            continue;
        }
        let full = format!("{prefix}{name}");
        match entry {
            DirectoryEntry::Directory(sub) => {
                out.push(Entry { name: format!("{full}/"), size: Some(0), is_dir: true });
                walk(&sub, &format!("{full}/"), out)?;
            }
            DirectoryEntry::File(f) => {
                out.push(Entry { name: full, size: Some(f.size() as u64), is_dir: false });
            }
        }
    }
    Ok(())
}

/// Extract the whole image into `out_dir`. `progress` counts uncompressed bytes
/// against the sum of every file's size.
pub fn decompress(src: &Path, out_dir: &Path, progress: &Progress) -> io::Result<()> {
    fs::create_dir_all(out_dir)?;
    let iso = ISO9660::new(File::open(src)?).map_err(to_io)?;

    let mut listing = Vec::new();
    walk(&iso.root, "", &mut listing)?;
    let total: u64 = listing.iter().filter(|e| !e.is_dir).map(|e| e.size.unwrap_or(0)).sum();
    progress.set_total(total);

    extract_dir(&iso.root, out_dir, progress)
}

fn extract_dir(dir: &ISODirectory<File>, out_dir: &Path, progress: &Progress) -> io::Result<()> {
    for entry in dir.contents() {
        let entry = entry.map_err(to_io)?;
        let name = clean(entry.identifier());
        if name == "." || name == ".." {
            continue;
        }
        let dest = out_dir.join(&name);
        match entry {
            DirectoryEntry::Directory(sub) => {
                fs::create_dir_all(&dest)?;
                extract_dir(&sub, &dest, progress)?;
            }
            DirectoryEntry::File(f) => {
                let mut out = BufWriter::with_capacity(1 << 20, File::create(&dest)?);
                let mut counted = CountWriter::new(&mut out, progress);
                io::copy(&mut f.read(), &mut counted)?;
                out.flush()?;
            }
        }
    }
    Ok(())
}

/// Extract a single file (by its forward-slash internal path) to `dest`.
pub fn extract_member(src: &Path, inner: &str, dest: &Path) -> io::Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let iso = ISO9660::new(File::open(src)?).map_err(to_io)?;
    let want = inner.replace('\\', "/");
    match find(&iso.root, "", &want)? {
        Some(file) => {
            let mut out = BufWriter::with_capacity(1 << 20, File::create(dest)?);
            io::copy(&mut file.read(), &mut out)?;
            out.flush()?;
            Ok(())
        }
        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no such member in archive: {inner}"),
        )),
    }
}

/// Descend toward `want` (a `/`-separated path) and return the matching file.
fn find(dir: &ISODirectory<File>, prefix: &str, want: &str) -> io::Result<Option<ISOFile<File>>> {
    for entry in dir.contents() {
        let entry = entry.map_err(to_io)?;
        let name = clean(entry.identifier());
        if name == "." || name == ".." {
            continue;
        }
        let full = format!("{prefix}{name}");
        match entry {
            DirectoryEntry::Directory(sub) => {
                if want == full || want.starts_with(&format!("{full}/")) {
                    return find(&sub, &format!("{full}/"), want);
                }
            }
            DirectoryEntry::File(f) if full == want => return Ok(Some(f)),
            DirectoryEntry::File(_) => {}
        }
    }
    Ok(None)
}
