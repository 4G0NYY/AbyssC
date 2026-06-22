//! 7-Zip (`.7z`) — **read-only**. We list, browse, and extract members; the
//! surface keeps the right to forge `.7z`, so nothing here creates one.
//!
//! 7z archives are often *solid* (members share a compressed block), so reaching
//! one member can decode the block it lives in. The [`sevenz_rust2`] reader walks
//! folder-by-folder, which keeps that cost contained rather than whole-archive.

use crate::listing::Entry;
use crate::progress::{CountWriter, Progress};
use sevenz_rust2::{ArchiveReader, Password};
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

fn to_io<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

/// Canonicalize an archive-internal path for comparison: forward slashes, no
/// trailing slash.
fn normalize(name: &str) -> String {
    name.replace('\\', "/").trim_end_matches('/').to_string()
}

/// Read the table of contents without decompressing any payload.
pub fn list(src: &Path) -> io::Result<Vec<Entry>> {
    let reader = ArchiveReader::open(src, Password::empty()).map_err(to_io)?;
    let entries = reader
        .archive()
        .files
        .iter()
        .map(|e| Entry {
            name: e.name().to_string(),
            size: Some(e.size()),
            is_dir: e.is_directory(),
        })
        .collect();
    Ok(entries)
}

/// Extract the whole archive into `out_dir`, streaming each member. `progress`
/// counts uncompressed bytes written against the sum of the members' sizes.
pub fn decompress(src: &Path, out_dir: &Path, progress: &Progress) -> io::Result<()> {
    fs::create_dir_all(out_dir)?;
    let mut reader = ArchiveReader::open(src, Password::empty()).map_err(to_io)?;
    let total: u64 = reader
        .archive()
        .files
        .iter()
        .filter(|e| !e.is_directory())
        .map(|e| e.size())
        .sum();
    progress.set_total(total);

    let out_dir = out_dir.to_path_buf();
    reader
        .for_each_entries(|entry, rdr| {
            // `enclosed` rejects absolute paths and `..` traversal out of out_dir.
            let Some(dest) = enclosed(&out_dir, entry.name()) else {
                return Ok(true);
            };
            if entry.is_directory() {
                fs::create_dir_all(&dest)?;
            } else {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut out = BufWriter::with_capacity(1 << 20, File::create(&dest)?);
                let mut counted = CountWriter::new(&mut out, progress);
                io::copy(rdr, &mut counted)?;
                out.flush()?;
            }
            Ok(true)
        })
        .map_err(to_io)?;
    Ok(())
}

/// Extract a single member to `dest`, stopping as soon as it is found.
pub fn extract_member(src: &Path, inner: &str, dest: &Path) -> io::Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let want = normalize(inner);
    let dest = dest.to_path_buf();
    let mut reader = ArchiveReader::open(src, Password::empty()).map_err(to_io)?;
    let mut found = false;
    reader
        .for_each_entries(|entry, rdr| {
            if !entry.is_directory() && normalize(entry.name()) == want {
                let mut out = BufWriter::with_capacity(1 << 20, File::create(&dest)?);
                io::copy(rdr, &mut out)?;
                out.flush()?;
                found = true;
                return Ok(false); // stop iterating: we have our member.
            }
            Ok(true)
        })
        .map_err(to_io)?;
    if !found {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no such member in archive: {inner}"),
        ));
    }
    Ok(())
}

/// Join an archive-internal path under `base`, rejecting absolute paths and any
/// `..` component that would escape `base`.
fn enclosed(base: &Path, inner: &str) -> Option<PathBuf> {
    let mut out = base.to_path_buf();
    for part in inner.replace('\\', "/").split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            p => out.push(p),
        }
    }
    (out != base).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway unique directory under the system temp dir.
    fn scratch(tag: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("abyssc-7z-test-{tag}-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Build a real `.7z` with the writer side, then prove our read paths against
    /// it: full listing, single-member extraction, and whole-archive extraction.
    #[test]
    fn reads_a_real_sevenzip() {
        let root = scratch("root");
        let tree = root.join("tree");
        fs::create_dir_all(tree.join("sub")).unwrap();
        fs::write(tree.join("hello.txt"), b"hello from the abyss").unwrap();
        fs::write(tree.join("sub").join("deep.bin"), b"\x00\x01\x02nested").unwrap();

        let archive = root.join("bundle.7z");
        sevenz_rust2::compress_to_path(&tree, &archive).unwrap();

        // List: both files must surface.
        let entries = list(&archive).unwrap();
        let hello = entries
            .iter()
            .find(|e| !e.is_dir && e.name.replace('\\', "/").ends_with("hello.txt"))
            .expect("hello.txt listed");
        assert_eq!(hello.size, Some(20));

        // Extract just one member — its bytes must match, nothing else written.
        let one = root.join("just_hello.txt");
        extract_member(&archive, &hello.name, &one).unwrap();
        assert_eq!(fs::read(&one).unwrap(), b"hello from the abyss");

        // Missing member is a clean NotFound, not a panic.
        let err = extract_member(&archive, "nope.txt", &root.join("x")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);

        // Full extraction reproduces every file.
        let out = root.join("out");
        decompress(&archive, &out, &Progress::new()).unwrap();
        let deep = walkfind(&out, "deep.bin").expect("deep.bin extracted");
        assert_eq!(fs::read(deep).unwrap(), b"\x00\x01\x02nested");

        let _ = fs::remove_dir_all(&root);
    }

    /// Find a file by base name anywhere under `dir`.
    fn walkfind(dir: &Path, name: &str) -> Option<PathBuf> {
        for entry in fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(hit) = walkfind(&path, name) {
                    return Some(hit);
                }
            } else if path.file_name().and_then(|s| s.to_str()) == Some(name) {
                return Some(path);
            }
        }
        None
    }
}
