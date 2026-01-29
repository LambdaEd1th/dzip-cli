use clap::{Parser, Subcommand};
use dzip_core::Result;
use log::info;

mod commands;
mod config;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Enable verbose logging/output
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Unpack a dzip file
    Unpack {
        /// The dzip file to unpack
        input: String,
        /// The output directory
        #[arg(short, long, default_value = ".")]
        output: String,
    },
    /// Pack a directory into a dzip file
    Pack {
        /// The configuration file to pack (toml)
        input: String,
        /// The output directory
        #[arg(short, long, default_value = ".")]
        output: String,
    },
    /// Verify and list archive contents
    Verify {
        /// Input archive file
        input: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_level = if cli.verbose { "debug" } else { "info" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level)).init();

    match &cli.command {
        Commands::Unpack { input, output } => {
            commands::unpack::unpack_archive(input, output)?;
        }
        Commands::Pack { input, output } => {
            info!("Packing from config {} to output dir {}", input, output);
            commands::pack::pack_archive(input, output)?;
        }
        Commands::Verify { input } => {
            commands::verify::verify_archive(input)?;
        }
    }

    Ok(())
}
