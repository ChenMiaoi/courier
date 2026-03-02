use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "courier", about = "Courier MVP CLI", version)]
pub struct Cli {
    /// Override config file path.
    #[arg(long, global = true, value_name = "FILE")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Start Courier TUI.
    Tui,
    /// Initialize runtime and execute sync bootstrap.
    Sync,
    /// Run environment diagnostics.
    Doctor,
    /// Print Courier version.
    Version,
}
