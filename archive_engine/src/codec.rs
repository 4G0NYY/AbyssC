//! Compression codecs.
//!
//! Every codec is exposed through the same pair of inversion-of-control helpers:
//! [`Codec::compress`] hands a wrapped [`Write`] to a closure and then *finalizes*
//! the encoder, while [`Codec::decompress`] hands a wrapped [`Read`] to a closure.
//!
//! This keeps each codec's idiosyncratic finalization (`finish()`, drop-to-finish,
//! etc.) contained in one place, and lets the container layer (raw stream, tar, ...)
//! stay completely codec-agnostic.

use std::io::{self, Read, Write};

/// Streaming buffer size used to wrap source/sink handles (1 MiB).
///
/// Large buffers keep syscall overhead low, which is the dominant cost for the
/// very fast codecs (LZ4, Zstd at low levels).
const BUF: usize = 1 << 20;

/// A compression algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// No compression. Only meaningful as the codec behind a `.tar` container.
    Store,
    /// gzip / DEFLATE.
    Gzip,
    /// Zstandard.
    Zstd,
    /// LZ4 (frame format).
    Lz4,
    /// xz / LZMA.
    Xz,
    /// bzip2.
    Bzip2,
    /// Brotli.
    Brotli,
}

/// Tunables shared by every codec. Fields that a codec does not understand are
/// simply ignored, so callers never have to special-case the codec.
#[derive(Clone, Copy, Debug)]
pub struct CodecOptions {
    /// Compression level. `None` means "use the codec's default". The value is
    /// clamped to each codec's valid range, so a single `--level` flag works
    /// across all of them.
    pub level: Option<i32>,
    /// Worker threads for codecs that support parallel encoding (currently Zstd).
    /// `0` means "use all available cores".
    pub threads: u32,
}

impl Default for CodecOptions {
    fn default() -> Self {
        Self { level: None, threads: 0 }
    }
}

impl CodecOptions {
    pub fn new(level: Option<i32>, threads: u32) -> Self {
        Self { level, threads }
    }
}

impl Codec {
    /// Canonical short name, e.g. `"zstd"`.
    pub fn name(self) -> &'static str {
        match self {
            Codec::Store => "store",
            Codec::Gzip => "gzip",
            Codec::Zstd => "zstd",
            Codec::Lz4 => "lz4",
            Codec::Xz => "xz",
            Codec::Bzip2 => "bzip2",
            Codec::Brotli => "brotli",
        }
    }

    /// File-name suffixes that map to this codec for a raw single stream.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Codec::Store => &[],
            Codec::Gzip => &[".gz"],
            Codec::Zstd => &[".zst"],
            Codec::Lz4 => &[".lz4"],
            Codec::Xz => &[".xz"],
            Codec::Bzip2 => &[".bz2"],
            Codec::Brotli => &[".br"],
        }
    }

    /// Wrap `sink` with this codec's encoder, hand the encoder to `write_payload`,
    /// then flush and finalize the stream. The closure receives a `&mut dyn Write`
    /// so the same container code drives every codec.
    pub fn compress<W, F>(self, sink: W, opts: &CodecOptions, write_payload: F) -> io::Result<()>
    where
        W: Write,
        F: FnOnce(&mut dyn Write) -> io::Result<()>,
    {
        let mut sink = io::BufWriter::with_capacity(BUF, sink);
        match self {
            Codec::Store => {
                write_payload(&mut sink)?;
            }
            Codec::Gzip => {
                let level = opts.level.unwrap_or(6).clamp(0, 9) as u32;
                let mut enc =
                    flate2::write::GzEncoder::new(&mut sink, flate2::Compression::new(level));
                write_payload(&mut enc)?;
                enc.finish()?;
            }
            Codec::Zstd => {
                let level = opts.level.unwrap_or(3);
                let mut enc = zstd::stream::write::Encoder::new(&mut sink, level)?;
                let workers = if opts.threads == 0 { default_threads() } else { opts.threads };
                // Falls back to single-threaded if the build lacks multithreading.
                let _ = enc.multithread(workers);
                write_payload(&mut enc)?;
                enc.finish()?;
            }
            Codec::Lz4 => {
                let mut enc = lz4_flex::frame::FrameEncoder::new(&mut sink);
                write_payload(&mut enc)?;
                enc.finish().map_err(to_io)?;
            }
            Codec::Xz => {
                let level = opts.level.unwrap_or(6).clamp(0, 9) as u32;
                let mut enc = xz2::write::XzEncoder::new(&mut sink, level);
                write_payload(&mut enc)?;
                enc.finish()?;
            }
            Codec::Bzip2 => {
                let level = opts.level.unwrap_or(9).clamp(1, 9) as u32;
                let mut enc =
                    bzip2::write::BzEncoder::new(&mut sink, bzip2::Compression::new(level));
                write_payload(&mut enc)?;
                enc.finish()?;
            }
            Codec::Brotli => {
                let quality = opts.level.unwrap_or(6).clamp(0, 11) as u32;
                // `CompressorWriter` writes the stream terminator on drop, so we
                // scope it to force finalization before flushing the sink.
                {
                    let mut enc = brotli::CompressorWriter::new(&mut sink, BUF, quality, 22);
                    write_payload(&mut enc)?;
                    enc.flush()?;
                }
            }
        }
        sink.flush()?;
        Ok(())
    }

    /// Wrap `source` with this codec's decoder and hand it to `read_payload`.
    pub fn decompress<R, F>(self, source: R, read_payload: F) -> io::Result<()>
    where
        R: Read,
        F: FnOnce(&mut dyn Read) -> io::Result<()>,
    {
        let mut source = io::BufReader::with_capacity(BUF, source);
        match self {
            Codec::Store => {
                read_payload(&mut source)?;
            }
            Codec::Gzip => {
                let mut dec = flate2::read::GzDecoder::new(&mut source);
                read_payload(&mut dec)?;
            }
            Codec::Zstd => {
                let mut dec = zstd::stream::read::Decoder::new(&mut source)?;
                read_payload(&mut dec)?;
            }
            Codec::Lz4 => {
                let mut dec = lz4_flex::frame::FrameDecoder::new(&mut source);
                read_payload(&mut dec)?;
            }
            Codec::Xz => {
                let mut dec = xz2::read::XzDecoder::new(&mut source);
                read_payload(&mut dec)?;
            }
            Codec::Bzip2 => {
                let mut dec = bzip2::read::BzDecoder::new(&mut source);
                read_payload(&mut dec)?;
            }
            Codec::Brotli => {
                let mut dec = brotli::Decompressor::new(&mut source, BUF);
                read_payload(&mut dec)?;
            }
        }
        Ok(())
    }
}

fn to_io<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

fn default_threads() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1)
}
