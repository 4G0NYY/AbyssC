//! AbyssC archive engine: a modular, streaming compression core.
//!
//! An archive [`Format`] pairs a [`Container`] (how files are laid out) with a
//! [`Codec`] (how bytes are compressed). The codec layer wraps streams via an
//! inversion-of-control API so containers stay codec-agnostic and nothing is
//! ever buffered in memory in full.

pub mod codec;
pub mod compress;
pub mod decompress;
pub mod format;
pub mod listing;
mod util;
mod zip_archive;

pub use codec::{Codec, CodecOptions};
pub use compress::{Report, compress};
pub use decompress::decompress;
pub use format::{Container, Format};
pub use listing::{Entry, Listing, list};

use std::io;
use std::path::Path;

/// Backwards-compatible helper: compress a single file into a `.zip` archive.
pub fn compress_file(src: &Path, dest: &Path) -> io::Result<()> {
    compress(
        &[src.to_path_buf()],
        dest,
        Format::new(Container::Zip, Codec::Store),
        &CodecOptions::default(),
    )
    .map(|_| ())
}

/// Backwards-compatible helper: extract a `.zip` archive into a directory.
pub fn decompress_archive(src: &Path, out_dir: &Path) -> io::Result<()> {
    decompress(src, out_dir, Format::new(Container::Zip, Codec::Store))
}
