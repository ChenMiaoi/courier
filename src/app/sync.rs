use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use crate::infra::config::RuntimeConfig;
use crate::infra::error::{CourierError, ErrorCode, Result};
use crate::infra::imap::{FixtureImapClient, ImapClient, LoreImapClient, RemoteMail};
use crate::infra::mail_parser::{self, ParsedMailHeaders};
use crate::infra::mail_store::{self, IncomingMail, SyncBatch};

const INITIAL_SYNC_THREAD_LIMIT: usize = 20;

#[derive(Debug)]
struct RemoteMailEnvelope {
    remote: RemoteMail,
    parsed: ParsedMailHeaders,
}

#[derive(Debug, Clone)]
pub struct SyncRequest {
    pub mailbox: String,
    pub fixture_dir: Option<PathBuf>,
    pub uidvalidity: Option<u64>,
    pub reconnect_attempts: u8,
}

#[derive(Debug, Clone)]
pub struct SyncSummary {
    pub mailbox: String,
    pub source: String,
    pub fetched: usize,
    pub inserted: usize,
    pub updated: usize,
    pub rebuilt_roots: usize,
    pub mailbox_rebuilt: bool,
    pub uidvalidity: u64,
    pub checkpoint_last_seen_uid: u32,
    pub checkpoint_highest_modseq: Option<u64>,
    pub checkpoint_synced_at: Option<String>,
}

#[derive(Debug, Clone)]
enum SyncSource {
    Fixture {
        fixture_dir: PathBuf,
        uidvalidity_hint: u64,
    },
    Lore {
        base_url: String,
    },
}

impl SyncSource {
    fn label(&self) -> String {
        match self {
            Self::Fixture { fixture_dir, .. } => fixture_dir.display().to_string(),
            Self::Lore { base_url } => base_url.clone(),
        }
    }
}

