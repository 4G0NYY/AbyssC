//! A from-scratch **Asymmetric Numeral System** entropy coder — the Abyss's own
//! sigil, owing nothing to an external crate.
//!
//! This is a static, order-0, block-based **range ANS** (rANS): a 32-bit state
//! renormalized one byte at a time, with per-symbol probabilities quantized to a
//! total of [`TOTAL`]. Each block carries its own frequency table, so the coder
//! adapts to shifting statistics across a stream while never holding more than one
//! block in memory — a 100 GB file costs the same RAM as a 100 KB one, exactly as
//! the rest of the engine demands.
//!
//! It is wrapped as [`AnsWriter`]/[`AnsReader`] — a streaming [`Write`]/[`Read`]
//! pair — so it slots into the codec layer like any other algorithm, and is reused
//! directly by the encrypted `.abyss` container.
//!
//! rANS is LIFO: symbols are *encoded* last-to-first and *decoded* first-to-last.
//! As an order-0 model it captures a source's symbol frequencies, not its
//! repetitions — it is an entropy stage, not an LZ one.

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

/// Stream magic, so a decoder fails loudly on a non-ANS stream instead of decoding
/// garbage.
const MAGIC: &[u8; 4] = b"ANS1";

// --- Frequency model -------------------------------------------------------

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

/// Build the slot→symbol lookup used to decode, validating that the table sums to
/// exactly [`TOTAL`] (a corrupt or truncated table is rejected, not trusted).
fn slot_to_symbol(cum: &[u32; 257]) -> io::Result<Vec<u8>> {
    if cum[256] != TOTAL {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ANS: frequency table does not sum to the expected total",
        ));
    }
    let mut table = vec![0u8; TOTAL as usize];
    for s in 0..256 {
        for slot in cum[s]..cum[s + 1] {
            table[slot as usize] = s as u8;
        }
    }
    Ok(table)
}

// --- Block codec -----------------------------------------------------------

/// Entropy-code one block. rANS encodes back-to-front; the byte stream is then
/// reversed so the decoder can read it front-to-back. The final 4 bytes (after
/// reversal, the *first* 4) carry the terminal state.
fn encode_block(data: &[u8], freq: &[u16; 256], cum: &[u32; 257]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 2 + 16);
    let mut x: u32 = RANS_L;

    for &b in data.iter().rev() {
        let f = freq[b as usize] as u32;
        let c = cum[b as usize];
        // Renormalize down so the division below cannot overflow the 32-bit state.
        let x_max = ((RANS_L >> SCALE_BITS) << 8) * f;
        while x >= x_max {
            out.push((x & 0xff) as u8);
            x >>= 8;
        }
        x = ((x / f) << SCALE_BITS) + (x % f) + c;
    }

    // Flush the terminal state. The whole buffer is reversed below so the decoder
    // can read front-to-back; pushing the state big-endian here means it lands
    // little-endian (and first) after that reversal.
    out.extend_from_slice(&x.to_be_bytes());
    out.reverse();
    out
}

/// Decode `n` symbols from a block stream produced by [`encode_block`].
fn decode_block(
    stream: &[u8],
    n: usize,
    freq: &[u16; 256],
    cum: &[u32; 257],
    slot2sym: &[u8],
) -> io::Result<Vec<u8>> {
    let truncated =
        || io::Error::new(io::ErrorKind::InvalidData, "ANS: truncated block stream");
    if stream.len() < 4 {
        return Err(truncated());
    }
    let mut x = u32::from_le_bytes([stream[0], stream[1], stream[2], stream[3]]);
    let mut pos = 4;

    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let slot = x & MASK;
        let s = slot2sym[slot as usize];
        let f = freq[s as usize] as u32;
        let c = cum[s as usize];
        x = f * (x >> SCALE_BITS) + slot - c;
        // Renormalize up by pulling in bytes until the state re-enters its interval.
        while x < RANS_L {
            let byte = *stream.get(pos).ok_or_else(truncated)?;
            x = (x << 8) | byte as u32;
            pos += 1;
        }
        out.push(s);
    }
    Ok(out)
}

// --- Streaming writer ------------------------------------------------------

