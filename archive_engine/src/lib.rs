//! AbyssC archive engine: a modular, streaming compression core.
//!
//! An archive [`Format`] pairs a [`Container`] (how files are laid out) with a
//! [`Codec`] (how bytes are compressed). The codec layer wraps streams via an
//! inversion-of-control API so containers stay codec-agnostic and nothing is
//! ever buffered in memory in full.

mod abyss;
mod ans;
pub mod codec;
pub mod compress;
mod crypto;
pub mod decompress;
pub mod format;
mod iso_archive;
pub mod listing;
pub mod progress;
mod rar_archive;
mod sevenz_archive;
mod util;
mod zip_archive;

pub use codec::{Codec, CodecOptions};
pub use compress::{Report, compress, compress_with_progress};
pub use decompress::{decompress, decompress_with_progress, extract_member};
pub use format::{Container, Format};
pub use listing::{Entry, Listing, list};
pub use progress::Progress;
