use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use chrono::DateTime;
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::infra::error::{CourierError, ErrorCode, Result};

const LORE_BASE_URL: &str = "https://lore.kernel.org";
const LORE_HTTP_TIMEOUT_SECS: u64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ImapErrorKind {
    Connection,
    Authentication,
    MailboxSelection,
    Protocol,
}

#[derive(Debug, Clone)]
pub struct MailboxSnapshot {
    pub uidvalidity: u64,
    pub highest_uid: u32,
    pub highest_modseq: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct RemoteMail {
    pub uid: u32,
    pub modseq: Option<u64>,
    pub flags: Vec<String>,
    pub raw: Vec<u8>,
}

pub trait ImapClient {
    fn connect(&mut self) -> Result<()>;
    fn select_mailbox(&mut self, mailbox: &str) -> Result<MailboxSnapshot>;
    fn fetch_incremental(
        &mut self,
        mailbox: &str,
        after_uid: u32,
        since_modseq: Option<u64>,
    ) -> Result<Vec<RemoteMail>>;
}

#[derive(Debug, Clone)]
pub struct FixtureImapClient {
    root_dir: PathBuf,
    default_uidvalidity: u64,
    connected: bool,
}

impl FixtureImapClient {
    pub fn new(root_dir: PathBuf, default_uidvalidity: u64) -> Self {
        Self {
            root_dir,
            default_uidvalidity,
            connected: false,
        }
    }

    fn mailbox_dir(&self, mailbox: &str) -> PathBuf {
        let mailbox_candidate = self.root_dir.join(mailbox);
        if mailbox_candidate.is_dir() {
            mailbox_candidate
        } else {
            self.root_dir.clone()
        }
    }

    fn ensure_connected(&self) -> Result<()> {
        if self.connected {
            return Ok(());
        }

        Err(imap_error(
            ImapErrorKind::Connection,
            "client is not connected",
        ))
    }

    fn read_uidvalidity(&self, mailbox: &str) -> Result<u64> {
        let path = self.mailbox_dir(mailbox).join(".uidvalidity");
        if !path.exists() {
            return Ok(self.default_uidvalidity);
        }

        let content = fs::read_to_string(&path).map_err(|error| {
            CourierError::with_source(
                ErrorCode::Imap,
                format!("failed to read UIDVALIDITY from {}", path.display()),
                error,
            )
        })?;

        let value = content.trim();
        if value.is_empty() {
            return Ok(self.default_uidvalidity);
        }

        value.parse::<u64>().map_err(|error| {
            CourierError::with_source(
                ErrorCode::Imap,
                format!(
                    "invalid UIDVALIDITY value '{}' in {}",
                    value,
                    path.display()
                ),
                error,
            )
        })
    }

    fn scan_entries(&self, mailbox: &str) -> Result<Vec<FixtureEntry>> {
        let dir = self.mailbox_dir(mailbox);
        if !dir.exists() {
            return Err(imap_error(
                ImapErrorKind::MailboxSelection,
                format!("mailbox directory {} not found", dir.display()),
            ));
        }

        let mut files: Vec<PathBuf> = fs::read_dir(&dir)
            .map_err(|error| {
                CourierError::with_source(
                    ErrorCode::Imap,
                    format!("failed to read mailbox directory {}", dir.display()),
                    error,
                )
            })?
            .filter_map(|entry| entry.ok().map(|item| item.path()))
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("eml"))
            })
            .collect();

        files.sort_by(|left, right| {
            left.file_name()
                .and_then(|name| name.to_str())
                .cmp(&right.file_name().and_then(|name| name.to_str()))
        });

        let mut used_uids = HashSet::new();
        let mut entries = Vec::with_capacity(files.len());

        for (index, path) in files.into_iter().enumerate() {
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();

            let mut uid = leading_uid(&file_name).unwrap_or(index as u32 + 1);
            while !used_uids.insert(uid) {
                uid = uid.saturating_add(1);
            }

            let metadata = fs::metadata(&path).map_err(|error| {
                CourierError::with_source(
                    ErrorCode::Imap,
                    format!("failed to read metadata for {}", path.display()),
                    error,
                )
            })?;

            let modseq = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs());

            entries.push(FixtureEntry { path, uid, modseq });
        }

        entries.sort_by_key(|entry| entry.uid);
        Ok(entries)
    }
}

