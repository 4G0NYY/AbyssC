use archive_engine::{CodecOptions, Container, Format, Report};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

#[derive(Parser)]
#[command(
    name = "abyssc",
    author = "4G0NYY",
    version,
    about = "A performance-optimized modular compression engine, straight from the depths of the Abyss.",
    long_about = None,
    // We ship our own themed `help` subcommand instead of clap's default.
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compress files/directories. Format is auto-detected from the output extension.
    ///
    /// Examples:
    ///   abyssc compress -o backup.tar.zst  src/ notes.txt
    ///   abyssc compress -o data.zst -l 19   data.bin
    #[command(visible_alias = "c")]
    Compress {
        /// Output archive. Its extension selects the format: .zst .tar.zst .zip
        /// .gz .tar.gz .xz .lz4 .bz2 .br .tar ...
        #[arg(short, long)]
        output: PathBuf,
        /// Compression level (codec-specific; higher = smaller but slower).
        #[arg(short, long)]
        level: Option<i32>,
        /// Worker threads for codecs that support it (zstd). 0 = all cores.
        #[arg(short, long, default_value_t = 0)]
        threads: u32,
        /// Force a format instead of detecting it from the output extension.
        #[arg(short, long, value_name = "NAME")]
        format: Option<String>,
        /// Seal the archive with a password (`.abyss` only). Encrypts and
        /// authenticates the contents; the same password is needed to extract.
        #[arg(short, long, value_name = "PASSWORD")]
        password: Option<String>,
        /// Files and/or directories to compress.
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
    },
    /// Extract an archive. Format is auto-detected from the input extension.
    #[command(visible_alias = "x")]
    Extract {
        /// Archive to extract.
        #[arg(short, long)]
        input: PathBuf,
        /// Output directory (created if missing).
        #[arg(short, long, default_value = ".")]
        output: PathBuf,
        /// Force a format instead of detecting it from the input extension.
        #[arg(short, long, value_name = "NAME")]
        format: Option<String>,
        /// Password to unseal an encrypted `.abyss` archive.
        #[arg(short, long, value_name = "PASSWORD")]
        password: Option<String>,
    },
    /// List an archive's contents without extracting it.
    #[command(visible_alias = "l")]
    List {
        /// Archive to inspect.
        #[arg(short, long)]
        input: PathBuf,
        /// Force a format instead of detecting it from the input extension.
        #[arg(short, long, value_name = "NAME")]
        format: Option<String>,
        /// Password to read a sealed `.abyss` archive's contents.
        #[arg(short, long, value_name = "PASSWORD")]
        password: Option<String>,
    },
    /// Show the Abyssal field guide: formats, levels, and incantations.
    Help,
    /// Show the version, rendered from the depths.
    #[command(visible_alias = "v")]
    Version,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Compress { output, level, threads, format, password, inputs } => {
            run_compress(&inputs, &output, level, threads, format.as_deref(), password)
        }
        Commands::Extract { input, output, format, password } => {
            run_extract(&input, &output, format.as_deref(), password.as_deref())
        }
        Commands::List { input, format, password } => {
            run_list(&input, format.as_deref(), password.as_deref())
        }
        Commands::Help => {
            print_guide();
            Ok(())
        }
        Commands::Version => {
            print_banner();
            Ok(())
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run_compress(
    inputs: &[PathBuf],
    output: &Path,
    level: Option<i32>,
    threads: u32,
    format_override: Option<&str>,
    password: Option<String>,
) -> Result<(), String> {
    let format = resolve_format(format_override, output, "output")?;

    // Encryption is an `.abyss` feature; refuse to silently drop a password the
    // user clearly meant to apply.
    if password.as_deref().is_some_and(|p| !p.is_empty())
        && format.container != Container::Abyss
    {
        return Err(format!(
            "--password only applies to .abyss archives, not '{}'; \
             use a .abyss output (or -f abyss)",
            format.label()
        ));
    }
    let opts = CodecOptions::new(level, threads).with_password(password);

    println!(
        "Compressing {} item(s) -> {} [{}]",
        inputs.len(),
        output.display(),
        format.label()
    );

    let start = Instant::now();
    let report = archive_engine::compress(inputs, output, format, &opts)
        .map_err(|e| format!("compression failed: {e}"))?;
    let elapsed = start.elapsed().as_secs_f64();

    print_report(&report, elapsed);
    Ok(())
}

fn run_extract(
    input: &Path,
    output: &Path,
    format_override: Option<&str>,
    password: Option<&str>,
) -> Result<(), String> {
    let format = resolve_format(format_override, input, "input")?;

    println!(
        "Extracting {} -> {} [{}]",
        input.display(),
        output.display(),
        format.label()
    );

    let start = Instant::now();
    archive_engine::decompress(input, output, format, password)
        .map_err(|e| format!("extraction failed: {e}"))?;
    let elapsed = start.elapsed().as_secs_f64();

    println!("Done in {:.2}s.", elapsed);
    Ok(())
}

fn run_list(
    input: &Path,
    format_override: Option<&str>,
    password: Option<&str>,
) -> Result<(), String> {
    let format = resolve_format(format_override, input, "input")?;
    let listing = archive_engine::list(input, format, password)
        .map_err(|e| format!("could not read archive: {e}"))?;

    println!("Archive: {} [{}]", input.display(), listing.format.label());

    if listing.single_stream {
        let name = listing
            .entries
            .first()
            .map(|e| e.name.as_str())
            .unwrap_or("?");
        let on_disk = std::fs::metadata(input).map(|m| m.len()).unwrap_or(0);
        println!(
            "  single {} stream -> {}  ({} compressed on disk)",
            listing.format.codec.name(),
            name,
            fmt_bytes(on_disk)
        );
        return Ok(());
    }

    println!("{:>14}  NAME", "SIZE");
    let mut total = 0u64;
    let mut files = 0u64;
    let mut dirs = 0u64;
    for entry in &listing.entries {
        if entry.is_dir {
            dirs += 1;
            println!("{:>14}  {}/", "<dir>", entry.name.trim_end_matches('/'));
        } else {
            let size = entry.size.unwrap_or(0);
            total += size;
            files += 1;
            println!("{:>14}  {}", fmt_bytes(size), entry.name);
        }
    }
    println!(
        "  {files} file(s), {dirs} dir(s), {} uncompressed",
        fmt_bytes(total)
    );
    Ok(())
}

/// Resolve a format from an explicit `--format` name, else from the path's
/// extension. `role` is "input"/"output" for a helpful error message.
fn resolve_format(
    name: Option<&str>,
    path: &Path,
    role: &str,
) -> Result<Format, String> {
    match name {
        Some(name) => Format::from_name(name)
            .ok_or_else(|| format!("unknown format '{name}'")),
        None => Format::from_path(path).ok_or_else(|| {
            format!(
                "could not detect a format from the {role} extension of '{}'; \
                 pass --format to choose one",
                path.display()
            )
        }),
    }
}

fn print_report(report: &Report, elapsed: f64) {
    let ratio = report.ratio();
    let saved = if report.uncompressed == 0 {
        0.0
    } else {
        (1.0 - ratio) * 100.0
    };
    // Avoid dividing by zero on instant runs.
    let throughput = if elapsed > 0.0 {
        report.uncompressed as f64 / elapsed
    } else {
        report.uncompressed as f64
    };

    println!(
        "Done: {} -> {} ({:.1}% saved, ratio {:.3})",
        fmt_bytes(report.uncompressed),
        fmt_bytes(report.compressed),
        saved,
        ratio
    );
    println!("Time: {:.2}s  ({}/s)", elapsed, fmt_bytes(throughput as u64));
}

/// The themed `version` output: an abyssal banner. Version is pulled from the
/// crate metadata at compile time, so it always matches `[workspace.package]`.
fn print_banner() {
    // Figlet "Standard" rendering of "AbyssC".
    const ART: &str = r#"
      _    _                    ____
     / \  | |__  _   _ ___ ___  / ___|
    / _ \ | '_ \| | | / __/ __|| |
   / ___ \| |_) | |_| \__ \__ \| |___
  /_/   \_\_.__/ \__, |___/___/ \____|
                 |___/
"#;
    println!("{ART}");
    println!("  AbyssC v{}  —  compression from the depths", env!("CARGO_PKG_VERSION"));
    println!("  codecs: zstd · lz4 · gzip · xz · bzip2 · brotli · ans · store");
    println!("  sealed: .abyss  —  our own ANS, optionally encrypted");
    println!();
    println!("  \"It is only natural that those without power have no voice.\"");
}

