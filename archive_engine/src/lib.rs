pub mod compress;
pub mod decompress;

// Re-export functions for a simpler public API
pub use compress::compress_file;
pub use decompress::decompress_archive;