impl ImapClient for FixtureImapClient {
    fn connect(&mut self) -> Result<()> {
        if !self.root_dir.exists() {
            return Err(imap_error(
                ImapErrorKind::Connection,
                format!("fixture root {} does not exist", self.root_dir.display()),
            ));
        }

        if !self.root_dir.is_dir() {
            return Err(imap_error(
                ImapErrorKind::Connection,
                format!(
                    "fixture root {} is not a directory",
                    self.root_dir.display()
                ),
            ));
        }

        self.connected = true;
        Ok(())
    }

    fn select_mailbox(&mut self, mailbox: &str) -> Result<MailboxSnapshot> {
        self.ensure_connected()?;

        let uidvalidity = self.read_uidvalidity(mailbox)?;
        let entries = self.scan_entries(mailbox)?;

        let highest_uid = entries.iter().map(|entry| entry.uid).max().unwrap_or(0);
        let highest_modseq = entries.iter().filter_map(|entry| entry.modseq).max();

        Ok(MailboxSnapshot {
            uidvalidity,
            highest_uid,
            highest_modseq,
        })
    }

    fn fetch_incremental(
        &mut self,
        mailbox: &str,
        after_uid: u32,
        since_modseq: Option<u64>,
    ) -> Result<Vec<RemoteMail>> {
        self.ensure_connected()?;

        let entries = self.scan_entries(mailbox)?;
        let mut fetched = Vec::new();

        for entry in entries {
            let fetch_by_uid = entry.uid > after_uid;
            let fetch_by_modseq = since_modseq
                .zip(entry.modseq)
                .is_some_and(|(checkpoint, current)| current > checkpoint);

            if !fetch_by_uid && !fetch_by_modseq {
                continue;
            }

            let raw = fs::read(&entry.path).map_err(|error| {
                CourierError::with_source(
                    ErrorCode::Imap,
                    format!("failed to read fixture mail {}", entry.path.display()),
                    error,
                )
            })?;

            let flags = parse_flags(&raw);
            fetched.push(RemoteMail {
                uid: entry.uid,
                modseq: entry.modseq,
                flags,
                raw,
            });
        }

        fetched.sort_by_key(|mail| mail.uid);
        Ok(fetched)
    }
}

#[derive(Debug, Clone)]
pub struct LoreImapClient {
    base_url: String,
    connected: bool,
    client: reqwest::blocking::Client,
}

#[derive(Debug, Clone)]
struct LoreFeedEntry {
    message_url: String,
    modseq: u64,
}

