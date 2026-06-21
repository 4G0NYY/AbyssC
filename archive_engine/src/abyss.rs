//! The `.abyss` container — the engine's own sealed form.
//!
//! Like `zip`, it bundles *and* crunches *and* (optionally) seals in one pass, so
//! it sits beside the codec layer rather than inside it: it needs to interleave a
//! tar bundle, the from-scratch [`ans`] entropy coder, and the [`crypto`]
//! encryption layer, and to finalize them in the right order — something the
//! generic inversion-of-control codec API cannot express.
//!
//! On-disk shape:
//!
//! ```text
//!   [6]  magic "ABYSSC"
//!   [1]  version
//!   [1]  flags        (bit 0: encrypted)
//!   […]  payload  =  encrypt?( ans( tar(files…) ) )
//! ```
//!
//! The pipeline is **bundle → compress → encrypt** on the way down, and the exact
//! reverse on the way up. Encryption is the outermost layer so an attacker sees
//! only ciphertext — never the compressed size structure of individual members.

use crate::compress::build_tar;
use crate::listing::Entry;
use crate::progress::{CountReader, CountWriter, Progress};
use crate::{ans, crypto};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

/// Streaming buffer size, matching the rest of the engine (1 MiB).
const BUF: usize = 1 << 20;

const MAGIC: &[u8; 6] = b"ABYSSC";
const VERSION: u8 = 1;
const FLAG_ENCRYPTED: u8 = 0b0000_0001;

/// Compress (and optionally encrypt) `inputs` into a `.abyss` archive at `dest`.
///
/// `progress` counts uncompressed input bytes as the tar bundle is fed in.
pub fn compress(
    inputs: &[PathBuf],
    dest: &Path,
    progress: &Progress,
    password: Option<&str>,
) -> io::Result<()> {
    let mut file = BufWriter::with_capacity(BUF, File::create(dest)?);
    write_header(&mut file, password.is_some())?;

    match password {
        // bundle → ANS → encrypt → file
        Some(pw) => {
            let enc = crypto::EncryptWriter::new(file, pw)?;
            let mut ans = ans::AnsWriter::new(enc)?;
            {
                let mut counted = CountWriter::new(&mut ans, progress);
                build_tar(inputs, &mut counted)?;
            }
            ans.finish()?;
            let mut enc = ans.into_inner();
            enc.finish()?;
            enc.into_inner().flush()?;
        }
        // bundle → ANS → file
        None => {
            let mut ans = ans::AnsWriter::new(file)?;
            {
                let mut counted = CountWriter::new(&mut ans, progress);
                build_tar(inputs, &mut counted)?;
            }
            ans.finish()?;
            ans.into_inner().flush()?;
        }
    }
    Ok(())
}

/// Extract a `.abyss` archive into `out_dir`.
///
/// `progress` counts compressed bytes read from disk (total = the archive size).
pub fn decompress(
    src: &Path,
    out_dir: &Path,
    progress: &Progress,
    password: Option<&str>,
) -> io::Result<()> {
    fs::create_dir_all(out_dir)?;
    progress.set_total(fs::metadata(src).map(|m| m.len()).unwrap_or(0));
    let counted = CountReader::new(File::open(src)?, progress);
    let mut reader = open_payload(BufReader::with_capacity(BUF, counted), password)?;
    let mut archive = tar::Archive::new(&mut reader);
    archive.unpack(out_dir)?;
    Ok(())
}

/// List a `.abyss` archive's members without extracting.
pub fn list(src: &Path, password: Option<&str>) -> io::Result<Vec<Entry>> {
    let reader = open_payload(BufReader::with_capacity(BUF, File::open(src)?), password)?;
    let mut archive = tar::Archive::new(reader);
    let mut entries = Vec::new();
    for entry in archive.entries()? {
        let entry = entry?;
        let header = entry.header();
        let is_dir = header.entry_type().is_dir();
        let size = header.size().unwrap_or(0);
        let name = entry.path()?.to_string_lossy().into_owned();
        entries.push(Entry { name, size: Some(size), is_dir });
    }
    Ok(entries)
}

/// Pull a single member out of a `.abyss` archive to `dest`.
pub fn extract_member(
    src: &Path,
    inner: &str,
    dest: &Path,
    password: Option<&str>,
) -> io::Result<()> {
    let want = normalize_member(inner);
    let reader = open_payload(BufReader::with_capacity(BUF, File::open(src)?), password)?;
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = normalize_member(&entry.path()?.to_string_lossy());
        if name == want {
            let mut out = BufWriter::with_capacity(BUF, File::create(dest)?);
            io::copy(&mut entry, &mut out)?;
            out.flush()?;
            return Ok(());
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("no such member in archive: {inner}"),
    ))
}

// --- Internals -------------------------------------------------------------

/// Write the container header (magic, version, flags).
fn write_header<W: Write>(w: &mut W, encrypted: bool) -> io::Result<()> {
    w.write_all(MAGIC)?;
    w.write_all(&[VERSION])?;
    w.write_all(&[if encrypted { FLAG_ENCRYPTED } else { 0 }])?;
    Ok(())
}

/// Read and validate the header, returning whether the archive is encrypted.
fn read_header<R: Read>(r: &mut R) -> io::Result<bool> {
    let mut magic = [0u8; 6];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not an .abyss archive (bad magic)"));
    }
    let mut meta = [0u8; 2];
    r.read_exact(&mut meta)?;
    let (version, flags) = (meta[0], meta[1]);
    if version != VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported .abyss version {version}"),
        ));
    }
    Ok(flags & FLAG_ENCRYPTED != 0)
}

/// Read the header off `source`, then build the layered decode pipeline
/// (optional decryption → ANS) and hand back a plain [`Read`] over the tar bytes.
///
/// The returned reader borrows for the same lifetime `'a` as `source`, so a
/// progress-counting source (which borrows the shared `Progress`) can flow
/// through without demanding a `'static` bound.
fn open_payload<'a, R: Read + 'a>(
    mut source: R,
    password: Option<&str>,
) -> io::Result<Box<dyn Read + 'a>> {
    let encrypted = read_header(&mut source)?;
    let undecrypted: Box<dyn Read + 'a> = if encrypted {
        let pw = password.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "this archive is sealed — a password is required to open it",
            )
        })?;
        Box::new(crypto::DecryptReader::new(source, pw)?)
    } else {
        Box::new(source)
    };
    Ok(Box::new(ans::AnsReader::new(undecrypted)?))
}

/// Canonicalize an archive-internal path for comparison (forward slashes, no
/// trailing slash) — mirrors the helper in `decompress`.
fn normalize_member(name: &str) -> String {
    name.replace('\\', "/").trim_end_matches('/').to_string()
}
