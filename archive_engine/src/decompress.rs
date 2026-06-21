//! High-level decompression entry point.

use crate::codec::Codec;
use crate::format::{Container, Format};
use crate::progress::{CountReader, Progress};
use crate::zip_archive;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

/// Extract `src` into `out_dir` using `format`.
///
/// - `Zip`/`Tar` archives are unpacked into `out_dir`.
/// - `Raw` streams are decompressed to a single file inside `out_dir`, named by
///   stripping the codec's extension from the source file name.
pub fn decompress(src: &Path, out_dir: &Path, format: Format) -> io::Result<()> {
    decompress_with_progress(src, out_dir, format, &Progress::new())
}

/// Like [`decompress`], but reports streaming progress through `progress`.
///
/// For `Raw`/`Tar` the counter tracks **compressed bytes read from the source**
/// (total = the archive's on-disk size). For `Zip` it tracks **uncompressed
/// bytes written** (total = the sum of entry sizes), since the zip reader seeks.
pub fn decompress_with_progress(
    src: &Path,
    out_dir: &Path,
    format: Format,
    progress: &Progress,
) -> io::Result<()> {
    match format.container {
        Container::Zip => zip_archive::decompress(src, out_dir, progress),
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