impl LoreImapClient {
    pub fn new(base_url: Option<&str>) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(LORE_HTTP_TIMEOUT_SECS))
            .user_agent(format!("courier/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| {
                CourierError::with_source(
                    ErrorCode::Imap,
                    "failed to initialize lore HTTP client",
                    error,
                )
            })?;

        Ok(Self {
            base_url: base_url
                .unwrap_or(LORE_BASE_URL)
                .trim_end_matches('/')
                .to_string(),
            connected: false,
            client,
        })
    }

    fn ensure_connected(&self) -> Result<()> {
        if self.connected {
            return Ok(());
        }

        Err(imap_error(
            ImapErrorKind::Connection,
            "client is not connected",
        ))
    }

    fn feed_url(&self, mailbox: &str) -> String {
        let mailbox = mailbox.trim_matches('/');
        format!("{}/{}/new.atom", self.base_url, mailbox)
    }

    fn fetch_feed_entries(&self, mailbox: &str) -> Result<Vec<LoreFeedEntry>> {
        let url = self.feed_url(mailbox);
        let response = self.client.get(&url).send().map_err(|error| {
            CourierError::with_source(
                ErrorCode::Imap,
                format!("failed to fetch lore feed {url}"),
                error,
            )
        })?;

        let status = response.status();
        let body = response.text().map_err(|error| {
            CourierError::with_source(
                ErrorCode::Imap,
                format!("failed to read lore feed body {url}"),
                error,
            )
        })?;

        if !status.is_success() {
            return Err(imap_error(
                ImapErrorKind::MailboxSelection,
                format!("failed to fetch lore feed {url}: HTTP {status}"),
            ));
        }

        parse_lore_atom_entries(&body)
    }

    fn fetch_raw_mail(&self, message_url: &str) -> Result<Vec<u8>> {
        let mut last_error: Option<CourierError> = None;

        for raw_url in lore_raw_url_candidates(message_url) {
            let response = match self.client.get(&raw_url).send() {
                Ok(response) => response,
                Err(error) => {
                    last_error = Some(CourierError::with_source(
                        ErrorCode::Imap,
                        format!("failed to fetch lore raw message {raw_url}"),
                        error,
                    ));
                    continue;
                }
            };

            if !response.status().is_success() {
                last_error = Some(imap_error(
                    ImapErrorKind::Protocol,
                    format!(
                        "failed to fetch lore raw message {raw_url}: HTTP {}",
                        response.status()
                    ),
                ));
                continue;
            }

            let bytes = response.bytes().map_err(|error| {
                CourierError::with_source(
                    ErrorCode::Imap,
                    format!("failed to read lore raw message body {raw_url}"),
                    error,
                )
            })?;

            if !bytes.is_empty() {
                return Ok(bytes.to_vec());
            }

            last_error = Some(imap_error(
                ImapErrorKind::Protocol,
                format!("lore raw message is empty: {raw_url}"),
            ));
        }

        Err(last_error.unwrap_or_else(|| {
            imap_error(
                ImapErrorKind::Protocol,
                format!("failed to resolve raw message URL for {message_url}"),
            )
        }))
    }
}

impl ImapClient for LoreImapClient {
    fn connect(&mut self) -> Result<()> {
        self.connected = true;
        Ok(())
    }

    fn select_mailbox(&mut self, mailbox: &str) -> Result<MailboxSnapshot> {
        self.ensure_connected()?;
        let entries = self.fetch_feed_entries(mailbox)?;
        let highest_modseq = entries.iter().map(|entry| entry.modseq).max();

        Ok(MailboxSnapshot {
            uidvalidity: 1,
            highest_uid: 0,
            highest_modseq,
        })
    }

    fn fetch_incremental(
        &mut self,
        mailbox: &str,
        _after_uid: u32,
        since_modseq: Option<u64>,
    ) -> Result<Vec<RemoteMail>> {
        self.ensure_connected()?;
        let entries = self.fetch_feed_entries(mailbox)?;

        let mut fetched = Vec::new();
        for entry in entries {
            if since_modseq.is_some_and(|checkpoint| entry.modseq <= checkpoint) {
                continue;
            }

            let raw = self.fetch_raw_mail(&entry.message_url)?;
            fetched.push(RemoteMail {
                uid: 0,
                modseq: Some(entry.modseq),
                flags: Vec::new(),
                raw,
            });
        }

        fetched.sort_by_key(|mail| mail.modseq.unwrap_or(0));
        Ok(fetched)
    }
}

#[derive(Debug, Clone)]
struct FixtureEntry {
    path: PathBuf,
    uid: u32,
    modseq: Option<u64>,
}

fn parse_flags(raw: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(raw);
    let mut current_header = String::new();
    let mut current_value = String::new();

    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }

        if line.starts_with(' ') || line.starts_with('\t') {
            if current_header.eq_ignore_ascii_case("x-flags") {
                if !current_value.is_empty() {
                    current_value.push(' ');
                }
                current_value.push_str(line.trim());
            }
            continue;
        }

        if current_header.eq_ignore_ascii_case("x-flags") {
            break;
        }

        if let Some((name, value)) = line.split_once(':') {
            current_header = name.trim().to_string();
            current_value = value.trim().to_string();
        }
    }

    if current_header.eq_ignore_ascii_case("x-flags") {
        return split_flags(&current_value);
    }

    Vec::new()
}

