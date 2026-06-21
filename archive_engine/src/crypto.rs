//! Password-based authenticated encryption for sealed archives.
//!
//! A wall the surface cannot climb. Two well-trodden primitives, no hand-rolled
//! cryptography:
//!
//! - **Argon2id** stretches the password into a 256-bit key, salted per archive.
//!   It is deliberately slow and memory-hard, so brute force costs dearly.
//! - **ChaCha20-Poly1305** in a STREAM construction encrypts *and authenticates*
//!   the payload in 64 KiB chunks. Each chunk carries a tag, so any tampering,
//!   truncation, or reordering is caught — and a wrong password fails cleanly
//!   instead of yielding plausible-looking garbage. ChaCha is fast in pure
//!   software, needing no AES hardware, which keeps throughput high everywhere.
//!
//! Exposed as a [`Write`]/[`Read`] pair so it layers over any stream — here, the
//! ANS-coded payload of a `.abyss` archive.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::generic_array::GenericArray;
use chacha20poly1305::aead::stream::{DecryptorBE32, EncryptorBE32};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use rand::RngCore;
use std::io::{self, Read, Write};
use zeroize::Zeroizing;

/// Header magic for the encryption layer.
const MAGIC: &[u8; 4] = b"ABYX";
/// Layer version, bumped if the on-disk shape ever changes.
const VERSION: u8 = 1;
/// Plaintext bytes per authenticated chunk.
const CHUNK: usize = 64 * 1024;
/// Random salt length fed to Argon2 (bytes).
const SALT_LEN: usize = 16;
/// STREAM nonce-prefix length for ChaCha20-Poly1305: its 12-byte nonce, less the
/// 5 bytes the BE32 construction spends on a counter and last-block flag.
const NONCE_LEN: usize = 7;
/// A chunk record tag.
const TAG_MORE: u8 = 0;
const TAG_LAST: u8 = 1;

/// Derive a 256-bit key from `password` and `salt` with the given Argon2 cost
/// parameters. Returned zeroizing so the key is wiped from memory on drop.
fn derive_key(
    password: &str,
    salt: &[u8],
    params: Params,
) -> io::Result<Zeroizing<[u8; 32]>> {
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(password.as_bytes(), salt, key.as_mut_slice())
        .map_err(|e| io::Error::other(format!("key derivation failed: {e}")))?;
    Ok(key)
}

fn aead_error(_e: chacha20poly1305::aead::Error) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "decryption failed — wrong password or a corrupted archive",
    )
}

// --- Encrypting writer -----------------------------------------------------

/// A [`Write`] that encrypts everything written through it, emitting a
/// self-describing header followed by length-framed authenticated chunks.
///
/// Call [`EncryptWriter::finish`] to seal the final chunk; without it the stream
/// is incomplete and will refuse to decrypt.
pub struct EncryptWriter<W: Write> {
    inner: W,
    enc: Option<EncryptorBE32<ChaCha20Poly1305>>,
    buf: Vec<u8>,
    finished: bool,
}

impl<W: Write> EncryptWriter<W> {
    /// Wrap `inner`, deriving a fresh key and writing the encryption header.
    pub fn new(mut inner: W, password: &str) -> io::Result<Self> {
        let mut salt = [0u8; SALT_LEN];
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        rand::rngs::OsRng.fill_bytes(&mut nonce);

        let params = Params::default();
        let key = derive_key(password, &salt, params.clone())?;
        let cipher = ChaCha20Poly1305::new_from_slice(key.as_slice())
            .map_err(|e| io::Error::other(e.to_string()))?;
        let enc = EncryptorBE32::from_aead(cipher, GenericArray::from_slice(&nonce));

        // Header: magic, version, salt, nonce, then the Argon2 cost parameters so
        // a decoder can reproduce the exact key derivation.
        inner.write_all(MAGIC)?;
        inner.write_all(&[VERSION])?;
        inner.write_all(&salt)?;
        inner.write_all(&nonce)?;
        inner.write_all(&params.m_cost().to_le_bytes())?;
        inner.write_all(&params.t_cost().to_le_bytes())?;
        inner.write_all(&params.p_cost().to_le_bytes())?;

        Ok(Self { inner, enc: Some(enc), buf: Vec::with_capacity(CHUNK), finished: false })
    }

    fn write_record(&mut self, tag: u8, ciphertext: &[u8]) -> io::Result<()> {
        self.inner.write_all(&[tag])?;
        self.inner.write_all(&(ciphertext.len() as u32).to_le_bytes())?;
        self.inner.write_all(ciphertext)?;
        Ok(())
    }

