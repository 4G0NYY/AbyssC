use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "abyssc", author = "4G0NYY", version = "0.1.0", about = "A performance-optimized modular compression engine, straight from the depths of the Abyss.", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compress a single file into a .zip archive
    Zip {
        #[arg(short, long)]
        src: PathBuf,
        #[arg(short, long)]
        dest: PathBuf,
    },
    /// Unzip an archive into a target directory
    Unzip {
        #[arg(short, long)]
        src: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Zip { src, dest } => {
            println!("Compressing {:?} to {:?}...", src, dest);
            if let Err(e) = archive_engine::compress_file(&src, &dest) {
                eprintln!("Error compressing file: {}", e);
            } else {
                println!("Success!");
            }
        }
        Commands::Unzip { src, out } => {
            println!("Extracting {:?} to {:?}...", src, out);
            if let Err(e) = archive_engine::decompress_archive(&src, &out) {
                eprintln!("Error extracting archive: {}", e);
            } else {
                println!("Success!");
            }
        }
    }
}