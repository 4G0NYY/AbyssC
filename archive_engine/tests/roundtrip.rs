//! End-to-end round-trip tests: compress then extract must reproduce the input
//! byte-for-byte, for every supported format.

use archive_engine::{CodecOptions, Format};
use std::fs;
use std::path::PathBuf;

/// Build a payload that has both compressible runs and incompressible noise so
/// the codecs actually exercise their machinery.
fn sample_payload() -> Vec<u8> {
    let mut data = Vec::new();
    for i in 0..50_000u32 {
        data.extend_from_slice(b"The Abyss gazes also. ");
        data.extend_from_slice(&i.to_le_bytes());
    }
    data
}

fn unique_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "abyssc_test_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Round-trip a single-file (raw stream) format.
fn check_raw(format_name: &str, archive_ext: &str) {
    let dir = unique_dir(format_name);
    let payload = sample_payload();

    let input = dir.join("input.bin");
    fs::write(&input, &payload).unwrap();

    let archive = dir.join(format!("input.bin{archive_ext}"));
    let format = Format::from_path(&archive).expect("extension should map to a format");

    let report = archive_engine::compress(
        &[input.clone()],
        &archive,
        format,
        &CodecOptions::default(),
    )
    .unwrap_or_else(|e| panic!("compress {format_name} failed: {e}"));
    assert_eq!(report.uncompressed, payload.len() as u64);

    let out_dir = dir.join("out");
    archive_engine::decompress(&archive, &out_dir, format)
        .unwrap_or_else(|e| panic!("extract {format_name} failed: {e}"));

    let restored = fs::read(out_dir.join("input.bin")).unwrap();
    assert_eq!(restored, payload, "{format_name}: round-trip mismatch");

    fs::remove_dir_all(&dir).ok();
}

/// Round-trip a multi-file container format (tar.* or zip) over a directory tree.
fn check_container(format_name: &str, archive_name: &str) {
    let dir = unique_dir(format_name);
    let payload = sample_payload();

    // tree/a.txt and tree/sub/b.bin
    let tree = dir.join("tree");
    fs::create_dir_all(tree.join("sub")).unwrap();
    fs::write(tree.join("a.txt"), b"hello from a").unwrap();
    fs::write(tree.join("sub/b.bin"), &payload).unwrap();

    let archive = dir.join(archive_name);
    let format = Format::from_path(&archive).expect("extension should map to a format");

    archive_engine::compress(&[tree.clone()], &archive, format, &CodecOptions::default())
        .unwrap_or_else(|e| panic!("compress {format_name} failed: {e}"));

    let out_dir = dir.join("out");
    archive_engine::decompress(&archive, &out_dir, format)
        .unwrap_or_else(|e| panic!("extract {format_name} failed: {e}"));

    let a = fs::read(out_dir.join("tree/a.txt")).unwrap();
    let b = fs::read(out_dir.join("tree/sub/b.bin")).unwrap();
    assert_eq!(a, b"hello from a", "{format_name}: a.txt mismatch");
    assert_eq!(b, payload, "{format_name}: b.bin mismatch");

    fs::remove_dir_all(&dir).ok();
}

/// Pull one nested member out of a container without unpacking the rest.
fn check_extract_member(format_name: &str, archive_name: &str) {
    let dir = unique_dir(format_name);
    let payload = sample_payload();

    let tree = dir.join("tree");
    fs::create_dir_all(tree.join("sub")).unwrap();
    fs::write(tree.join("a.txt"), b"hello from a").unwrap();
    fs::write(tree.join("sub/b.bin"), &payload).unwrap();

    let archive = dir.join(archive_name);
    let format = Format::from_path(&archive).expect("extension should map to a format");
    archive_engine::compress(&[tree.clone()], &archive, format, &CodecOptions::default())
        .unwrap_or_else(|e| panic!("compress {format_name} failed: {e}"));

    // A deeply-nested member, drawn out on its own.
    let one = dir.join("just_b.bin");
    archive_engine::extract_member(&archive, format, "tree/sub/b.bin", &one)
        .unwrap_or_else(|e| panic!("extract_member {format_name} failed: {e}"));
    assert_eq!(fs::read(&one).unwrap(), payload, "{format_name}: member mismatch");

    // A missing member is an error, not a silent empty file.
    let missing = dir.join("nope");
    assert!(
        archive_engine::extract_member(&archive, format, "tree/ghost", &missing).is_err(),
        "{format_name}: missing member should error"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn extract_member_pulls_one_file() {
    check_extract_member("tar", "tree.tar");
    check_extract_member("tar.zst", "tree.tar.zst");
    check_extract_member("tar.gz", "tree.tar.gz");
    check_extract_member("zip", "tree.zip");
}

#[test]
fn raw_streams_roundtrip() {
    check_raw("gzip", ".gz");
    check_raw("zstd", ".zst");
    check_raw("lz4", ".lz4");
    check_raw("xz", ".xz");
    check_raw("bzip2", ".bz2");
    check_raw("brotli", ".br");
}

#[test]
fn containers_roundtrip() {
    check_container("tar", "tree.tar");
    check_container("tar.gz", "tree.tar.gz");
    check_container("tar.zst", "tree.tar.zst");
    check_container("tar.xz", "tree.tar.xz");
    check_container("tar.bz2", "tree.tar.bz2");
    check_container("tar.lz4", "tree.tar.lz4");
    check_container("tar.br", "tree.tar.br");
    check_container("zip", "tree.zip");
}
