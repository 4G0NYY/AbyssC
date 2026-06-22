//! A from-scratch **Asymmetric Numeral System** entropy coder — the Abyss's own
//! sigil, owing nothing to an external crate.
//!
//! This is a static, order-0, block-based **range ANS** (rANS): a 32-bit state
//! renormalized one byte at a time, with per-symbol probabilities quantized to a
//! total of [`TOTAL`]. Each block carries its own frequency table, so the coder
//! adapts to shifting statistics across a stream.
//!
//! It is wrapped as [`AnsWriter`]/[`AnsReader`] — a streaming [`Write`]/[`Read`]
//! pair — so it slots into the codec layer like any other algorithm, and is reused
//! directly by the encrypted `.abyss` container.
//!
//! rANS is LIFO: symbols are *encoded* last-to-first and *decoded* first-to-last.
//! As an order-0 model it captures a source's symbol frequencies, not its
//! repetitions — it is an entropy stage, not an LZ one.
//!
//! # How it earns its speed
//!
//! - **Interleaved lanes.** Each block is coded with [`N_LANES`] independent rANS
//!   states running over a single shared byte stream (symbol `i` rides lane
//!   `i % N_LANES`). The states have no dependency on one another, so a modern
//!   out-of-order core keeps several in flight at once instead of stalling on a
//!   serial chain.
//! - **Division-free encode.** Each block precomputes a per-symbol reciprocal so
//!   the encoder's hot loop multiplies and shifts instead of dividing.
//! - **One-lookup decode.** A packed `slot → (symbol, freq, cum)` table turns the
//!   decode step into a single table read.
//! - **Parallel blocks.** Blocks are wholly independent — their own table, their
//!   own terminal states — so a batch of them is encoded/decoded across every core
//!   via `rayon`. Only a bounded batch (roughly one block per core) is ever held
//!   in memory at once, so a 100 GB file costs no more RAM than a 100 MB one.

use rayon::prelude::*;
use std::io::{self, Read, Write};

/// Probability resolution: every block's frequencies are normalized to sum to
/// exactly `TOTAL`. 12 bits (4096) is the sweet spot — fine enough for tight
/// coding, small enough that the per-block table stays cheap.
const SCALE_BITS: u32 = 12;
const TOTAL: u32 = 1 << SCALE_BITS;
const MASK: u32 = TOTAL - 1;

/// Lower bound of the rANS state interval. The state lives in `[RANS_L, RANS_L<<8)`
/// and is renormalized by emitting/absorbing whole bytes.
const RANS_L: u32 = 1 << 23;

/// Bytes buffered per block before it is entropy-coded. Larger blocks amortize the
/// frequency table and model the source more tightly; 1 MiB matches the engine's
/// streaming buffer size.
const BLOCK: usize = 1 << 20;

/// Independent rANS states coded in parallel within a single block. Four lanes
/// saturate the execution ports of a typical core without bloating the per-block
/// terminal-state overhead.
const N_LANES: usize = 4;

/// Stream magic. Bumped to `ANS2` when the block format gained interleaved lanes —
/// a decoder fails loudly on an older or non-ANS stream instead of decoding garbage.
const MAGIC: &[u8; 4] = b"ANS2";

