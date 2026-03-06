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
    /// Execute mailbox sync worker.
    Sync {
        /// Mailbox name to sync (defaults to [source].mailbox or linux-kernel).
        /// Use INBOX to trigger real IMAP sync when [imap] config is complete.
        #[arg(long)]
        mailbox: Option<String>,
        /// Local fixture directory for offline/local test (.eml files).
        #[arg(long, value_name = "DIR")]
        fixture_dir: Option<PathBuf>,
        /// Override UIDVALIDITY when using --fixture-dir.
        #[arg(long, value_name = "N")]
        uidvalidity: Option<u64>,
        /// Maximum reconnect attempts for the sync loop (default 3).
        #[arg(long, value_name = "N")]
        reconnect_attempts: Option<u8>,
    },
    /// Run environment diagnostics.
    Doctor,
    /// Print Courier version.
    Version,
}
