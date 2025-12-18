use clap::{Parser, Subcommand};
use log::error;
use std::path::PathBuf;

use dzip_cli::compression::create_default_registry;
use dzip_cli::{pack, unpack};

#[derive(Parser)]
#[command(
    name = "dzip_cli",
    author = "Ed1th",
    version,
    about = "Marmalade SDK .dz Archive Tool",
    long_about = "A CLI tool to unpack and pack Marmalade SDK .dz archives."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Unpack a .dz archive
    Unpack {
        /// Input .dz file
        input: PathBuf,
        /// Output directory (optional)
        #[arg(short, long)]
        outdir: Option<PathBuf>,
        /// Keep raw compressed data for unsupported chunks (e.g. CHUNK_DZ)
        #[arg(long)]
        keep_raw: bool,
    },
    /// Pack a directory based on a TOML config
    Pack {
        /// Input config.toml file
        config: PathBuf,
    },
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Cli::parse();

    // Initialize the codec registry
    let registry = create_default_registry();

    let res = match args.command {
        Commands::Unpack {
            input,
            outdir,
            keep_raw,
        } => unpack::do_unpack(&input, outdir, keep_raw, &registry),
        Commands::Pack { config } => pack::do_pack(&config, &registry),
    };

    if let Err(e) = res {
        // Print the full error chain using {:#}
        error!("{:#}", e);
        std::process::exit(1);
    }
}
