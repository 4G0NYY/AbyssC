//! ZIP container. Unlike the other formats, ZIP bundles *and* compresses in one
//! pass (DEFLATE per entry), so it bypasses the codec layer entirely. Kept for
//! broad compatibility with other tools; reach for `.tar.zst` when you want speed.

use crate::codec::CodecOptions;
use crate::progress::{CountReader, CountWriter, Progress};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Seek, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// Compress files and/or directories into a `.zip` archive.
///
/// `progress` counts uncompressed bytes read from disk as each entry is added.
pub fn compress(
    inputs: &[PathBuf],
    dest: &Path,
    opts: &CodecOptions,
    progress: &Progress,
) -> io::Result<()> {
    let writer = BufWriter::with_capacity(1 << 20, File::create(dest)?);
    let mut zip = ZipWriter::new(writer);

    let mut options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .large_file(true)
        .unix_permissions(0o644);
    if let Some(level) = opts.level {
        options = options.compression_level(Some(level.clamp(0, 9) as i64));
    }

    for input in inputs {
        let base = input
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid input file name"))?;
        add_path(&mut zip, input, base, options, progress)?;
    }

    zip.finish()?;
    Ok(())
}

/// Recursively add a path to the archive under `arch_name`.
fn add_path<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    disk_path: &Path,
    arch_name: &str,
    options: SimpleFileOptions,
    progress: &Progress,
) -> io::Result<()> {
    let meta = fs::metadata(disk_path)?;
    if meta.is_dir() {
        zip.add_directory(format!("{arch_name}/"), options)?;
        for entry in fs::read_dir(disk_path)? {
            let entry = entry?;
            let child = entry.file_name();
            let child = child.to_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "non-UTF-8 file name")
            })?;
            add_path(zip, &entry.path(), &format!("{arch_name}/{child}"), options, progress)?;
        }
    } else {
        zip.start_file(arch_name.to_string(), options)?;
        let mut src =
            CountReader::new(BufReader::with_capacity(1 << 20, File::open(disk_path)?), progress);
        io::copy(&mut src, zip)?;
    }
    Ok(())
}

/// Extract a `.zip` archive into `out_dir`, streaming each entry.
///
/// `progress` counts uncompressed bytes written; the total is the sum of every
/// entry's stored size (computed up front in one cheap pass over the index).
pub fn decompress(src: &Path, out_dir: &Path, progress: &Progress) -> io::Result<()> {
    let file = BufReader::new(File::open(src)?);
    let mut archive = ZipArchive::new(file)?;
    fs::create_dir_all(out_dir)?;

    let mut total = 0u64;
    for i in 0..archive.len() {
        if let Ok(entry) = archive.by_index(i) {
            if !entry.is_dir() {
                total += entry.size();
            }
        }
    }
    progress.set_total(total);

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        // `enclosed_name` rejects absolute paths and `..` traversal.
        let outpath = match entry.enclosed_name() {
            Some(path) => out_dir.join(path),
            None => continue,
        };

        if entry.is_dir() {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile = CountWriter::new(BufWriter::new(File::create(&outpath)?), progress);
            io::copy(&mut entry, &mut outfile)?;
        }
    }
    Ok(())
}