/// The themed `help` output: a blunt, efficient field guide.
fn print_guide() {
    println!(
        r#"
   A B Y S S C  ::  compression from the depths

   The surface clings to its files. I compress them. What took a
   continent to hold, I fold into a glowing orb. Choose your power.

 INCANTATIONS
   abyssc compress  -o <archive> [opts] <inputs...>   (alias: c)
   abyssc extract   -i <archive> [-o <dir>] [-p pw]   (alias: x)
   abyssc list      -i <archive> [-p pw]              (alias: l)
   abyssc help                                         this guide
   abyssc version                                      the banner (alias: v)
   abyssc <command> --help                             clap's detail

 OPTIONS
   -o, --output     destination; its extension decides the format
   -i, --input      source archive
   -l, --level      effort. higher = smaller, slower. codec-bound.
   -t, --threads    workers (zstd). 0 = every core you have.
   -f, --format     override extension detection (e.g. -f tar.zst)
   -p, --password   seal/unseal a .abyss archive (encrypted + authenticated)

 FORMATS  (extension -> codec)
   .abyss           ans      our own sigil. bundles, ANS-codes, can be sealed.
   .ans .tar.ans    ans      the raw entropy coder, unsealed.
   .zst .tar.zst    zstd     balance of speed and ratio. multithreaded.
   .lz4 .tar.lz4    lz4      raw velocity. the fastest blade.
   .gz  .tar.gz     gzip     the old, ubiquitous standard.
   .xz  .tar.xz     xz/lzma  patient. crushes hardest, moves slowest.
   .bz2 .tar.bz2    bzip2    legacy weight.
   .br  .tar.br     brotli   the web's chosen ratio.
   .zip             zip      portable. deflate per entry.
   .tar             store    bundle only. no compression.

   single-stream (.zst, .gz, .ans, ...) take ONE file.
   tar.*, .zip, and .abyss swallow whole directories.

 READ-ONLY  (the surface's forms — list & extract, never create)
   .jar .war .ear .apk   ZIP under the skin.
   .7z                   7-Zip. lzma/lzma2/bzip2/ppmd, AES-256.
   .rar                  RAR. creation is proprietary.
   .iso                  ISO-9660 disc images.

   abyssc l -i game.iso              walk a disc image's contents
   abyssc x -i bundle.7z -o ./out    unfold a .7z (compress refuses it)

 EFFICIENT PATHS
   abyssc c -o backup.tar.zst project/        bundle a tree, fast
   abyssc c -o data.lz4 data.bin              maximum throughput
   abyssc c -o data.zst -l 19 -t 8 data.bin   maximum compression
   abyssc c -o vault.abyss -p hunter2 secrets/  seal it from the surface
   abyssc l -i backup.tar.zst                 look without touching
   abyssc x -i vault.abyss -p hunter2 -o ./out  unseal and unfold

   "It is only natural that those without power have no voice."
"#
    );
}

fn fmt_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}