fn invalid(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

// --- Frequency model -------------------------------------------------------

/// Tally symbol occurrences. Four interleaved bins break the load-modify-store
/// dependency chain a single histogram would impose, so the (memory-bound) count
/// runs closer to the machine's real throughput.
fn histogram(block: &[u8]) -> [u32; 256] {
    let mut bins = [[0u32; 256]; 4];
    let mut chunks = block.chunks_exact(4);
    for ch in &mut chunks {
        bins[0][ch[0] as usize] += 1;
        bins[1][ch[1] as usize] += 1;
        bins[2][ch[2] as usize] += 1;
        bins[3][ch[3] as usize] += 1;
    }
    for &b in chunks.remainder() {
        bins[0][b as usize] += 1;
    }

    let mut counts = [0u32; 256];
    for i in 0..256 {
        counts[i] = bins[0][i] + bins[1][i] + bins[2][i] + bins[3][i];
    }
    counts
}

/// Quantize raw symbol counts to frequencies summing to exactly [`TOTAL`], with
/// every present symbol guaranteed at least 1 (so it stays decodable).
fn normalize(counts: &[u32; 256]) -> [u16; 256] {
    let total: u64 = counts.iter().map(|&c| c as u64).sum();
    let mut freq = [0u16; 256];
    if total == 0 {
        return freq;
    }

    let mut sum: i64 = 0;
    for (i, &count) in counts.iter().enumerate() {
        if count == 0 {
            continue;
        }
        // Proportional share, floored, then lifted to a minimum of 1.
        let f = ((count as u64 * TOTAL as u64) / total).max(1) as u16;
        freq[i] = f;
        sum += f as i64;
    }

    // Floors and the min-of-1 lift leave the sum off `TOTAL`; nudge the largest
    // symbols until it lands exactly, never letting any present symbol reach 0.
    let mut diff = TOTAL as i64 - sum;
    while diff != 0 {
        // Index of the current largest frequency (the most robust to nudging).
        let (mut bi, mut bf) = (usize::MAX, 0u16);
        for (i, &f) in freq.iter().enumerate() {
            if f > bf {
                bf = f;
                bi = i;
            }
        }
        if diff > 0 {
            freq[bi] += 1;
            diff -= 1;
        } else {
            // Only shave a symbol that can spare it, else move to the next largest.
            if let Some((idx, _)) =
                freq.iter().enumerate().filter(|&(_, &f)| f > 1).max_by_key(|&(_, &f)| f)
            {
                freq[idx] -= 1;
                diff += 1;
            } else {
                break; // Cannot happen: present symbols <= 256 <= TOTAL.
            }
        }
    }
    freq
}

/// Cumulative frequencies: `cum[s]..cum[s + 1]` is symbol `s`'s slot range, and
/// `cum[256] == TOTAL`.
fn cumulative(freq: &[u16; 256]) -> [u32; 257] {
    let mut cum = [0u32; 257];
    for i in 0..256 {
        cum[i + 1] = cum[i] + freq[i] as u32;
    }
    cum
}

// --- Encode / decode symbol tables -----------------------------------------

/// Per-symbol encode parameters. The reciprocal (`rcp_freq`, `rcp_shift`) turns
/// the rANS division by `freq` into a multiply-high and a shift; `bias` and
/// `cmpl_freq` fold the cumulative offset and `TOTAL - freq` into the update.
/// (Alverson's fixed-point reciprocal, the standard ryg-rANS construction.)
#[derive(Clone, Copy)]
struct EncSym {
    x_max: u32,
    rcp_freq: u32,
    rcp_shift: u32,
    bias: u32,
    cmpl_freq: u32,
}

/// Build the per-symbol encode table for one block's frequencies.
fn build_enc(freq: &[u16; 256], cum: &[u32; 257]) -> [EncSym; 256] {
    let mut enc = [EncSym { x_max: 0, rcp_freq: 0, rcp_shift: 0, bias: 0, cmpl_freq: 0 }; 256];
    for s in 0..256 {
        let f = freq[s] as u32;
        let start = cum[s];
        // Renorm threshold: emit bytes until the state can absorb this symbol.
        let x_max = ((RANS_L >> SCALE_BITS) << 8) * f;
        let cmpl_freq = TOTAL - f;
        let (rcp_freq, rcp_shift, bias) = if f < 2 {
            // freq 0 (absent, never encoded) or 1: division by 1 needs no reciprocal.
            (!0u32, 0u32, start + TOTAL - 1)
        } else {
            let mut shift = 0u32;
            while f > (1u32 << shift) {
                shift += 1;
            }
            let rcp = (((1u64 << (shift + 31)) + f as u64 - 1) / f as u64) as u32;
            (rcp, shift - 1, start)
        };
        enc[s] = EncSym { x_max, rcp_freq, rcp_shift, bias, cmpl_freq };
    }
    enc
}

/// Build the packed `slot → (symbol, cum, freq)` decode table, validating that the
/// frequencies sum to exactly [`TOTAL`] (a corrupt or truncated table is rejected,
/// not trusted). Each entry packs `symbol` in bits 0..8, `cum` in 8..20, and
/// `freq - 1` in 20..32 — one load per decoded symbol instead of three.
fn build_dec(freq: &[u16; 256], cum: &[u32; 257]) -> io::Result<Vec<u32>> {
    if cum[256] != TOTAL {
        return Err(invalid("ANS: frequency table does not sum to the expected total"));
    }
    let mut dec = vec![0u32; TOTAL as usize];
    for s in 0..256 {
        let f = freq[s] as u32;
        if f == 0 {
            continue;
        }
        let start = cum[s];
        // start <= 4095 (12 bits) and f-1 <= 4095 (12 bits), so all three fit a u32.
        let packed = (s as u32) | (start << 8) | ((f - 1) << 20);
        for slot in start..start + f {
            dec[slot as usize] = packed;
        }
    }
    Ok(dec)
}

// --- Block codec -----------------------------------------------------------

/// Entropy-code one block with [`N_LANES`] interleaved rANS states.
///
/// rANS encodes back-to-front: renorm bytes are pushed in encode order, then the
/// whole buffer is reversed once so the decoder reads it front-to-back. (Pushing
/// and reversing beats writing into a pre-zeroed scratch buffer — the one O(n)
/// reversal is far cheaper than initializing 1.5–2x the block up front.) The
/// terminal states are pushed last (lane 0 last), so they land first after the
/// reversal.
fn encode_block(data: &[u8], enc: &[EncSym; 256]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2 + 4 * N_LANES + 16);
    let mut state = [RANS_L; N_LANES];

    for i in (0..data.len()).rev() {
        let lane = i & (N_LANES - 1);
        let sym = &enc[data[i] as usize];
        let mut x = state[lane];
        // Renormalize down so the encode step cannot overflow the 32-bit state.
        while x >= sym.x_max {
            out.push((x & 0xff) as u8);
            x >>= 8;
        }
        // x = C(s, x), the division replaced by a reciprocal multiply.
        let q = (((x as u64 * sym.rcp_freq as u64) >> 32) as u32) >> sym.rcp_shift;
        state[lane] = x + sym.bias + q * sym.cmpl_freq;
    }

    // Flush terminal states big-endian and in reverse lane order; after the buffer
    // is reversed they read back little-endian with lane 0 first.
    for lane in (0..N_LANES).rev() {
        out.extend_from_slice(&state[lane].to_be_bytes());
    }
    out.reverse();
    out
}

