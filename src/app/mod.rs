pub mod cli;

use clap::Parser;

use crate::infra::b4::{self, B4Status};
use crate::infra::bootstrap;
use crate::infra::config;
use crate::infra::error::Result;
use crate::infra::logging;

pub fn run() -> Result<()> {
    let cli = cli::Cli::parse();
    let command = cli.command.unwrap_or(cli::Command::Tui);

    if matches!(command, cli::Command::Version) {
        println!("courier {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let runtime = config::load(cli.config.as_deref())?;
    logging::init(&runtime.log_filter)?;

    let bootstrap_state = bootstrap::prepare(&runtime)?;

    match command {
        cli::Command::Tui => crate::ui::run(&runtime, &bootstrap_state),
        cli::Command::Sync => {
            tracing::info!(
                database = %bootstrap_state.db.path.display(),
                schema_version = bootstrap_state.db.schema_version,
                "sync command initialized"
            );
            println!(
                "sync bootstrap complete: database={} schema_version={} created={}",
                bootstrap_state.db.path.display(),
                bootstrap_state.db.schema_version,
                bootstrap_state.db.created
            );
            Ok(())
        }
        cli::Command::Doctor => {
            let b4_status = b4::check(runtime.b4_path.as_deref());

            println!("courier doctor");
            println!("  config_path: {}", runtime.config_path.display());
            println!("  data_dir: {}", runtime.data_dir.display());
            println!("  database_path: {}", runtime.database_path.display());
            println!("  schema_version: {}", bootstrap_state.db.schema_version);
            println!(
                "  schema_version_expected: {}",
                crate::infra::db::CURRENT_SCHEMA_VERSION
            );
            println!(
                "  migrations_applied: {:?}",
                bootstrap_state.db.applied_migrations
            );
            println!(
                "  database_created_this_run: {}",
                bootstrap_state.db.created
            );

            match b4_status.status {
                B4Status::Available { path, version } => {
                    println!("  b4_path: {}", path.display());
                    println!("  b4_version: {}", version);
                    println!("  b4_status: ok");
                }
                B4Status::Broken { path, reason } => {
                    println!("  b4_path: {}", path.display());
                    println!("  b4_status: broken ({reason})");
                }
                B4Status::Missing => {
                    println!("  b4_path: <not found>");
                    println!("  b4_status: missing");
                }
            }

            Ok(())
        }
        cli::Command::Version => Ok(()),
    }
}