pub fn run(config: &RuntimeConfig, request: SyncRequest) -> Result<SyncSummary> {
    let source = resolve_sync_source(config, &request);
    let attempts = request.reconnect_attempts.max(1);
    let mut last_error: Option<CourierError> = None;

    for attempt in 1..=attempts {
        match run_once(config, &request.mailbox, &source) {
            Ok(summary) => return Ok(summary),
            Err(error) => {
                tracing::warn!(
                    attempt,
                    attempts,
                    mailbox = %request.mailbox,
                    source = %source.label(),
                    error = %error,
                    "sync attempt failed"
                );
                last_error = Some(error);
                if attempt < attempts {
                    thread::sleep(Duration::from_millis(200));
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        CourierError::new(
            ErrorCode::Imap,
            format!("sync failed after {} attempts", attempts),
        )
    }))
}

fn resolve_sync_source(config: &RuntimeConfig, request: &SyncRequest) -> SyncSource {
    if let Some(fixture_dir) = request.fixture_dir.as_ref() {
        return SyncSource::Fixture {
            fixture_dir: fixture_dir.clone(),
            uidvalidity_hint: request.uidvalidity.unwrap_or(1),
        };
    }

    SyncSource::Lore {
        base_url: config.lore_base_url.clone(),
    }
}

fn run_once(config: &RuntimeConfig, mailbox: &str, source: &SyncSource) -> Result<SyncSummary> {
    let checkpoint = mail_store::load_mailbox_state(&config.database_path, mailbox)?;
    let checkpoint_last_seen_uid = checkpoint
        .as_ref()
        .map(|state| state.last_seen_uid)
        .unwrap_or(0);
    let mailbox_message_count = mail_store::mailbox_message_count(&config.database_path, mailbox)?;
    let initial_window_sync = mailbox_message_count == 0;

    let mut client: Box<dyn ImapClient> = match source {
        SyncSource::Fixture {
            fixture_dir,
            uidvalidity_hint,
        } => Box::new(FixtureImapClient::new(
            fixture_dir.to_path_buf(),
            *uidvalidity_hint,
        )),
        SyncSource::Lore { base_url } => Box::new(LoreImapClient::new(Some(base_url))?),
    };

    client.connect()?;
    let snapshot = client.select_mailbox(mailbox)?;

    let mailbox_rebuilt = checkpoint
        .as_ref()
        .is_some_and(|state| state.uidvalidity != snapshot.uidvalidity);

    let after_uid = if mailbox_rebuilt {
        0
    } else {
        checkpoint_last_seen_uid
    };

    let since_modseq = if mailbox_rebuilt {
        None
    } else {
        checkpoint.as_ref().and_then(|state| state.highest_modseq)
    };

    let remote_messages = client.fetch_incremental(mailbox, after_uid, since_modseq)?;
    let mut envelopes = parse_remote_messages(mailbox, remote_messages);
    if initial_window_sync {
        envelopes = retain_latest_threads(envelopes, INITIAL_SYNC_THREAD_LIMIT);
    }

    let fetched = envelopes.len();
    let mut incoming = Vec::with_capacity(fetched);
    let mut synthetic_uid = checkpoint_last_seen_uid;

    for envelope in envelopes {
        let mut remote = envelope.remote;
        if remote.uid == 0 {
            synthetic_uid = synthetic_uid.saturating_add(1);
            remote.uid = synthetic_uid;
        }

        let raw_path = persist_raw_mail(config, mailbox, remote.uid, &remote.raw)?;

        incoming.push(IncomingMail {
            mailbox: mailbox.to_string(),
            uid: remote.uid,
            modseq: remote.modseq,
            flags: remote.flags,
            raw_path,
            parsed: envelope.parsed,
        });
    }

    let fetched_highest_uid = incoming
        .iter()
        .map(|mail| mail.uid)
        .max()
        .unwrap_or(checkpoint_last_seen_uid);
    let fetched_highest_modseq = incoming.iter().filter_map(|mail| mail.modseq).max();

    let batch_highest_uid = snapshot
        .highest_uid
        .max(fetched_highest_uid)
        .max(checkpoint_last_seen_uid);

    let batch_highest_modseq = max_option(snapshot.highest_modseq, fetched_highest_modseq);

    let write_result = mail_store::apply_sync_batch(
        &config.database_path,
        SyncBatch {
            mailbox: mailbox.to_string(),
            uidvalidity: snapshot.uidvalidity,
            highest_uid: batch_highest_uid,
            highest_modseq: batch_highest_modseq,
            mails: incoming,
        },
    )?;

    Ok(SyncSummary {
        mailbox: write_result.state.mailbox.clone(),
        source: source.label(),
        fetched,
        inserted: write_result.inserted,
        updated: write_result.updated,
        rebuilt_roots: write_result.rebuilt_roots,
        mailbox_rebuilt: write_result.mailbox_rebuilt,
        uidvalidity: write_result.state.uidvalidity,
        checkpoint_last_seen_uid: write_result.state.last_seen_uid,
        checkpoint_highest_modseq: write_result.state.highest_modseq,
        checkpoint_synced_at: write_result.state.synced_at.clone(),
    })
}

fn parse_remote_messages(
    mailbox: &str,
    remote_messages: Vec<RemoteMail>,
) -> Vec<RemoteMailEnvelope> {
    remote_messages
        .into_iter()
        .enumerate()
        .map(|(index, remote)| {
            let fallback_message_id = if remote.uid == 0 {
                format!("synthetic-{mailbox}-{index}@local")
            } else {
                format!("synthetic-{mailbox}-{}@local", remote.uid)
            };
            let parsed = mail_parser::parse_headers(&remote.raw, fallback_message_id);
            RemoteMailEnvelope { remote, parsed }
        })
        .collect()
}

fn retain_latest_threads(
    messages: Vec<RemoteMailEnvelope>,
    thread_limit: usize,
) -> Vec<RemoteMailEnvelope> {
    if thread_limit == 0 || messages.is_empty() {
        return Vec::new();
    }

    let mut index_by_message_id = HashMap::new();
    for (index, message) in messages.iter().enumerate() {
        index_by_message_id.insert(message.parsed.message_id.clone(), index);
    }

    let root_keys: Vec<String> = (0..messages.len())
        .map(|index| thread_root_key(index, &messages, &index_by_message_id))
        .collect();

    let mut latest_rank_by_thread = HashMap::new();
    for (index, root_key) in root_keys.iter().enumerate() {
        let rank = message_sort_rank(&messages[index]);
        latest_rank_by_thread
            .entry(root_key.clone())
            .and_modify(|existing| {
                if rank > *existing {
                    *existing = rank;
                }
            })
            .or_insert(rank);
    }

    let mut threads: Vec<(String, u64)> = latest_rank_by_thread.into_iter().collect();
    threads.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let selected_roots: HashSet<String> = threads
        .into_iter()
        .take(thread_limit)
        .map(|(root_key, _)| root_key)
        .collect();

    let mut selected: Vec<RemoteMailEnvelope> = messages
        .into_iter()
        .zip(root_keys)
        .filter_map(|(message, root_key)| {
            if selected_roots.contains(&root_key) {
                Some(message)
            } else {
                None
            }
        })
        .collect();

    selected.sort_by_key(message_sort_rank);
    selected
}

fn message_sort_rank(message: &RemoteMailEnvelope) -> u64 {
    let modseq = message.remote.modseq.unwrap_or(0);
    (modseq << 32) | message.remote.uid as u64
}

fn thread_root_key(
    index: usize,
    messages: &[RemoteMailEnvelope],
    index_by_message_id: &HashMap<String, usize>,
) -> String {
    let mut current = index;
    let mut seen = HashSet::new();

    loop {
        if !seen.insert(current) {
            return messages[index].parsed.message_id.clone();
        }

        let Some(parent) = parent_index(current, messages, index_by_message_id) else {
            if let Some(root_hint) = messages[current].parsed.references.first()
                && !root_hint.is_empty()
            {
                return root_hint.clone();
            }
            return messages[current].parsed.message_id.clone();
        };
        current = parent;
    }
}

fn parent_index(
    index: usize,
    messages: &[RemoteMailEnvelope],
    index_by_message_id: &HashMap<String, usize>,
) -> Option<usize> {
    let current_message_id = messages[index].parsed.message_id.as_str();

    if let Some(parent) = messages[index]
        .parsed
        .references
        .iter()
        .rev()
        .find_map(|reference| index_by_message_id.get(reference).copied())
        && messages[parent].parsed.message_id != current_message_id
    {
        return Some(parent);
    }

    messages[index]
        .parsed
        .in_reply_to
        .as_ref()
        .and_then(|reply_to| index_by_message_id.get(reply_to).copied())
        .filter(|parent| messages[*parent].parsed.message_id != current_message_id)
}

fn persist_raw_mail(
    config: &RuntimeConfig,
    mailbox: &str,
    uid: u32,
    raw: &[u8],
) -> Result<PathBuf> {
    let mailbox_dir = config.raw_mail_dir.join(mailbox);
    fs::create_dir_all(&mailbox_dir).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Io,
            format!(
                "failed to create raw mail directory {}",
                mailbox_dir.display()
            ),
            error,
        )
    })?;

    let path = mailbox_dir.join(format!("{:010}.eml", uid));
    fs::write(&path, raw).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Io,
            format!("failed to write raw mail file {}", path.display()),
            error,
        )
    })?;

    Ok(path)
}