fn split_flags(raw: &str) -> Vec<String> {
    raw.split(|ch: char| ch.is_whitespace() || ch == ',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn leading_uid(file_name: &str) -> Option<u32> {
    let prefix: String = file_name
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();

    if prefix.is_empty() {
        None
    } else {
        prefix.parse::<u32>().ok()
    }
}

fn parse_lore_atom_entries(xml: &str) -> Result<Vec<LoreFeedEntry>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut buffer = Vec::new();
    let mut entries = Vec::new();
    let mut seen_urls = HashSet::new();

    let mut in_entry = false;
    let mut current_tag: Option<Vec<u8>> = None;
    let mut current_link: Option<String> = None;
    let mut current_id: Option<String> = None;
    let mut current_modseq: Option<u64> = None;

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => {
                let tag = event.name().as_ref().to_vec();
                if tag == b"entry" {
                    in_entry = true;
                    current_tag = None;
                    current_link = None;
                    current_id = None;
                    current_modseq = None;
                } else if in_entry && tag == b"link" {
                    if let Some(link) = parse_link_href(&event) {
                        current_link = Some(link);
                    }
                    current_tag = None;
                } else if in_entry {
                    current_tag = Some(tag);
                }
            }
            Ok(Event::Empty(event)) => {
                if in_entry
                    && event.name().as_ref() == b"link"
                    && let Some(link) = parse_link_href(&event)
                {
                    current_link = Some(link);
                }
            }
            Ok(Event::Text(event)) => {
                if !in_entry {
                    buffer.clear();
                    continue;
                }

                let text = event.unescape().map_err(|error| {
                    CourierError::with_source(
                        ErrorCode::Imap,
                        "failed to decode lore atom text",
                        error,
                    )
                })?;

                match current_tag.as_deref() {
                    Some(b"updated") | Some(b"published") => {
                        if let Some(modseq) = parse_atom_timestamp(text.as_ref()) {
                            current_modseq =
                                Some(current_modseq.map_or(modseq, |prev| prev.max(modseq)));
                        }
                    }
                    Some(b"id") => {
                        if current_id.is_none() {
                            current_id = Some(text.into_owned());
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(event)) => {
                let tag = event.name().as_ref().to_vec();
                if tag == b"entry" {
                    let message_url = current_link.take().or_else(|| current_id.take());
                    if let (Some(message_url), Some(modseq)) = (message_url, current_modseq)
                        && let Some(normalized) = normalize_lore_message_url(&message_url)
                        && seen_urls.insert(normalized.clone())
                    {
                        entries.push(LoreFeedEntry {
                            message_url: normalized,
                            modseq,
                        });
                    }

                    in_entry = false;
                    current_tag = None;
                    current_modseq = None;
                } else if in_entry {
                    current_tag = None;
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(CourierError::with_source(
                    ErrorCode::Imap,
                    "failed to parse lore atom feed",
                    error,
                ));
            }
            _ => {}
        }

        buffer.clear();
    }

    entries.sort_by(|left, right| {
        left.modseq
            .cmp(&right.modseq)
            .then_with(|| left.message_url.cmp(&right.message_url))
    });

    Ok(entries)
}

fn parse_link_href(event: &quick_xml::events::BytesStart<'_>) -> Option<String> {
    let mut rel: Option<String> = None;
    let mut href: Option<String> = None;

    for attr in event.attributes().flatten() {
        if attr.key.as_ref() == b"rel" {
            rel = Some(String::from_utf8_lossy(attr.value.as_ref()).to_string());
        }
        if attr.key.as_ref() == b"href" {
            href = Some(String::from_utf8_lossy(attr.value.as_ref()).to_string());
        }
    }

    let relation = rel.unwrap_or_else(|| "alternate".to_string());
    if relation == "alternate" || relation == "self" {
        href
    } else {
        None
    }
}

fn parse_atom_timestamp(value: &str) -> Option<u64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .and_then(|datetime| datetime.timestamp().try_into().ok())
}

fn normalize_lore_message_url(value: &str) -> Option<String> {
    if !value.contains("//") {
        return None;
    }

    let without_fragment = value.split('#').next().unwrap_or(value);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    let trimmed = without_query.trim_end_matches('/');
    if trimmed.is_empty() {
        None
    } else {
        Some(format!("{trimmed}/"))
    }
}

fn lore_raw_url_candidates(message_url: &str) -> Vec<String> {
    let normalized =
        normalize_lore_message_url(message_url).unwrap_or_else(|| message_url.to_string());
    let base = normalized.trim_end_matches('/');

    let candidates = vec![
        format!("{base}/raw"),
        format!("{base}/raw/"),
        format!("{normalized}raw"),
    ];

    let mut uniq = Vec::new();
    let mut seen = HashSet::new();
    for candidate in candidates {
        if seen.insert(candidate.clone()) {
            uniq.push(candidate);
        }
    }

    uniq
}

fn imap_error(kind: ImapErrorKind, message: impl Into<String>) -> CourierError {
    CourierError::new(
        ErrorCode::Imap,
        format!("{}: {}", classify(kind), message.into()),
    )
}

fn classify(kind: ImapErrorKind) -> &'static str {
    match kind {
        ImapErrorKind::Connection => "connection",
        ImapErrorKind::Authentication => "authentication",
        ImapErrorKind::MailboxSelection => "mailbox",
        ImapErrorKind::Protocol => "protocol",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{
        FixtureImapClient, ImapClient, lore_raw_url_candidates, parse_atom_timestamp,
        parse_lore_atom_entries,
    };

    fn temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("courier-imap-{label}-{nonce}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn fixture_client_fetches_incremental_messages() {
        let root = temp_dir("incremental");
        fs::write(
            root.join("1-root.eml"),
            "Message-ID: <a@example.com>\nSubject: root\n\nbody\n",
        )
        .expect("write root");
        thread::sleep(Duration::from_millis(5));
        fs::write(
            root.join("2-reply.eml"),
            "Message-ID: <b@example.com>\nSubject: re\n\nbody\n",
        )
        .expect("write reply");

        let mut client = FixtureImapClient::new(root.clone(), 42);
        client.connect().expect("connect");

        let selected = client.select_mailbox("inbox").expect("select mailbox");
        assert_eq!(selected.uidvalidity, 42);
        assert_eq!(selected.highest_uid, 2);

        let first_batch = client
            .fetch_incremental("inbox", 0, None)
            .expect("fetch first batch");
        assert_eq!(first_batch.len(), 2);

        let second_batch = client
            .fetch_incremental("inbox", 2, selected.highest_modseq)
            .expect("fetch second batch");
        assert!(second_batch.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reads_uidvalidity_from_marker_file() {
        let root = temp_dir("uidvalidity");
        fs::write(root.join(".uidvalidity"), "9001\n").expect("write marker");
        fs::write(
            root.join("1.eml"),
            "Message-ID: <x@example.com>\nSubject: x\n\nbody\n",
        )
        .expect("write message");

        let mut client = FixtureImapClient::new(root.clone(), 1);
        client.connect().expect("connect");
        let snapshot = client.select_mailbox("inbox").expect("select");
        assert_eq!(snapshot.uidvalidity, 9001);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parses_lore_atom_entries() {
        let xml = r#"
<feed xmlns=\"http://www.w3.org/2005/Atom\">
  <entry>
    <id>https://lore.kernel.org/io-uring/msg-a/</id>
    <updated>2026-03-03T09:00:00+00:00</updated>
    <link rel=\"alternate\" href=\"https://lore.kernel.org/io-uring/msg-a/\" />
  </entry>
  <entry>
    <id>https://lore.kernel.org/io-uring/msg-b/</id>
    <published>2026-03-03T10:00:00+00:00</published>
    <link rel=\"alternate\" href=\"https://lore.kernel.org/io-uring/msg-b/\" />
  </entry>
</feed>
"#;

        let entries = parse_lore_atom_entries(xml).expect("parse atom");
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].message_url,
            "https://lore.kernel.org/io-uring/msg-a/"
        );
        assert_eq!(
            entries[1].message_url,
            "https://lore.kernel.org/io-uring/msg-b/"
        );
        assert!(entries[1].modseq > entries[0].modseq);
    }

    #[test]
    fn builds_lore_raw_candidates() {
        let candidates = lore_raw_url_candidates("https://lore.kernel.org/io-uring/abc123/");
        assert!(candidates.iter().any(|url| url.ends_with("/raw")));
    }

    #[test]
    fn parses_atom_timestamps() {
        let ts = parse_atom_timestamp("2026-03-03T10:00:00+00:00").expect("timestamp");
        assert!(ts > 0);
    }
}