/// Decode `n` symbols from a block stream produced by [`encode_block`], given the
/// block's frequency table.
fn decode_block(stream: &[u8], n: usize, freq: &[u16; 256]) -> io::Result<Vec<u8>> {
    let truncated = || invalid("ANS: truncated block stream");
    let cum = cumulative(freq);
    let dec = build_dec(freq, &cum)?;

    if stream.len() < 4 * N_LANES {
        return Err(truncated());
    }
    let mut state = [0u32; N_LANES];
    let mut pos = 0usize;
    for lane in &mut state {
        *lane = u32::from_le_bytes([stream[pos], stream[pos + 1], stream[pos + 2], stream[pos + 3]]);
        pos += 4;
    }

    let mut out = vec![0u8; n];
    for i in 0..n {
        let lane = i & (N_LANES - 1);
        let mut x = state[lane];
        let slot = (x & MASK) as usize;
        let packed = dec[slot];
        let sym = (packed & 0xff) as u8;
        let start = (packed >> 8) & 0xfff;
        let f = ((packed >> 20) & 0xfff) + 1;
        x = f * (x >> SCALE_BITS) + slot as u32 - start;
        // Renormalize up by pulling in bytes until the state re-enters its interval.
        while x < RANS_L {
            let byte = *stream.get(pos).ok_or_else(truncated)?;
            x = (x << 8) | byte as u32;
            pos += 1;
        }
        state[lane] = x;
        out[i] = sym;
    }
    Ok(out)
}

/// Serialize one block to its on-wire form, header and stream coalesced into a
/// single buffer: `[raw_len][nsym][(symbol, freq)...][stream_len][stream]`. Pure
/// and self-contained, so a batch of these runs in parallel.
fn serialize_block(block: &[u8]) -> Vec<u8> {
    let counts = histogram(block);
    let freq = normalize(&counts);
    let cum = cumulative(&freq);
    let enc = build_enc(&freq, &cum);
    let stream = encode_block(block, &enc);

    let present: Vec<usize> = (0..256).filter(|&i| freq[i] > 0).collect();
    let mut out = Vec::with_capacity(4 + 2 + present.len() * 3 + 4 + stream.len());
    // raw_len > 0 marks a data block (0 is reserved for the EOF marker).
    out.extend_from_slice(&(block.len() as u32).to_le_bytes());
    out.extend_from_slice(&(present.len() as u16).to_le_bytes());
    for i in present {
        out.push(i as u8);
        out.extend_from_slice(&freq[i].to_le_bytes());
    }
    out.extend_from_slice(&(stream.len() as u32).to_le_bytes());
    out.extend_from_slice(&stream);
    out
}

// --- Streaming writer ------------------------------------------------------