    /// Seal the final chunk and flush. Idempotent.
    pub fn finish(&mut self) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        let enc = self.enc.take().expect("encryptor present until finish");
        let last = std::mem::take(&mut self.buf);
        let ct = enc.encrypt_last(last.as_slice()).map_err(aead_error)?;
        self.write_record(TAG_LAST, &ct)?;
        self.inner.flush()?;
        self.finished = true;
        Ok(())
    }

    /// Recover the wrapped writer (call after [`EncryptWriter::finish`]).
    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> Write for EncryptWriter<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        // Emit only when strictly more than a chunk is buffered, so the chunk we
        // seal is never the last one — the last must go through `encrypt_last`.
        while self.buf.len() > CHUNK {
            let chunk: Vec<u8> = self.buf.drain(..CHUNK).collect();
            let enc = self.enc.as_mut().expect("encryptor present before finish");
            let ct = enc.encrypt_next(chunk.as_slice()).map_err(aead_error)?;
            self.write_record(TAG_MORE, &ct)?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

// --- Decrypting reader -----------------------------------------------------

/// A [`Read`] that decrypts a stream produced by [`EncryptWriter`], verifying the
/// authentication tag on every chunk.
pub struct DecryptReader<R: Read> {
    inner: R,
    dec: Option<DecryptorBE32<ChaCha20Poly1305>>,
    out: Vec<u8>,
    pos: usize,
    done: bool,
}

impl<R: Read> DecryptReader<R> {
    /// Wrap `inner`, reading the header and re-deriving the key from `password`.
    pub fn new(mut inner: R, password: &str) -> io::Result<Self> {
        let mut magic = [0u8; 4];
        inner.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not an encrypted Abyss stream (bad magic)",
            ));
        }
        let mut version = [0u8; 1];
        inner.read_exact(&mut version)?;
        if version[0] != VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported encryption version {}", version[0]),
            ));
        }

        let mut salt = [0u8; SALT_LEN];
        inner.read_exact(&mut salt)?;
        let mut nonce = [0u8; NONCE_LEN];
        inner.read_exact(&mut nonce)?;
        let m_cost = read_u32(&mut inner)?;
        let t_cost = read_u32(&mut inner)?;
        let p_cost = read_u32(&mut inner)?;

        let params = Params::new(m_cost, t_cost, p_cost, None)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let key = derive_key(password, &salt, params)?;
        let cipher = ChaCha20Poly1305::new_from_slice(key.as_slice())
            .map_err(|e| io::Error::other(e.to_string()))?;
        let dec = DecryptorBE32::from_aead(cipher, GenericArray::from_slice(&nonce));

        Ok(Self { inner, dec: Some(dec), out: Vec::new(), pos: 0, done: false })
    }

    /// Read and decrypt the next chunk record into the output buffer.
    fn fill(&mut self) -> io::Result<()> {
        let mut tag = [0u8; 1];
        self.inner.read_exact(&mut tag)?;
        let len = read_u32(&mut self.inner)? as usize;
        // A valid record is at most one chunk plus the 16-byte Poly1305 tag.
        if len > CHUNK + 16 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "encrypted chunk too large"));
        }
        let mut ct = vec![0u8; len];
        self.inner.read_exact(&mut ct)?;

        let plaintext = match tag[0] {
            TAG_LAST => {
                let dec = self.dec.take().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "encrypted stream ended twice")
                })?;
                self.done = true;
                dec.decrypt_last(ct.as_slice()).map_err(aead_error)?
            }
            TAG_MORE => {
                let dec = self.dec.as_mut().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "data after end of encrypted stream")
                })?;
                dec.decrypt_next(ct.as_slice()).map_err(aead_error)?
            }
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unknown encrypted chunk tag {other}"),
                ));
            }
        };
        self.out = plaintext;
        self.pos = 0;
        Ok(())
    }
}

impl<R: Read> Read for DecryptReader<R> {
    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.pos < self.out.len() {
                let n = (self.out.len() - self.pos).min(dst.len());
                dst[..n].copy_from_slice(&self.out[self.pos..self.pos + n]);
                self.pos += n;
                return Ok(n);
            }
            if self.done {
                return Ok(0);
            }
            self.fill()?;
        }
    }
}

fn read_u32<R: Read>(r: &mut R) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seal(data: &[u8], password: &str) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut w = EncryptWriter::new(&mut out, password).unwrap();
            w.write_all(data).unwrap();
            w.finish().unwrap();
        }
        out
    }

    #[test]
    fn roundtrips_with_correct_password() {
        for len in [0usize, 1, CHUNK - 1, CHUNK, CHUNK + 1, 5 * CHUNK + 7] {
            let data: Vec<u8> = (0..len).map(|i| (i * 31 + 7) as u8).collect();
            let sealed = seal(&data, "the abyss remembers");
            let mut restored = Vec::new();
            DecryptReader::new(&sealed[..], "the abyss remembers")
                .unwrap()
                .read_to_end(&mut restored)
                .unwrap();
            assert_eq!(restored, data, "len {len}");
        }
    }

    #[test]
    fn wrong_password_fails() {
        let sealed = seal(b"a secret kept in the deep", "correct horse");
        let mut restored = Vec::new();
        let result = DecryptReader::new(&sealed[..], "battery staple")
            .unwrap()
            .read_to_end(&mut restored);
        assert!(result.is_err(), "a wrong password must not decrypt");
    }

    #[test]
    fn tampering_is_detected() {
        let mut sealed = seal(b"integrity matters in the depths", "key");
        // Flip a byte deep in the ciphertext body, past the header.
        let i = sealed.len() - 4;
        sealed[i] ^= 0x80;
        let mut restored = Vec::new();
        let result =
            DecryptReader::new(&sealed[..], "key").unwrap().read_to_end(&mut restored);
        assert!(result.is_err(), "tampered ciphertext must fail authentication");
    }
}
