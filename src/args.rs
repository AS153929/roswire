use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "roswire",
    version,
    about = "JSON-first RouterOS CLI bridge for AI agents and automation.",
    long_about = None
)]
pub struct Cli {
    /// Output machine-readable JSON.
    #[arg(long)]
    pub json: bool,

    /// Enable debug diagnostics on stderr.
    #[arg(long)]
    pub debug: bool,

    /// Raw command tokens passed after global options.
    #[arg(value_name = "TOKEN")]
    pub tokens: Vec<String>,
}