fn max_option(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(l), Some(r)) => Some(l.max(r)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::infra::db;
    use crate::infra::mail_store;

    use super::{SyncRequest, run};
    use crate::infra::config::RuntimeConfig;

    fn temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("courier-sync-{label}-{nonce}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn sync_worker_imports_fixture_mails_and_builds_threads() {
        let root = temp_dir("fixture");
        let fixture_dir = root.join("fixture");
        let data_dir = root.join("data");
        let raw_dir = data_dir.join("raw");
        let db_path = data_dir.join("courier.db");
        fs::create_dir_all(&fixture_dir).expect("create fixture dir");
        fs::create_dir_all(&raw_dir).expect("create raw dir");

        fs::write(
            fixture_dir.join("1-root.eml"),
            "Message-ID: <root@example.com>\nSubject: [PATCH 0/2] root\nFrom: alice@example.com\n\nbody\n",
        )
        .expect("write root");
        fs::write(
            fixture_dir.join("2-reply.eml"),
            "Message-ID: <reply@example.com>\nSubject: Re: [PATCH 0/2] root\nFrom: bob@example.com\nIn-Reply-To: <root@example.com>\nReferences: <root@example.com>\n\nbody\n",
        )
        .expect("write reply");

        db::initialize(&db_path).expect("initialize db");

        let runtime = RuntimeConfig {
            config_path: root.join("config.toml"),
            data_dir: data_dir.clone(),
            database_path: db_path.clone(),
            raw_mail_dir: raw_dir,
            patch_dir: data_dir.join("patches"),
            log_dir: data_dir.join("logs"),
            b4_path: None,
            log_filter: "info".to_string(),
            imap_mailbox: "inbox".to_string(),
            lore_base_url: "https://lore.kernel.org".to_string(),
            kernel_trees: Vec::new(),
        };

        let first = run(
            &runtime,
            SyncRequest {
                mailbox: "inbox".to_string(),
                fixture_dir: Some(fixture_dir.clone()),
                uidvalidity: Some(1),
                reconnect_attempts: 1,
            },
        )
        .expect("first sync");
        assert_eq!(first.fetched, 2);
        assert_eq!(first.inserted, 2);
        assert_eq!(first.updated, 0);

        let second = run(
            &runtime,
            SyncRequest {
                mailbox: "inbox".to_string(),
                fixture_dir: Some(fixture_dir),
                uidvalidity: Some(1),
                reconnect_attempts: 1,
            },
        )
        .expect("second sync");
        assert_eq!(second.fetched, 0);
        assert_eq!(second.inserted, 0);
        assert_eq!(second.updated, 0);

        let rows = mail_store::load_thread_rows_by_mailbox(&db_path, "inbox", 20)
            .expect("load thread rows");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.depth == 1));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn initial_empty_mailbox_sync_keeps_latest_twenty_threads() {
        let root = temp_dir("initial-window");
        let fixture_dir = root.join("fixture");
        let data_dir = root.join("data");
        let raw_dir = data_dir.join("raw");
        let db_path = data_dir.join("courier.db");
        fs::create_dir_all(&fixture_dir).expect("create fixture dir");
        fs::create_dir_all(&raw_dir).expect("create raw dir");

        for uid in 1..=25u32 {
            fs::write(
                fixture_dir.join(format!("{uid:04}-thread-{uid}.eml")),
                format!(
                    "Message-ID: <thread-{uid}@example.com>\nSubject: [PATCH] thread {uid}\nFrom: user{uid}@example.com\n\nbody {uid}\n"
                ),
            )
            .expect("write fixture");
        }

        db::initialize(&db_path).expect("initialize db");

        let runtime = RuntimeConfig {
            config_path: root.join("config.toml"),
            data_dir: data_dir.clone(),
            database_path: db_path.clone(),
            raw_mail_dir: raw_dir,
            patch_dir: data_dir.join("patches"),
            log_dir: data_dir.join("logs"),
            b4_path: None,
            log_filter: "info".to_string(),
            imap_mailbox: "inbox".to_string(),
            lore_base_url: "https://lore.kernel.org".to_string(),
            kernel_trees: Vec::new(),
        };

        let summary = run(
            &runtime,
            SyncRequest {
                mailbox: "inbox".to_string(),
                fixture_dir: Some(fixture_dir),
                uidvalidity: Some(1),
                reconnect_attempts: 1,
            },
        )
        .expect("first sync");

        assert_eq!(summary.fetched, 20);
        assert_eq!(summary.inserted, 20);
        assert_eq!(summary.updated, 0);
        assert_eq!(summary.checkpoint_last_seen_uid, 25);

        let rows = mail_store::load_thread_rows_by_mailbox(&db_path, "inbox", 100)
            .expect("load thread rows");
        assert_eq!(rows.len(), 20);
        assert!(
            !rows
                .iter()
                .any(|row| row.message_id == "thread-1@example.com")
        );
        assert!(
            rows.iter()
                .any(|row| row.message_id == "thread-25@example.com")
        );

        let _ = fs::remove_dir_all(root);
    }
}
