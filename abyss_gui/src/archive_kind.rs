//! The curated set of archive formats the GUI offers.
//!
//! Like WinRAR, the GUI always produces a *container* (so it never trips over
//! the "single stream takes one file" rule). Each kind knows its engine
//! [`Format`], the file extension to suggest, its level range, and a one-line
//! description for the chooser.

use archive_engine::{Codec, Container, Format};
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArchiveKind {
    Zstd,
    Lz4,
    Xz,
    Gzip,
    Bzip2,
    Brotli,
    Zip,
    TarStore,
}

impl ArchiveKind {
    /// Order shown in the chooser — fastest-balanced first, niche last.
    pub const ALL: [ArchiveKind; 8] = [
        ArchiveKind::Zstd,
        ArchiveKind::Lz4,
        ArchiveKind::Xz,
        ArchiveKind::Brotli,
        ArchiveKind::Gzip,
        ArchiveKind::Bzip2,
        ArchiveKind::Zip,
        ArchiveKind::TarStore,
    ];

    pub fn format(self) -> Format {
        use ArchiveKind::*;
        match self {
            Zstd => Format::new(Container::Tar, Codec::Zstd),
            Lz4 => Format::new(Container::Tar, Codec::Lz4),
            Xz => Format::new(Container::Tar, Codec::Xz),
            Gzip => Format::new(Container::Tar, Codec::Gzip),
            Bzip2 => Format::new(Container::Tar, Codec::Bzip2),
            Brotli => Format::new(Container::Tar, Codec::Brotli),
            Zip => Format::new(Container::Zip, Codec::Store),
            TarStore => Format::new(Container::Tar, Codec::Store),
        }
    }

    /// Extension to append when suggesting an output name.
    pub fn extension(self) -> &'static str {
        use ArchiveKind::*;
        match self {
            Zstd => ".tar.zst",
            Lz4 => ".tar.lz4",
            Xz => ".tar.xz",
            Gzip => ".tar.gz",
            Bzip2 => ".tar.bz2",
            Brotli => ".tar.br",
            Zip => ".zip",
            TarStore => ".tar",
        }
    }

    /// `(min, max, default)` for the level slider, or `None` if level is ignored.
    pub fn level_range(self) -> Option<(i32, i32, i32)> {
        use ArchiveKind::*;
        match self {
            Zstd => Some((1, 22, 3)),
            Gzip => Some((0, 9, 6)),
            Xz => Some((0, 9, 6)),
            Bzip2 => Some((1, 9, 9)),
            Brotli => Some((0, 11, 6)),
            Zip => Some((0, 9, 6)),
            Lz4 | TarStore => None,
        }
    }

    /// Default level (used to reset the slider when the kind changes).
    pub fn default_level(self) -> i32 {
        self.level_range().map(|(_, _, d)| d).unwrap_or(0)
    }

    /// Whether this codec uses the worker-thread control.
    pub fn uses_threads(self) -> bool {
        matches!(self, ArchiveKind::Zstd)
    }

    /// A short description shown beneath the chooser.
    pub fn tagline(self) -> &'static str {
        use ArchiveKind::*;
        match self {
            Zstd => "Balanced speed and ratio. Claims every core.",
            Lz4 => "Raw velocity. The fastest blade.",
            Xz => "Crushes hardest, moves slowest.",
            Gzip => "The old, ubiquitous standard.",
            Bzip2 => "Legacy weight.",
            Brotli => "The web's chosen ratio.",
            Zip => "Portable. Deflate per entry.",
            TarStore => "Bundle only. No compression.",
        }
    }

    fn short(self) -> &'static str {
        use ArchiveKind::*;
        match self {
            Zstd => "Zstandard",
            Lz4 => "LZ4",
            Xz => "XZ / LZMA",
            Gzip => "Gzip",
            Bzip2 => "Bzip2",
            Brotli => "Brotli",
            Zip => "ZIP",
            TarStore => "Tar (store)",
        }
    }
}

impl fmt::Display for ArchiveKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}   ({})", self.short(), self.extension())
    }
}
