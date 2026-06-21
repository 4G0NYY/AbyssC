//! High-level compression entry point. Picks the container strategy and lets the
//! codec layer handle the actual byte crunching.

use crate::codec::CodecOptions;
use crate::format::{Container, Format};
use crate::{util, zip_archive};
use std::fs::{self, File};
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};

/// Outcome of a compression run, for reporting.
#[derive(Debug, Clone, Copy)]
pub struct Report {
    /// Total size of the inputs before compression.
    pub uncompressed: u64,
    /// Size of the produced archive.
    pub compressed: u64,
}

impl Report {
    /// Compressed / uncompressed (0.0 if there was no input).
    pub fn ratio(&self) -> f64 {
        if self.uncompressed == 0 {
            0.0
        } else {
            self.compressed as f64 / self.uncompressed as f64
        }
    }
}

/// Compress `inputs` into `dest` using `format`.
///
/// - `Raw` formats require exactly one regular file.
/// - `Tar` and `Zip` accept any mix of files and directories.
pub fn compress(
    inputs: &[PathBuf],
    dest: &Path,
    format: Format,
    opts: &CodecOptions,
) -> io::Result<Report> {
    if inputs.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no input paths given"));
    }
    for path in inputs {
        if !path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("input not found: {}", path.display()),
            ));
        }
    }

    let uncompressed = util::total_size(inputs);

    match format.container {
        Container::Zip => zip_archive::compress(inputs, dest, opts)?,
        Container::Raw => {
            if inputs.len() != 1 || inputs[0].is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "single-stream formats (.gz/.zst/.lz4/.xz/.bz2/.br) take exactly one file; \
                     use a .tar.* or .zip target for multiple files or directories",
                ));
            }
            let mut src = BufReader::with_capacity(1 << 20, File::open(&inputs[0])?);
            let out = File::create(dest)?;
            format.codec.compress(out, opts, |w| {
                io::copy(&mut src, w)?;
                Ok(())
            })?;
        }
        Container::Tar => {
            let out = File::create(dest)?;
            format.codec.compress(out, opts, |w| build_tar(inputs, w))?;
        }
    }

    let compressed = util::path_size(dest);
    Ok(Report { uncompressed, compressed })
}

/// Stream the inputs into a tar archive written to `writer`. Directories are
/// added recursively; archive paths are kept relative to each input's name.
fn build_tar(inputs: &[PathBuf], writer: &mut dyn Write) -> io::Result<()> {
    let mut builder = tar::Builder::new(writer);
    builder.follow_symlinks(false);

    for input in inputs {
        let name = input.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "input has no file name")
        })?;
        let meta = fs::symlink_metadata(input)?;
        if meta.is_dir() {
            builder.append_dir_all(name, input)?;
        } else {
            builder.append_path_with_name(input, name)?;
        }
    }

    builder.finish()?;
    Ok(())
}
