use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "og", about = "OpenGraphs CLI (local-first experiment tracking)")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the local daemon (placeholder bootstrap path).
    Serve {
        /// Bind address for ogd.
        #[arg(long, default_value = ogd::DEFAULT_BIND_ADDR)]
        bind: String,
    },
    /// Print version information.
    Version,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Command::Version) {
        Command::Serve { bind } => ogd::start(&bind).await?,
        Command::Version => println!("og {}", env!("CARGO_PKG_VERSION")),
    }

    Ok(())
}
