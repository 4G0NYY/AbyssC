use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

pub fn compress_file(src: &Path, dest: &Path) -> io::Result<()> {
    let file = File::create(dest)?;
    let mut zip = ZipWriter::new(file);

    // Configure the ZIP options to use standard DEFLATE compression
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);

    let file_name = src
        .file_name()
        .and_then(|os_str| os_str.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid file name"))?;

    // Begin a new file inside the zip archive
    zip.start_file(file_name, options)?;

    let mut src_file = File::open(src)?;
    let mut buffer = Vec::new();
    src_file.read_to_end(&mut buffer)?;

    // Write the raw bytes into the zip stream
    zip.write_all(&buffer)?;
    zip.finish()?;

    Ok(())
}