/// A [`Write`] that entropy-codes its input with rANS, one [`BLOCK`] at a time and
/// a batch of blocks across every core.
///
/// Call [`AnsWriter::finish`] to flush the final partial block and the
/// end-of-stream marker; dropping without finishing leaves a truncated stream.
pub struct AnsWriter<W: Write> {
    inner: W,
    buf: Vec<u8>,
    pending: Vec<Vec<u8>>,
    batch: usize,
    finished: bool,
}

impl<W: Write> AnsWriter<W> {
    /// Wrap `inner`, writing the stream magic immediately.
    pub fn new(mut inner: W) -> io::Result<Self> {
        inner.write_all(MAGIC)?;
        let batch = rayon::current_num_threads().max(1);
        Ok(Self {
            inner,
            buf: Vec::with_capacity(BLOCK),
            pending: Vec::with_capacity(batch),
            batch,
            finished: false,
        })
    }

    /// Entropy-code the pending batch in parallel and write the results in order.
    fn flush_batch(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let blocks = std::mem::take(&mut self.pending);
        let encoded: Vec<Vec<u8>> = blocks.par_iter().map(|b| serialize_block(b)).collect();
        for chunk in &encoded {
            self.inner.write_all(chunk)?;
        }
        Ok(())
    }

    /// Flush the final block and write the end-of-stream marker. Idempotent.
    pub fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        if !self.buf.is_empty() {
            let block = std::mem::take(&mut self.buf);
            self.pending.push(block);
        }
        self.flush_batch()?;
        self.inner.write_all(&0u32.to_le_bytes())?; // EOF: a zero-length block.
        self.inner.flush()?;
        self.finished = true;
        Ok(())
    }

    /// Recover the wrapped writer (call after [`AnsWriter::finish`]).
    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for AnsWriter<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut rest = data;

        // Top up a partially-filled block from a previous call first.
        if !self.buf.is_empty() {
            let take = (BLOCK - self.buf.len()).min(rest.len());
            self.buf.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.buf.len() == BLOCK {
                let block = std::mem::replace(&mut self.buf, Vec::with_capacity(BLOCK));
                self.pending.push(block);
                if self.pending.len() >= self.batch {
                    self.flush_batch()?;
                }
            }
        }

        // Slice whole blocks straight from the input — one copy into the batch,
        // no quadratic shuffling of a growing buffer.
        while rest.len() >= BLOCK {
            self.pending.push(rest[..BLOCK].to_vec());
            rest = &rest[BLOCK..];
            if self.pending.len() >= self.batch {
                self.flush_batch()?;
            }
        }

        // Stash whatever is left of this write for next time.
        self.buf.extend_from_slice(rest);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // A streaming entropy coder cannot flush mid-block without ending it, so
        // `flush` only passes through; real finalization is `finish`.
        self.inner.flush()
    }
}

// --- Streaming reader ------------------------------------------------------

/// One block as read off the wire, before decoding.
struct RawBlock {
    raw_len: usize,
    freq: [u16; 256],
    stream: Vec<u8>,
}

/// A [`Read`] that decodes an rANS stream produced by [`AnsWriter`], a batch of
/// blocks across every core.
pub struct AnsReader<R: Read> {
    inner: R,
    out: Vec<u8>,
    pos: usize,
    eof: bool,
    batch: usize,
}

impl<R: Read> AnsReader<R> {
    /// Wrap `inner`, validating the stream magic up front.
    pub fn new(mut inner: R) -> io::Result<Self> {
        let mut magic = [0u8; 4];
        inner.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(invalid("not an Abyss ANS stream (bad or outdated magic)"));
        }
        let batch = rayon::current_num_threads().max(1);
        Ok(Self { inner, out: Vec::new(), pos: 0, eof: false, batch })
    }

    /// Read one block's header and stream off the wire (no decoding yet).
    fn read_block(&mut self) -> io::Result<Option<RawBlock>> {
        let raw_len = read_u32(&mut self.inner)?;
        if raw_len == 0 {
            return Ok(None); // EOF marker.
        }
        if raw_len as usize > BLOCK {
            return Err(invalid("ANS: implausible block length"));
        }

        let nsym = read_u16(&mut self.inner)? as usize;
        if nsym > 256 {
            return Err(invalid("ANS: implausible symbol count"));
        }
        // Pull the whole frequency table in one read, then parse it.
        let mut table = vec![0u8; nsym * 3];
        self.inner.read_exact(&mut table)?;
        let mut freq = [0u16; 256];
        for entry in table.chunks_exact(3) {
            freq[entry[0] as usize] = u16::from_le_bytes([entry[1], entry[2]]);
        }

        let stream_len = read_u32(&mut self.inner)? as usize;
        if stream_len > 2 * BLOCK + 64 {
            return Err(invalid("ANS: implausible stream length"));
        }
        let mut stream = vec![0u8; stream_len];
        self.inner.read_exact(&mut stream)?;

        Ok(Some(RawBlock { raw_len: raw_len as usize, freq, stream }))
    }

    /// Read a batch of blocks, decode them in parallel, and buffer the result.
    fn fill(&mut self) -> io::Result<()> {
        let mut raws: Vec<RawBlock> = Vec::with_capacity(self.batch);
        while raws.len() < self.batch {
            match self.read_block()? {
                Some(rb) => raws.push(rb),
                None => {
                    self.eof = true;
                    break;
                }
            }
        }

        let decoded: Vec<io::Result<Vec<u8>>> =
            raws.par_iter().map(|rb| decode_block(&rb.stream, rb.raw_len, &rb.freq)).collect();

        let mut out = Vec::new();
        for chunk in decoded {
            out.extend_from_slice(&chunk?);
        }
        self.out = out;
        self.pos = 0;
        Ok(())
    }
}

