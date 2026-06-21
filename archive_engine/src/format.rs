//! Archive format = a container (how files are laid out) + a codec (how bytes
//! are compressed). Formats are detected from file names so the CLI can stay a
//! thin dispatcher.

use crate::codec::Codec;
use std::path::Path;

/// How the payload is laid out before/after the codec runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    /// A single compressed stream (`.gz`, `.zst`, ...). Exactly one input file.
    Raw,
    /// A tar stream wrapped in a codec (`.tar.zst`, ...). Any number of files/dirs.
    Tar,
    /// A ZIP archive (compression handled per-entry by the zip crate).
    Zip,
}

/// A fully-resolved archive format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub container: Container,
    pub codec: Codec,
}

impl Format {
    pub const fn new(container: Container, codec: Codec) -> Self {
        Self { container, codec }
    }

    /// Detect a format from a path's file name (case-insensitive). Longer/double
    /// extensions (`.tar.gz`) are matched before single ones (`.gz`).
    pub fn from_path(path: &Path) -> Option<Format> {
        let name = path.file_name()?.to_str()?.to_ascii_lowercase();
        Self::from_filename(&name)
    }

    fn from_filename(name: &str) -> Option<Format> {
        use Codec::*;
        use Container::*;
        // Ordered longest-first so e.g. ".tar.gz" wins over ".gz".
        const TABLE: &[(&str, Container, Codec)] = &[
            (".tar.gz", Tar, Gzip),
            (".tgz", Tar, Gzip),
            (".tar.zst", Tar, Zstd),
            (".tzst", Tar, Zstd),
            (".tar.xz", Tar, Xz),
            (".txz", Tar, Xz),
            (".tar.bz2", Tar, Bzip2),
            (".tbz2", Tar, Bzip2),
            (".tbz", Tar, Bzip2),
            (".tar.lz4", Tar, Lz4),
            (".tar.br", Tar, Brotli),
            (".tar", Tar, Store),
            (".zip", Zip, Store),
            (".gz", Raw, Gzip),
            (".zst", Raw, Zstd),
            (".lz4", Raw, Lz4),
            (".xz", Raw, Xz),
            (".bz2", Raw, Bzip2),
            (".br", Raw, Brotli),
        ];
        TABLE
            .iter()
            .find(|(ext, _, _)| name.ends_with(ext))
            .map(|&(_, container, codec)| Format::new(container, codec))
    }

    /// Parse an explicit format name (used by a `--format` override).
    pub fn from_name(s: &str) -> Option<Format> {
        use Codec::*;
        use Container::*;
        let s = s.trim().to_ascii_lowercase();
        Some(match s.as_str() {
            "zip" => Format::new(Zip, Store),
            "tar" => Format::new(Tar, Store),
            "gz" | "gzip" => Format::new(Raw, Gzip),
            "zst" | "zstd" => Format::new(Raw, Zstd),
            "lz4" => Format::new(Raw, Lz4),
            "xz" | "lzma" => Format::new(Raw, Xz),
            "bz2" | "bzip2" => Format::new(Raw, Bzip2),
            "br" | "brotli" => Format::new(Raw, Brotli),
            "tgz" | "tar.gz" => Format::new(Tar, Gzip),
            "tzst" | "tar.zst" => Format::new(Tar, Zstd),
            "txz" | "tar.xz" => Format::new(Tar, Xz),
            "tbz2" | "tbz" | "tar.bz2" => Format::new(Tar, Bzip2),
            "tar.lz4" => Format::new(Tar, Lz4),
            "tar.br" => Format::new(Tar, Brotli),
            _ => return None,
        })
    }

    /// Human-readable label such as `"tar.zst"` or `"zip"`.
    pub fn label(&self) -> String {
        match self.container {
            Container::Zip => "zip".to_string(),
            Container::Raw => self.codec.name().to_string(),
            Container::Tar => match self.codec {
                Codec::Store => "tar".to_string(),
                codec => format!("tar.{}", codec.name()),
            },
        }
    }
}
