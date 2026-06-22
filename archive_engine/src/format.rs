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
    /// The sealed `.abyss` form: a tar bundle, ANS-coded, optionally encrypted.
    /// Like `Zip`, it bundles, compresses, and seals in one pass.
    Abyss,
    /// A 7-Zip (`.7z`) archive. **Read-only** — we list/browse/extract, never create.
    SevenZip,
    /// A RAR (`.rar`) archive. **Read-only** — RAR creation is proprietary.
    Rar,
    /// An ISO-9660 (`.iso`) disc image. **Read-only**.
    Iso,
}

impl Container {
    /// Whether this container can be *created* by the engine. Foreign formats
    /// (`.7z`, `.rar`, `.iso`) are read-only: we open them, but the surface keeps
    /// the right to forge them.
    pub fn writable(self) -> bool {
        !matches!(self, Container::SevenZip | Container::Rar | Container::Iso)
    }
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
            (".tar.ans", Tar, Ans),
            (".tar", Tar, Store),
            (".abyss", Abyss, Ans),
            (".zip", Zip, Store),
            // The ZIP family: all ZIP under the skin, so they ride the same path.
            (".jar", Zip, Store),
            (".war", Zip, Store),
            (".ear", Zip, Store),
            (".apk", Zip, Store),
            (".zipx", Zip, Store),
            // Foreign, read-only containers. Codec is irrelevant (handled within).
            (".7z", SevenZip, Store),
            (".rar", Rar, Store),
            (".iso", Iso, Store),
            (".gz", Raw, Gzip),
            (".zst", Raw, Zstd),
            (".lz4", Raw, Lz4),
            (".xz", Raw, Xz),
            (".bz2", Raw, Bzip2),
            (".br", Raw, Brotli),
            (".ans", Raw, Ans),
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
            "abyss" => Format::new(Abyss, Ans),
            "ans" => Format::new(Raw, Ans),
            "tar.ans" => Format::new(Tar, Ans),
            "zip" | "jar" | "war" | "ear" | "apk" | "zipx" => Format::new(Zip, Store),
            "7z" | "7zip" | "sevenzip" => Format::new(SevenZip, Store),
            "rar" => Format::new(Rar, Store),
            "iso" => Format::new(Iso, Store),
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
            Container::Abyss => "abyss".to_string(),
            Container::SevenZip => "7z".to_string(),
            Container::Rar => "rar".to_string(),
            Container::Iso => "iso".to_string(),
            Container::Raw => self.codec.name().to_string(),
            Container::Tar => match self.codec {
                Codec::Store => "tar".to_string(),
                codec => format!("tar.{}", codec.name()),
            },
        }
    }
}