impl<R: Read> Read for AnsReader<R> {
    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.pos < self.out.len() {
                let n = (self.out.len() - self.pos).min(dst.len());
                dst[..n].copy_from_slice(&self.out[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }
            if self.eof {
                return Ok(0);
            }
            self.fill()?;
        }
    }
}

// --- Little-endian reader helpers ------------------------------------------

fn read_u16<R: Read>(r: &mut R) -> io::Result<u16> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
}

fn read_u32<R: Read>(r: &mut R) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(data: &[u8]) {
        let mut compressed = Vec::new();
        {
            let mut w = AnsWriter::new(&mut compressed).unwrap();
            w.write_all(data).unwrap();
            w.finish().unwrap();
        }
        let mut restored = Vec::new();
        let mut r = AnsReader::new(&compressed[..]).unwrap();
        r.read_to_end(&mut restored).unwrap();
        assert_eq!(restored, data);
    }

    #[test]
    fn roundtrips_empty() {
        roundtrip(b"");
    }

    #[test]
    fn roundtrips_single_symbol() {
        roundtrip(&vec![b'x'; 100_000]);
    }

    #[test]
    fn roundtrips_tiny_partial_lanes() {
        // Fewer symbols than lanes — exercises the unused-but-flushed lanes.
        for n in 0..=8usize {
            roundtrip(&vec![b'q'; n]);
            roundtrip(&(0..n as u8).collect::<Vec<u8>>());
        }
    }

    #[test]
    fn roundtrips_text() {
        let mut data = Vec::new();
        for i in 0..20_000u32 {
            data.extend_from_slice(b"The Abyss gazes also. ");
            data.extend_from_slice(&i.to_le_bytes());
        }
        roundtrip(&data);
    }

    #[test]
    fn roundtrips_all_bytes_and_noise() {
        let mut data: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
        // A pseudo-random tail to exercise a flat-ish distribution across blocks.
        let mut state = 0x1234_5678u32;
        for _ in 0..70_000 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            data.push((state >> 24) as u8);
        }
        roundtrip(&data);
    }

    #[test]
    fn roundtrips_multiblock_parallel() {
        // Several megabytes spanning many blocks, mixing structure and noise so the
        // parallel batch path and per-block tables are all exercised.
        let mut data = Vec::with_capacity(5 * 1024 * 1024);
        let mut state = 0xC0FF_EE00u32;
        while data.len() < 5 * 1024 * 1024 {
            data.extend_from_slice(b"depths fold into a glowing orb; ");
            for _ in 0..16 {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                data.push((state >> 24) as u8);
            }
        }
        roundtrip(&data);
    }

    #[test]
    fn roundtrips_exact_block_boundaries() {
        roundtrip(&vec![b'z'; BLOCK]);
        roundtrip(&vec![b'z'; BLOCK + 1]);
        roundtrip(&vec![b'z'; 2 * BLOCK]);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = b"ANS1".to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        assert!(AnsReader::new(&bytes[..]).is_err());
    }

    #[test]
    fn normalize_sums_to_total() {
        let mut counts = [0u32; 256];
        for (i, c) in counts.iter_mut().enumerate() {
            *c = (i as u32) + 1; // a skewed but fully-populated distribution
        }
        let freq = normalize(&counts);
        let sum: u32 = freq.iter().map(|&f| f as u32).sum();
        assert_eq!(sum, TOTAL);
        // Every present symbol keeps a non-zero share.
        assert!(freq.iter().all(|&f| f > 0));
    }
}
