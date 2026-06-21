use std::fs::{self, File};
use std::io::{self, copy};
use std::path::Path;
use zip::ZipArchive;

pub fn decompress_archive(src: &Path, out_dir: &Path) -> io::Result<()> {
    let file = File::open(src)?;
    let mut archive = ZipArchive::new(file)?;

    if !out_dir.exists() {
        fs::create_dir_all(out_dir)?;
    }

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = match file.enclosed_name() {
            Some(path) => out_dir.join(path),
            None => continue, // Skip insecure/malformed paths
        };

        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    fs::create_dir_all(p)?;
                }
            }
            let mut outfile = File::create(&outpath)?;
            copy(&mut file, &mut outfile)?;
        }
    }

    Ok(())
}