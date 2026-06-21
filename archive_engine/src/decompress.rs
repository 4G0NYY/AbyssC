//! High-level decompression entry point.

use crate::codec::Codec;
use crate::format::{Container, Format};
use crate::progress::{CountReader, Progress};
use crate::{abyss, zip_archive};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

/// Extract `src` into `out_dir` using `format`.
///
/// - `Zip`/`Tar`/`Abyss` archives are unpacked into `out_dir`.
/// - `Raw` streams are decompressed to a single file inside `out_dir`, named by
///   stripping the codec's extension from the source file name.
///
/// `password` is only consulted for a sealed `.abyss` archive; pass `None`
/// otherwise. An encrypted archive opened with `None` (or a wrong password) is an
/// error, never a silent failure.
pub fn decompress(
    src: &Path,
    out_dir: &Path,
    format: Format,
    password: Option<&str>,
) -> io::Result<()> {
    decompress_with_progress(src, out_dir, format, &Progress::new(), password)
}

/// Like [`decompress`], but reports streaming progress through `progress`.
///
/// For `Raw`/`Tar`/`Abyss` the counter tracks **compressed bytes read from the
/// source** (total = the archive's on-disk size). For `Zip` it tracks
/// **uncompressed bytes written** (total = the sum of entry sizes), since the zip
/// reader seeks.
pub fn decompress_with_progress(
    src: &Path,
    out_dir: &Path,
    format: Format,
    progress: &Progress,
    password: Option<&str>,
) -> io::Result<()> {
    match format.container {
        Container::Zip => zip_archive::decompress(src, out_dir, progress),
        Container::Abyss => abyss::decompress(src, out_dir, progress, password),
        Container::Tar => {
            fs::create_dir_all(out_dir)?;
            progress.set_total(fs::metadata(src).map(|m| m.len()).unwrap_or(0));
            let file = CountReader::new(File::open(src)?, progress);
            format.codec.decompress(file, |reader| {
                let mut archive = tar::Archive::new(reader);
                archive.unpack(out_dir)?;
                Ok(())
            })
        }
        Container::Raw => {
            fs::create_dir_all(out_dir)?;
            progress.set_total(fs::metadata(src).map(|m| m.len()).unwrap_or(0));
            let target = out_dir.join(raw_output_name(src, format.codec));
            let file = CountReader::new(File::open(src)?, progress);
            format.codec.decompress(file, |reader| {
                let mut out = BufWriter::with_capacity(1 << 20, File::create(&target)?);
                io::copy(reader, &mut out)?;
                out.flush()?;
                Ok(())
            })
        }
    }
}

/// Extract a single member from `src` to the file path `dest`, decompressing
/// only that one entry — the trick that lets the GUI open a file straight out of
/// an archive without unpacking the whole thing.
///
/// `inner` is the member's archive-internal path exactly as it appears in a
/// [`Listing`] (forward-slash separated). For single-stream (`Raw`) formats there
/// is only ever one member, so `inner` is ignored and the whole stream is written.
///
/// Note that `Tar` and `Abyss` are sequential: reaching a member means streaming
/// (and discarding) everything before it, so the cost scales with the member's
/// position, not its size. `Zip` seeks straight to the entry via its index.
///
/// `password` is only consulted for a sealed `.abyss` archive.
pub fn extract_member(
    src: &Path,
    format: Format,
    inner: &str,
    dest: &Path,
    password: Option<&str>,
) -> io::Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    match format.container {
        Container::Abyss => abyss::extract_member(src, inner, dest, password),
        Container::Raw => {
            let file = File::open(src)?;
            format.codec.decompress(file, |reader| {
                let mut out = BufWriter::with_capacity(1 << 20, File::create(dest)?);
                io::copy(reader, &mut out)?;
                out.flush()?;
                Ok(())
            })
        }
        Container::Tar => {
            let want = normalize_member(inner);
            let file = File::open(src)?;
            let mut found = false;
            format.codec.decompress(file, |reader| {
                let mut archive = tar::Archive::new(reader);
                for entry in archive.entries()? {
                    let mut entry = entry?;
                    let name = normalize_member(&entry.path()?.to_string_lossy());
                    if name == want {
                        let mut out = BufWriter::with_capacity(1 << 20, File::create(dest)?);
                        io::copy(&mut entry, &mut out)?;
                        out.flush()?;
                        found = true;
                        break;
                    }
                }
                Ok(())
            })?;
            if !found {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no such member in archive: {inner}"),
                ));
            }
            Ok(())
        }
        Container::Zip => {
            let want = normalize_member(inner);
            let file = BufReader::new(File::open(src)?);
            let mut archive = zip::ZipArchive::new(file)?;
            let mut index = None;
            for i in 0..archive.len() {
                let entry = archive.by_index(i)?;
                if normalize_member(entry.name()) == want {
                    index = Some(i);
                    break;
                }
            }
            let i = index.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no such member in archive: {inner}"),
                )
            })?;
            let mut entry = archive.by_index(i)?;
            let mut out = BufWriter::with_capacity(1 << 20, File::create(dest)?);
            io::copy(&mut entry, &mut out)?;
            out.flush()?;
            Ok(())
        }
    }
}

/// Canonicalize an archive-internal path for comparison: forward slashes, no
/// trailing slash. Tar paths can arrive with platform separators; this levels them.
fn normalize_member(name: &str) -> String {
    name.replace('\\', "/").trim_end_matches('/').to_string()
}

/// Derive the decompressed file name for a raw stream by stripping the codec's
/// suffix (`archive.txt.zst` -> `archive.txt`).
pub(crate) fn raw_output_name(src: &Path, codec: Codec) -> PathBuf {
    if let Some(name) = src.file_name().and_then(|s| s.to_str()) {
        for ext in codec.extensions() {
            if let Some(stripped) = name.strip_suffix(ext) {
                if !stripped.is_empty() {
                    return PathBuf::from(stripped);
                }
            }
        }
    }
    // Fall back to dropping the final extension, then to a generic name.
    src.file_stem()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output"))
}