/// A [`Write`] that entropy-codes its input with rANS, one [`BLOCK`] at a time.
///
/// Call [`AnsWriter::finish`] to flush the final partial block and the
/// end-of-stream marker; dropping without finishing leaves a truncated stream.
pub struct AnsWriter<W: Write> {
    inner: W,
    buf: Vec<u8>,
    finished: bool,
}

impl<W: Write> AnsWriter<W> {
    /// Wrap `inner`, writing the stream magic immediately.
    pub fn new(mut inner: W) -> io::Result<Self> {
        inner.write_all(MAGIC)?;
        Ok(Self { inner, buf: Vec::with_capacity(BLOCK), finished: false })
    }

    /// Entropy-code and emit one block: `[raw_len][table][stream_len][stream]`.
    fn emit_block(&mut self, block: &[u8]) -> io::Result<()> {
        let mut counts = [0u32; 256];
        for &b in block {
            counts[b as usize] += 1;
        }
        let freq = normalize(&counts);
        let cum = cumulative(&freq);
        let stream = encode_block(block, &freq, &cum);

        // raw_len > 0 marks a data block (0 is reserved for the EOF marker).
        self.inner.write_all(&(block.len() as u32).to_le_bytes())?;

        // The frequency table: a count of present symbols, then `(symbol, freq)`.
        let present: Vec<usize> = (0..256).filter(|&i| freq[i] > 0).collect();
        self.inner.write_all(&(present.len() as u16).to_le_bytes())?;
        for i in present {
            self.inner.write_all(&[i as u8])?;
            self.inner.write_all(&freq[i].to_le_bytes())?;
        }

        self.inner.write_all(&(stream.len() as u32).to_le_bytes())?;
        self.inner.write_all(&stream)?;
        Ok(())
    }

    /// Flush the final block and write the end-of-stream marker. Idempotent.
    pub fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        if !self.buf.is_empty() {
            let block = std::mem::take(&mut self.buf);
            self.emit_block(&block)?;
        }
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
        self.buf.extend_from_slice(data);
        while self.buf.len() >= BLOCK {
            let block: Vec<u8> = self.buf.drain(..BLOCK).collect();
            self.emit_block(&block)?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // A streaming entropy coder cannot flush mid-block without ending it, so
        // `flush` only passes through; real finalization is `finish`.
        self.inner.flush()
    }
}

// --- Streaming reader ------------------------------------------------------

/// A [`Read`] that decodes an rANS stream produced by [`AnsWriter`].
pub struct AnsReader<R: Read> {
    inner: R,
    out: Vec<u8>,
    pos: usize,
    eof: bool,
}

impl<R: Read> AnsReader<R> {
    /// Wrap `inner`, validating the stream magic up front.
    pub fn new(mut inner: R) -> io::Result<Self> {
        let mut magic = [0u8; 4];
        inner.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not an Abyss ANS stream (bad magic)",
            ));
        }
        Ok(Self { inner, out: Vec::new(), pos: 0, eof: false })
    }

    /// Read, decode, and buffer the next block. Sets `eof` at the marker.
    fn fill(&mut self) -> io::Result<()> {
        let raw_len = read_u32(&mut self.inner)?;
        if raw_len == 0 {
            self.eof = true;
            return Ok(());
        }
        if raw_len as usize > BLOCK {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "ANS: implausible block length"));
        }

        let nsym = read_u16(&mut self.inner)? as usize;
        if nsym > 256 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "ANS: implausible symbol count"));
        }
        let mut freq = [0u16; 256];
        for _ in 0..nsym {
            let sym = read_u8(&mut self.inner)? as usize;
            freq[sym] = read_u16(&mut self.inner)?;
        }
        let cum = cumulative(&freq);
        let slot2sym = slot_to_symbol(&cum)?;

        let stream_len = read_u32(&mut self.inner)? as usize;
        if stream_len > 2 * BLOCK + 64 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "ANS: implausible stream length"));
        }
        let mut stream = vec![0u8; stream_len];
        self.inner.read_exact(&mut stream)?;

        self.out = decode_block(&stream, raw_len as usize, &freq, &cum, &slot2sym)?;
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

fn read_u8<R: Read>(r: &mut R) -> io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

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
    fn rejects_bad_magic() {
        let mut bytes = b"XXXX".to_vec();
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
