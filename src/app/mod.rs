pub mod cli;
pub mod sync;

use clap::Parser;

use crate::infra::b4::{self, B4Status};
use crate::infra::bootstrap;
use crate::infra::config;
use crate::infra::error::Result;
use crate::infra::logging;
use crate::infra::ui_state;

const DEFAULT_SYNC_RECONNECT_ATTEMPTS: u8 = 3;

pub fn run() -> Result<()> {
    let cli = cli::Cli::parse();
    let command = cli.command.unwrap_or(cli::Command::Tui);

    if matches!(command, cli::Command::Version) {
        println!("courier {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let runtime = config::load(cli.config.as_deref())?;
    logging::init(&runtime.log_filter, &runtime.log_dir)?;

    let bootstrap_state = bootstrap::prepare(&runtime)?;

    match command {
        cli::Command::Tui => {
            let ui_state_path = ui_state::path_for_data_dir(&runtime.data_dir);
            let startup_mailboxes = load_startup_mailboxes(&ui_state_path);

            for mailbox in startup_mailboxes {
                let startup_sync_request = sync::SyncRequest {
                    mailbox: mailbox.clone(),
                    fixture_dir: None,
                    uidvalidity: None,
                    reconnect_attempts: DEFAULT_SYNC_RECONNECT_ATTEMPTS,
                };

                if let Err(error) =
                    run_sync_command(&runtime, &bootstrap_state, startup_sync_request, false)
                {
                    tracing::warn!(
                        mailbox = %mailbox,
                        error = %error,
                        "startup sync failed, continuing with local cache"
                    );
                    eprintln!("warning: startup sync failed for {mailbox}: {error}");
                }
            }

            crate::ui::run(&runtime, &bootstrap_state)
        }
        cli::Command::Sync {
            mailbox,
            fixture_dir,
            uidvalidity,
            reconnect_attempts,
        } => {
            let request = sync::SyncRequest {
                mailbox: mailbox.unwrap_or_else(|| runtime.imap_mailbox.clone()),
                fixture_dir,
                uidvalidity,
                reconnect_attempts: reconnect_attempts.unwrap_or(DEFAULT_SYNC_RECONNECT_ATTEMPTS),
            };
            run_sync_command(&runtime, &bootstrap_state, request, true)?;
            Ok(())
        }
        cli::Command::Doctor => {
            let b4_status = b4::check(runtime.b4_path.as_deref());

            println!("courier doctor");
            println!("  config_path: {}", runtime.config_path.display());
            println!("  data_dir: {}", runtime.data_dir.display());
            println!("  database_path: {}", runtime.database_path.display());
            println!("  raw_mail_dir: {}", runtime.raw_mail_dir.display());
            println!("  patch_dir: {}", runtime.patch_dir.display());
            println!("  log_dir: {}", runtime.log_dir.display());
            println!("  imap_mailbox: {}", runtime.imap_mailbox);
            println!("  lore_base_url: {}", runtime.lore_base_url);
            if runtime.kernel_trees.is_empty() {
                println!("  kernel_trees: <none>");
            } else {
                for tree in &runtime.kernel_trees {
                    println!("  kernel_tree: {}", tree.display());
                }
            }
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

fn load_startup_mailboxes(ui_state_path: &std::path::Path) -> Vec<String> {
    match ui_state::load(ui_state_path) {
        Ok(Some(state)) => state.normalized_enabled_mailboxes(),
        Ok(None) => Vec::new(),
        Err(error) => {
            tracing::warn!(
                path = %ui_state_path.display(),
                error = %error,
                "failed to load ui state for startup sync"
            );
            Vec::new()
        }
    }
}

fn run_sync_command(
    runtime: &config::RuntimeConfig,
    bootstrap_state: &bootstrap::BootstrapState,
    request: sync::SyncRequest,
    print_summary: bool,
) -> Result<sync::SyncSummary> {
    let summary = sync::run(runtime, request)?;

    tracing::info!(
        database = %bootstrap_state.db.path.display(),
        schema_version = bootstrap_state.db.schema_version,
        mailbox = %summary.mailbox,
        fetched = summary.fetched,
        inserted = summary.inserted,
        updated = summary.updated,
        rebuilt_roots = summary.rebuilt_roots,
        "sync command finished"
    );

    if print_summary {
        println!(
            "sync complete: mailbox={} source={} fetched={} inserted={} updated={} rebuilt_roots={} uidvalidity={} last_seen_uid={} highest_modseq={:?} synced_at={} mailbox_rebuilt={}",
            summary.mailbox,
            summary.source,
            summary.fetched,
            summary.inserted,
            summary.updated,
            summary.rebuilt_roots,
            summary.uidvalidity,
            summary.checkpoint_last_seen_uid,
            summary.checkpoint_highest_modseq,
            summary
                .checkpoint_synced_at
                .as_deref()
                .unwrap_or("<unknown>"),
            summary.mailbox_rebuilt
        );
    }

    Ok(summary)
}
