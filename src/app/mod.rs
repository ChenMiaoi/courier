//! Application-layer entrypoints and orchestration.
//!
//! This module keeps command routing and high-level policy in one place so the
//! lower layers can stay focused on storage, network, and UI primitives.

pub mod cli;
pub mod patch;
pub mod sync;

use clap::Parser;

use crate::infra::b4::{self, B4Status};
use crate::infra::bootstrap;
use crate::infra::config::{self, IMAP_INBOX_MAILBOX};
use crate::infra::error::Result;
use crate::infra::imap::{ImapClient, RemoteImapClient};
use crate::infra::logging;
use crate::infra::sendmail::{self, GitSendEmailStatus};

const DEFAULT_SYNC_RECONNECT_ATTEMPTS: u8 = 3;

pub fn run() -> Result<()> {
    let cli = cli::Cli::parse();
    let command = cli.command.unwrap_or(cli::Command::Tui);

    if matches!(command, cli::Command::Version) {
        println!("criew {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let mut runtime = config::load(cli.config.as_deref())?;
    logging::init(&runtime.log_filter, &runtime.log_dir)?;
    tracing::debug!(command = ?command, "user invoked cli command");

    let startup_config_path = runtime.config_path.clone();
    let mut bootstrap_state = bootstrap::prepare(&runtime)?;

    match command {
        cli::Command::Tui => loop {
            match crate::ui::run(&runtime, &bootstrap_state)? {
                crate::ui::TuiAction::Exit => break Ok(()),
                crate::ui::TuiAction::Restart => {
                    tracing::info!(
                        config_path = %startup_config_path.display(),
                        "user requested tui restart"
                    );
                    runtime = config::load(Some(startup_config_path.as_path()))?;
                    bootstrap_state = bootstrap::prepare(&runtime)?;
                }
            }
        },
        cli::Command::Sync {
            mailbox,
            fixture_dir,
            uidvalidity,
            reconnect_attempts,
        } => {
            let request = sync::SyncRequest {
                mailbox: mailbox.unwrap_or_else(|| runtime.source_mailbox.clone()),
                fixture_dir,
                uidvalidity,
                reconnect_attempts: reconnect_attempts.unwrap_or(DEFAULT_SYNC_RECONNECT_ATTEMPTS),
            };
            run_sync_command(&runtime, &bootstrap_state, request, true)?;
            Ok(())
        }
        cli::Command::Doctor => {
            let b4_status = b4::check(runtime.b4_path.as_deref(), Some(&runtime.data_dir));
            let send_email_status = sendmail::check();
            let reply_identity = sendmail::resolve_reply_identity();

            println!("criew doctor");
            println!("  config_path: {}", runtime.config_path.display());
            println!("  data_dir: {}", runtime.data_dir.display());
            println!("  database_path: {}", runtime.database_path.display());
            println!("  raw_mail_dir: {}", runtime.raw_mail_dir.display());
            println!("  patch_dir: {}", runtime.patch_dir.display());
            println!("  log_dir: {}", runtime.log_dir.display());
            println!("  source_mailbox: {}", runtime.source_mailbox);
            println!(
                "  default_active_mailbox: {}",
                runtime.default_active_mailbox()
            );
            println!("  lore_base_url: {}", runtime.lore_base_url);
            println!("  startup_sync: {}", runtime.startup_sync);
            println!("  ui_keymap: {}", runtime.ui_keymap.as_str());
            println!(
                "  inbox_auto_sync_interval_secs: {}",
                runtime.inbox_auto_sync_interval_secs
            );
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
            let self_email = config::resolve_self_email(&runtime);
            println!(
                "  self_email: {}",
                self_email.email.as_deref().unwrap_or("<missing>")
            );
            println!(
                "  self_email_source: {}",
                self_email
                    .source
                    .map(|source| source.as_str())
                    .unwrap_or("<none>")
            );
            if let Some(error) = self_email.git_error.as_deref() {
                println!("  self_email_lookup: error ({error})");
            }
            println!(
                "  imap_config_status: {}",
                if runtime.imap.is_complete() {
                    "complete"
                } else {
                    "incomplete"
                }
            );
            if runtime.imap.is_complete() {
                println!(
                    "  imap_connection_target: {}:{} ({})",
                    runtime.imap.server.as_deref().unwrap_or("<missing>"),
                    runtime.imap.server_port.unwrap_or_default(),
                    runtime
                        .imap
                        .encryption
                        .map(|value| value.as_str())
                        .unwrap_or("<missing>")
                );

                match RemoteImapClient::new(runtime.imap.clone()).and_then(|mut client| {
                    client.connect()?;
                    client.select_mailbox(IMAP_INBOX_MAILBOX)
                }) {
                    Ok(snapshot) => {
                        println!("  imap_connect_status: ok");
                        println!("  imap_select_mailbox: {}", IMAP_INBOX_MAILBOX);
                        println!("  imap_uidvalidity: {}", snapshot.uidvalidity);
                        println!("  imap_highest_uid: {}", snapshot.highest_uid);
                        println!(
                            "  imap_highest_modseq: {}",
                            snapshot
                                .highest_modseq
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "<none>".to_string())
                        );
                    }
                    Err(error) => {
                        println!("  imap_connect_status: error ({error})");
                    }
                }
            } else {
                let missing = runtime.imap.missing_required_fields();
                println!(
                    "  imap_missing_fields: {}",
                    if missing.is_empty() {
                        "<none>".to_string()
                    } else {
                        missing.join(", ")
                    }
                );
                println!("  imap_connect_status: skipped");
            }

            match send_email_status.status {
                GitSendEmailStatus::Available { path, version } => {
                    println!("  git_send_email_path: {}", path.display());
                    println!("  git_send_email_version: {}", version);
                    println!("  git_send_email_status: ok");
                }
                GitSendEmailStatus::Broken { path, reason } => {
                    println!("  git_send_email_path: {}", path.display());
                    println!("  git_send_email_status: broken ({reason})");
                }
                GitSendEmailStatus::Missing => {
                    println!("  git_send_email_path: <not found>");
                    println!("  git_send_email_status: missing");
                }
            }

            match reply_identity {
                Ok(identity) => {
                    println!("  reply_from: {}", identity.display);
                    println!("  reply_from_email: {}", identity.email);
                    println!("  reply_from_source: {}", identity.source.as_str());
                    println!("  reply_identity_status: ok");
                }
                Err(error) => {
                    println!("  reply_from: <missing>");
                    println!("  reply_from_email: <missing>");
                    println!("  reply_from_source: <none>");
                    println!("  reply_identity_status: error ({error})");
                }
            }

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
