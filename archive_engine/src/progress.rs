//! Lightweight progress reporting.
//!
//! A front-end (e.g. the GUI) hands the engine a shared [`Progress`] and then
//! polls it from another thread while the work runs. The engine only ever does
//! cheap relaxed atomic adds on the hot path — no locks, no channels, no
//! per-write allocation — so progress reporting costs essentially nothing.
//!
//! The *meaning* of the counters is owned by the engine and differs by
//! operation (uncompressed bytes fed in while compressing, bytes consumed while
//! extracting). Callers should treat [`Progress::fraction`] as the single
//! source of truth and not assume a fixed unit.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};

/// A shared, thread-safe progress counter.
///
/// Construct one with [`Progress::new`], pass `&Progress` into an engine
/// `*_with_progress` call on a worker thread, and read [`Progress::fraction`]
/// / [`Progress::processed`] from the UI thread.
#[derive(Debug, Default)]
pub struct Progress {
    processed: AtomicU64,
    total: AtomicU64,
}

impl Progress {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes processed so far.
    pub fn processed(&self) -> u64 {
        self.processed.load(Ordering::Relaxed)
    }

    /// Total work expected, or `0` if not yet known.
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Completion in `0.0..=1.0`. Returns `0.0` while the total is unknown and
    /// is clamped to `1.0` once processing meets or exceeds the estimate (tar
    /// headers can nudge processed slightly past the on-disk total).
    pub fn fraction(&self) -> f32 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        (self.processed() as f32 / total as f32).clamp(0.0, 1.0)
    }

    /// Record the expected total. Called once by the engine when it is known.
    pub(crate) fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
    }

    /// Add to the processed count. Called on every buffered chunk.
    pub(crate) fn add(&self, bytes: u64) {
        self.processed.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// A [`Write`] that tallies bytes written into a [`Progress`] as they pass.
pub(crate) struct CountWriter<'a, W: Write> {
    inner: W,
    progress: &'a Progress,
}

impl<'a, W: Write> CountWriter<'a, W> {
    pub(crate) fn new(inner: W, progress: &'a Progress) -> Self {
        Self { inner, progress }
    }
}

impl<W: Write> Write for CountWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.progress.add(n as u64);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A [`Read`] that tallies bytes read into a [`Progress`] as they pass.
pub(crate) struct CountReader<'a, R: Read> {
    inner: R,
    progress: &'a Progress,
}

impl<'a, R: Read> CountReader<'a, R> {
    pub(crate) fn new(inner: R, progress: &'a Progress) -> Self {
        Self { inner, progress }
    }
}

impl<R: Read> Read for CountReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.progress.add(n as u64);
        Ok(n)
    }
}
