//! Inspect an archive's contents without extracting it.

use crate::abyss;
use crate::decompress::raw_output_name;
use crate::format::{Container, Format};
use std::fs::File;
use std::io;
use std::path::Path;

/// One logical member of an archive.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Path of the entry inside the archive.
    pub name: String,
    /// Uncompressed size in bytes. `None` when it cannot be known without
    /// decompressing (single raw streams).
    pub size: Option<u64>,
    /// Whether the entry is a directory.
    pub is_dir: bool,
}

/// Result of inspecting an archive.
#[derive(Debug, Clone)]
pub struct Listing {
    /// Detected/forced format of the archive.
    pub format: Format,
    /// Members of the archive.
    pub entries: Vec<Entry>,
    /// True for single-stream formats, where there is exactly one (sizeless) member.
    pub single_stream: bool,
}

/// List the contents of `src` interpreted as `format`, without extracting.
///
/// `password` is only consulted for a sealed `.abyss` archive; listing an
/// encrypted archive still requires the password (its table of contents lives
/// behind the encryption layer).
pub fn list(src: &Path, format: Format, password: Option<&str>) -> io::Result<Listing> {
    let entries = match format.container {
        Container::Zip => list_zip(src)?,
        Container::Abyss => abyss::list(src, password)?,
        Container::Tar => list_tar(src, format)?,
        Container::Raw => {
            return Ok(Listing {
                format,
                entries: vec![Entry {
                    name: raw_output_name(src, format.codec)
                        .to_string_lossy()
                        .into_owned(),
                    size: None,
                    is_dir: false,
                }],
                single_stream: true,
            });
        }
    };
    Ok(Listing { format, entries, single_stream: false })
}

fn list_zip(src: &Path) -> io::Result<Vec<Entry>> {
    let file = io::BufReader::new(File::open(src)?);
    let mut archive = zip::ZipArchive::new(file)?;
    let mut entries = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        entries.push(Entry {
            name: entry.name().to_string(),
            size: Some(entry.size()),
            is_dir: entry.is_dir(),
        });
    }
    Ok(entries)
}

fn list_tar(src: &Path, format: Format) -> io::Result<Vec<Entry>> {
    let file = File::open(src)?;
    let mut entries = Vec::new();
    format.codec.decompress(file, |reader| {
        let mut archive = tar::Archive::new(reader);
        for entry in archive.entries()? {
            let entry = entry?;
            let header = entry.header();
            let is_dir = header.entry_type().is_dir();
            let size = header.size().unwrap_or(0);
            let name = entry.path()?.to_string_lossy().into_owned();
            entries.push(Entry { name, size: Some(size), is_dir });
        }
        Ok(())
    })?;
    Ok(entries)
}
