//! Mail source adapters behind a common sync trait.
//!
//! Fixture data, lore.kernel.org, and real IMAP all implement the same
//! high-level contract so sync orchestration can share checkpoint logic and
//! storage invariants regardless of where mail bytes came from.

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use chrono::{DateTime, NaiveDateTime, Utc};
use quick_xml::Reader;
use quick_xml::events::Event;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use webpki_roots::TLS_SERVER_ROOTS;

use crate::infra::config::{ImapConfig, ImapEncryption};
use crate::infra::error::{CourierError, ErrorCode, Result};

const LORE_BASE_URL: &str = "https://lore.kernel.org";
const GNU_ARCHIVE_MBOX_BASE_URL: &str = "https://lists.gnu.org/archive/mbox";
const LORE_HTTP_TIMEOUT_SECS: u64 = 20;
const REMOTE_IMAP_TIMEOUT_SECS: u64 = 20;
const HTTP_PROXY_RESPONSE_MAX_BYTES: usize = 8 * 1024;
const IMAP_FETCH_BATCH_SIZE: usize = 100;
const GNU_ARCHIVE_INITIAL_MONTH_LIMIT: usize = 2;
const GNU_ARCHIVE_UID_STRIDE: u32 = 1_000_000;

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

    fn fetch_header_candidates(
        &mut self,
        mailbox: &str,
        after_uid: u32,
        since_modseq: Option<u64>,
    ) -> Result<Vec<RemoteMail>> {
        self.fetch_incremental(mailbox, after_uid, since_modseq)
    }

    fn fetch_full_uids(&mut self, mailbox: &str, uids: &[u32]) -> Result<Vec<RemoteMail>> {
        let wanted: HashSet<u32> = uids.iter().copied().collect();
        let mut mails = self.fetch_incremental(mailbox, 0, None)?;
        mails.retain(|mail| wanted.contains(&mail.uid));
        Ok(mails)
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct GnuArchiveMonthEntry {
    month_key: String,
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
pub struct GnuArchiveClient {
    base_url: String,
    connected: bool,
    client: reqwest::blocking::Client,
}

impl GnuArchiveClient {
    pub fn new(base_url: Option<&str>) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(LORE_HTTP_TIMEOUT_SECS))
            .user_agent(format!("courier/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| {
                CourierError::with_source(
                    ErrorCode::Imap,
                    "failed to initialize GNU archive HTTP client",
                    error,
                )
            })?;

        Ok(Self {
            base_url: base_url
                .unwrap_or(GNU_ARCHIVE_MBOX_BASE_URL)
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

    fn index_url(&self, mailbox: &str) -> String {
        let mailbox = mailbox.trim_matches('/');
        format!("{}/{mailbox}/", self.base_url)
    }

    fn month_url(&self, mailbox: &str, month_key: &str) -> String {
        let mailbox = mailbox.trim_matches('/');
        format!("{}/{mailbox}/{month_key}", self.base_url)
    }

    fn fetch_month_entries(&self, mailbox: &str) -> Result<Vec<GnuArchiveMonthEntry>> {
        let url = self.index_url(mailbox);
        let response = self.client.get(&url).send().map_err(|error| {
            CourierError::with_source(
                ErrorCode::Imap,
                format!("failed to fetch GNU archive index {url}"),
                error,
            )
        })?;

        let status = response.status();
        let body = response.text().map_err(|error| {
            CourierError::with_source(
                ErrorCode::Imap,
                format!("failed to read GNU archive index body {url}"),
                error,
            )
        })?;

        if !status.is_success() {
            return Err(imap_error(
                ImapErrorKind::MailboxSelection,
                format!("failed to fetch GNU archive index {url}: HTTP {status}"),
            ));
        }

        parse_gnu_archive_month_entries(&body)
    }

    fn fetch_month_mbox(&self, mailbox: &str, month_key: &str) -> Result<Vec<u8>> {
        let url = self.month_url(mailbox, month_key);
        let response = self.client.get(&url).send().map_err(|error| {
            CourierError::with_source(
                ErrorCode::Imap,
                format!("failed to fetch GNU archive mbox {url}"),
                error,
            )
        })?;

        let status = response.status();
        if !status.is_success() {
            return Err(imap_error(
                ImapErrorKind::Protocol,
                format!("failed to fetch GNU archive mbox {url}: HTTP {status}"),
            ));
        }

        response
            .bytes()
            .map(|bytes| bytes.to_vec())
            .map_err(|error| {
                CourierError::with_source(
                    ErrorCode::Imap,
                    format!("failed to read GNU archive mbox body {url}"),
                    error,
                )
            })
    }
}

impl ImapClient for GnuArchiveClient {
    fn connect(&mut self) -> Result<()> {
        self.connected = true;
        Ok(())
    }

    fn select_mailbox(&mut self, mailbox: &str) -> Result<MailboxSnapshot> {
        self.ensure_connected()?;
        let entries = self.fetch_month_entries(mailbox)?;
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
        let months = self.fetch_month_entries(mailbox)?;
        let selected_months = select_gnu_archive_months(&months, since_modseq);

        let mut fetched = Vec::new();
        for month in selected_months {
            let raw_mbox = self.fetch_month_mbox(mailbox, &month.month_key)?;
            for (index, raw) in parse_gnu_archive_mbox_messages(&raw_mbox)
                .into_iter()
                .enumerate()
            {
                fetched.push(RemoteMail {
                    uid: gnu_archive_message_uid(&month.month_key, index),
                    modseq: Some(month.modseq),
                    flags: Vec::new(),
                    raw,
                });
            }
        }

        fetched.sort_by(|left, right| {
            left.modseq
                .cmp(&right.modseq)
                .then_with(|| left.uid.cmp(&right.uid))
        });
        Ok(fetched)
    }
}

pub struct RemoteImapClient {
    config: ImapConfig,
    session: Option<ImapSession>,
}

impl RemoteImapClient {
    pub fn new(config: ImapConfig) -> Result<Self> {
        let missing = config.missing_required_fields();
        if !missing.is_empty() {
            return Err(imap_error(
                ImapErrorKind::Connection,
                format!("incomplete IMAP config: missing {}", missing.join(", ")),
            ));
        }

        Ok(Self {
            config,
            session: None,
        })
    }

    fn session_mut(&mut self) -> Result<&mut ImapSession> {
        self.session.as_mut().ok_or_else(|| {
            imap_error(
                ImapErrorKind::Connection,
                "remote IMAP session is not connected",
            )
        })
    }
}

impl ImapClient for RemoteImapClient {
    fn connect(&mut self) -> Result<()> {
        let session = ImapSession::connect(&self.config)?;
        self.session = Some(session);
        Ok(())
    }

    fn select_mailbox(&mut self, mailbox: &str) -> Result<MailboxSnapshot> {
        self.session_mut()?.select_mailbox(mailbox)
    }

    fn fetch_incremental(
        &mut self,
        mailbox: &str,
        after_uid: u32,
        since_modseq: Option<u64>,
    ) -> Result<Vec<RemoteMail>> {
        let session = self.session_mut()?;
        let snapshot = session.select_mailbox(mailbox)?;
        let uids = collect_incremental_uids(session, snapshot, after_uid, since_modseq)?;
        session.fetch_uids(&uids, "BODY.PEEK[]")
    }

    fn fetch_header_candidates(
        &mut self,
        mailbox: &str,
        after_uid: u32,
        since_modseq: Option<u64>,
    ) -> Result<Vec<RemoteMail>> {
        let session = self.session_mut()?;
        let snapshot = session.select_mailbox(mailbox)?;
        let uids = collect_incremental_uids(session, snapshot, after_uid, since_modseq)?;
        session.fetch_uids(
            &uids,
            "BODY.PEEK[HEADER.FIELDS (MESSAGE-ID SUBJECT FROM DATE IN-REPLY-TO REFERENCES LIST-ID)]",
        )
    }

    fn fetch_full_uids(&mut self, mailbox: &str, uids: &[u32]) -> Result<Vec<RemoteMail>> {
        let session = self.session_mut()?;
        let _ = session.select_mailbox(mailbox)?;
        session.fetch_uids(uids, "BODY.PEEK[]")
    }
}

fn collect_incremental_uids(
    session: &mut ImapSession,
    snapshot: MailboxSnapshot,
    after_uid: u32,
    since_modseq: Option<u64>,
) -> Result<Vec<u32>> {
    let mut uids = BTreeSet::new();

    if snapshot.highest_uid > after_uid {
        for uid in session.search_uid_range(after_uid.saturating_add(1))? {
            uids.insert(uid);
        }
    }

    if let Some(modseq) = since_modseq
        && snapshot.highest_modseq.is_some()
    {
        for uid in session.search_modseq(modseq)? {
            uids.insert(uid);
        }
    }

    Ok(uids.into_iter().collect())
}

enum ImapTransport {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImapProxyScheme {
    Http,
    Socks5,
}

impl ImapProxyScheme {
    fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Socks5 => "socks5",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImapProxy {
    scheme: ImapProxyScheme,
    host: String,
    port: u16,
}

impl ImapProxy {
    fn redacted_url(&self) -> String {
        format!("{}://{}:{}", self.scheme.as_str(), self.host, self.port)
    }
}

impl Read for ImapTransport {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buf),
            Self::Tls(stream) => stream.read(buf),
        }
    }
}

impl Write for ImapTransport {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buf),
            Self::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

struct ImapSession {
    transport: ImapTransport,
    read_buffer: Vec<u8>,
    next_tag: u32,
    capabilities: HashSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GreetingKind {
    Ok,
    Preauth,
}

impl ImapSession {
    fn connect(config: &ImapConfig) -> Result<Self> {
        let server = config.server.as_deref().ok_or_else(|| {
            imap_error(
                ImapErrorKind::Connection,
                "missing imap.server in runtime config",
            )
        })?;
        let port = config.server_port.ok_or_else(|| {
            imap_error(
                ImapErrorKind::Connection,
                "missing imap.serverport in runtime config",
            )
        })?;
        let encryption = config.encryption.ok_or_else(|| {
            imap_error(
                ImapErrorKind::Connection,
                "missing imap.encryption in runtime config",
            )
        })?;

        let tcp_stream = connect_tcp(config, server, port)?;
        let transport = match encryption {
            ImapEncryption::Tls => ImapTransport::Tls(Box::new(connect_tls(server, tcp_stream)?)),
            ImapEncryption::Starttls | ImapEncryption::None => ImapTransport::Plain(tcp_stream),
        };

        let mut session = Self {
            transport,
            read_buffer: Vec::new(),
            next_tag: 1,
            capabilities: HashSet::new(),
        };

        let greeting = session.read_greeting()?;
        if matches!(encryption, ImapEncryption::Starttls) {
            session.command_ok("STARTTLS", ImapErrorKind::Connection)?;
            session.transport = match session.transport {
                ImapTransport::Plain(stream) => {
                    ImapTransport::Tls(Box::new(connect_tls(server, stream)?))
                }
                ImapTransport::Tls(_) => {
                    return Err(imap_error(
                        ImapErrorKind::Connection,
                        "STARTTLS attempted on TLS transport",
                    ));
                }
            };
            session.read_buffer.clear();
        }

        session.capabilities = session.fetch_capabilities()?;
        if !matches!(greeting, GreetingKind::Preauth) {
            session.login(config)?;
        }

        Ok(session)
    }

    fn read_greeting(&mut self) -> Result<GreetingKind> {
        let line = self.read_line_string()?;
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("* PREAUTH") {
            Ok(GreetingKind::Preauth)
        } else if upper.starts_with("* OK") {
            Ok(GreetingKind::Ok)
        } else if upper.starts_with("* BYE") {
            Err(imap_error(
                ImapErrorKind::Connection,
                format!("server closed connection during greeting: {line}"),
            ))
        } else {
            Err(imap_error(
                ImapErrorKind::Connection,
                format!("unexpected IMAP greeting: {line}"),
            ))
        }
    }

    fn login(&mut self, config: &ImapConfig) -> Result<()> {
        let user = config.login_user().ok_or_else(|| {
            imap_error(
                ImapErrorKind::Authentication,
                "missing imap.user in runtime config",
            )
        })?;
        let pass = config.pass.as_deref().ok_or_else(|| {
            imap_error(
                ImapErrorKind::Authentication,
                "missing imap.pass in runtime config",
            )
        })?;

        if config.encryption == Some(ImapEncryption::None)
            && self.capabilities.contains("LOGINDISABLED")
        {
            return Err(imap_error(
                ImapErrorKind::Authentication,
                "server disallows LOGIN over plaintext connections",
            ));
        }

        self.command_ok(
            &format!(
                "LOGIN {} {}",
                quote_imap_string(user),
                quote_imap_string(pass)
            ),
            ImapErrorKind::Authentication,
        )
    }

    fn fetch_capabilities(&mut self) -> Result<HashSet<String>> {
        let lines = self.command_ok_lines("CAPABILITY", ImapErrorKind::Protocol)?;
        let mut capabilities = HashSet::new();

        for line in lines {
            let upper = line.to_ascii_uppercase();
            if !upper.starts_with("* CAPABILITY ") {
                continue;
            }

            for token in line.split_whitespace().skip(2) {
                capabilities.insert(token.trim().to_ascii_uppercase());
            }
        }

        Ok(capabilities)
    }

    fn select_mailbox(&mut self, mailbox: &str) -> Result<MailboxSnapshot> {
        let lines = self.command_ok_lines(
            &format!("SELECT {}", quote_imap_string(mailbox)),
            ImapErrorKind::MailboxSelection,
        )?;

        let mut uidvalidity = None;
        let mut uidnext = None;
        let mut highest_modseq = None;

        for line in lines {
            if let Some(value) = parse_status_code_u64(&line, "UIDVALIDITY") {
                uidvalidity = Some(value);
            }
            if let Some(value) = parse_status_code_u64(&line, "UIDNEXT") {
                uidnext = Some(value as u32);
            }
            if let Some(value) = parse_status_code_u64(&line, "HIGHESTMODSEQ") {
                highest_modseq = Some(value);
            }
        }

        Ok(MailboxSnapshot {
            uidvalidity: uidvalidity.unwrap_or(1),
            highest_uid: uidnext.unwrap_or(1).saturating_sub(1),
            highest_modseq,
        })
    }

    fn search_uid_range(&mut self, first_uid: u32) -> Result<Vec<u32>> {
        self.search_uids(&format!("UID SEARCH UID {first_uid}:*"))
    }

    fn search_modseq(&mut self, modseq: u64) -> Result<Vec<u32>> {
        self.search_uids(&format!("UID SEARCH MODSEQ {modseq}"))
    }

    fn search_uids(&mut self, command: &str) -> Result<Vec<u32>> {
        let lines = self.command_ok_lines(command, ImapErrorKind::Protocol)?;
        let mut uids = BTreeSet::new();

        for line in lines {
            let upper = line.to_ascii_uppercase();
            if !upper.starts_with("* SEARCH") {
                continue;
            }
            for token in line.split_whitespace().skip(2) {
                if let Ok(uid) = token.parse::<u32>() {
                    uids.insert(uid);
                }
            }
        }

        Ok(uids.into_iter().collect())
    }

    fn fetch_uids(&mut self, uids: &[u32], body_peek: &str) -> Result<Vec<RemoteMail>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }

        let mut fetched = Vec::new();
        for chunk in uids.chunks(IMAP_FETCH_BATCH_SIZE) {
            fetched.extend(self.fetch_uid_chunk(chunk, body_peek)?);
        }
        fetched.sort_by_key(|mail| mail.uid);
        Ok(fetched)
    }

    fn fetch_uid_chunk(&mut self, uids: &[u32], body_peek: &str) -> Result<Vec<RemoteMail>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }

        let tag = self.next_tag();
        let uid_set = format_uid_sequence_set(uids);
        self.write_command(
            &tag,
            &format!("UID FETCH {uid_set} (UID FLAGS MODSEQ {body_peek})"),
        )?;

        let mut fetched = Vec::new();
        loop {
            let line = self.read_line_string()?;
            if line.starts_with('*') {
                if !line.to_ascii_uppercase().contains(" FETCH (") {
                    continue;
                }

                let fetched_uid = parse_fetch_uid(&line).ok_or_else(|| {
                    imap_error(
                        ImapErrorKind::Protocol,
                        format!("missing UID in FETCH response: {line}"),
                    )
                })?;
                let flags = parse_fetch_flags(&line);
                let modseq = parse_fetch_modseq(&line);
                let literal_len = parse_literal_len(&line).ok_or_else(|| {
                    imap_error(
                        ImapErrorKind::Protocol,
                        format!("missing literal size in FETCH response: {line}"),
                    )
                })?;
                let raw = self.read_exact_bytes(literal_len)?;
                self.consume_fetch_trailer()?;
                fetched.push(RemoteMail {
                    uid: fetched_uid,
                    modseq,
                    flags,
                    raw,
                });
                continue;
            }

            if line.starts_with(&tag) {
                ensure_tagged_ok(&line, ImapErrorKind::Protocol)?;
                return Ok(fetched);
            }
        }
    }

    fn consume_fetch_trailer(&mut self) -> Result<()> {
        loop {
            let line = self.read_line_string()?;
            if line.trim().is_empty() {
                continue;
            }
            if line.trim_end().ends_with(')') {
                return Ok(());
            }
            if line.starts_with('A') {
                return Err(imap_error(
                    ImapErrorKind::Protocol,
                    format!("truncated FETCH trailer before tagged completion: {line}"),
                ));
            }
        }
    }

    fn command_ok(&mut self, command: &str, kind: ImapErrorKind) -> Result<()> {
        self.command_ok_lines(command, kind).map(|_| ())
    }

    fn command_ok_lines(&mut self, command: &str, kind: ImapErrorKind) -> Result<Vec<String>> {
        let tag = self.next_tag();
        self.write_command(&tag, command)?;

        let mut lines = Vec::new();
        loop {
            let line = self.read_line_string()?;
            if line.starts_with(&tag) {
                ensure_tagged_ok(&line, kind)?;
                return Ok(lines);
            }
            lines.push(line);
        }
    }

    fn next_tag(&mut self) -> String {
        let tag = format!("A{:04}", self.next_tag);
        self.next_tag = self.next_tag.saturating_add(1);
        tag
    }

    fn write_command(&mut self, tag: &str, command: &str) -> Result<()> {
        let payload = format!("{tag} {command}\r\n");
        self.transport
            .write_all(payload.as_bytes())
            .map_err(|error| {
                CourierError::with_source(
                    ErrorCode::Imap,
                    format!("failed to write IMAP command '{command}'"),
                    error,
                )
            })?;
        self.transport.flush().map_err(|error| {
            CourierError::with_source(
                ErrorCode::Imap,
                format!("failed to flush IMAP command '{command}'"),
                error,
            )
        })
    }

    fn read_line_string(&mut self) -> Result<String> {
        let bytes = self.read_line_bytes()?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }

    fn read_line_bytes(&mut self) -> Result<Vec<u8>> {
        loop {
            if let Some(position) = self.read_buffer.iter().position(|byte| *byte == b'\n') {
                let mut line: Vec<u8> = self.read_buffer.drain(..=position).collect();
                if line.last() == Some(&b'\n') {
                    line.pop();
                }
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(line);
            }

            let mut chunk = [0u8; 4096];
            let read = self.transport.read(&mut chunk).map_err(|error| {
                CourierError::with_source(ErrorCode::Imap, "failed to read from IMAP socket", error)
            })?;
            if read == 0 {
                return Err(imap_error(
                    ImapErrorKind::Connection,
                    "unexpected EOF while reading IMAP response",
                ));
            }
            self.read_buffer.extend_from_slice(&chunk[..read]);
        }
    }

    fn read_exact_bytes(&mut self, len: usize) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        let drain = len.min(self.read_buffer.len());
        out.extend(self.read_buffer.drain(..drain));

        while out.len() < len {
            let remaining = len - out.len();
            let mut chunk = vec![0u8; remaining.min(4096)];
            let read = self.transport.read(&mut chunk).map_err(|error| {
                CourierError::with_source(ErrorCode::Imap, "failed to read IMAP literal", error)
            })?;
            if read == 0 {
                return Err(imap_error(
                    ImapErrorKind::Connection,
                    "unexpected EOF while reading IMAP literal",
                ));
            }
            out.extend_from_slice(&chunk[..read]);
        }

        Ok(out)
    }
}

fn format_uid_sequence_set(uids: &[u32]) -> String {
    if uids.is_empty() {
        return String::new();
    }

    let mut ordered: Vec<u32> = uids.to_vec();
    ordered.sort_unstable();
    ordered.dedup();

    let mut parts = Vec::new();
    let mut start = ordered[0];
    let mut end = ordered[0];

    for uid in ordered.into_iter().skip(1) {
        if uid == end.saturating_add(1) {
            end = uid;
            continue;
        }
        parts.push(render_uid_range(start, end));
        start = uid;
        end = uid;
    }
    parts.push(render_uid_range(start, end));

    parts.join(",")
}

fn render_uid_range(start: u32, end: u32) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}:{end}")
    }
}

fn parse_imap_proxy(proxy_url: &str) -> Result<ImapProxy> {
    let parsed = reqwest::Url::parse(proxy_url).map_err(|error| {
        CourierError::with_source(
            ErrorCode::ConfigParse,
            format!("invalid IMAP proxy URL '{proxy_url}'"),
            error,
        )
    })?;

    let scheme = match parsed.scheme() {
        "http" => ImapProxyScheme::Http,
        "socks5" | "socks5h" => ImapProxyScheme::Socks5,
        unsupported => {
            return Err(CourierError::new(
                ErrorCode::ConfigParse,
                format!(
                    "unsupported IMAP proxy scheme '{unsupported}'; use http://, socks5://, or socks5h://"
                ),
            ));
        }
    };

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(CourierError::new(
            ErrorCode::ConfigParse,
            "IMAP proxy authentication is not supported yet; use an unauthenticated local proxy",
        ));
    }

    if parsed.query().is_some()
        || parsed.fragment().is_some()
        || !(parsed.path().is_empty() || parsed.path() == "/")
    {
        return Err(CourierError::new(
            ErrorCode::ConfigParse,
            format!("invalid IMAP proxy URL '{proxy_url}': remove path, query, and fragment"),
        ));
    }

    let host = parsed.host_str().ok_or_else(|| {
        CourierError::new(
            ErrorCode::ConfigParse,
            format!("invalid IMAP proxy URL '{proxy_url}': missing host"),
        )
    })?;
    let port = parsed.port().unwrap_or(match scheme {
        ImapProxyScheme::Http => 80,
        ImapProxyScheme::Socks5 => 1080,
    });

    Ok(ImapProxy {
        scheme,
        host: host.to_string(),
        port,
    })
}

fn configure_tcp_timeouts(stream: &TcpStream, label: &str) -> Result<()> {
    stream
        .set_read_timeout(Some(Duration::from_secs(REMOTE_IMAP_TIMEOUT_SECS)))
        .map_err(|error| {
            CourierError::with_source(
                ErrorCode::Imap,
                format!("failed to configure IMAP read timeout for {label}"),
                error,
            )
        })?;
    stream
        .set_write_timeout(Some(Duration::from_secs(REMOTE_IMAP_TIMEOUT_SECS)))
        .map_err(|error| {
            CourierError::with_source(
                ErrorCode::Imap,
                format!("failed to configure IMAP write timeout for {label}"),
                error,
            )
        })?;

    Ok(())
}

fn connect_direct_tcp(server: &str, port: u16) -> Result<TcpStream> {
    let stream = TcpStream::connect((server, port)).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Imap,
            format!("failed to connect to IMAP server {server}:{port}"),
            error,
        )
    })?;
    configure_tcp_timeouts(&stream, &format!("IMAP server {server}:{port}"))?;

    Ok(stream)
}

fn read_http_proxy_response<S: Read>(
    stream: &mut S,
    proxy: &ImapProxy,
    target: &str,
) -> Result<String> {
    let mut response = Vec::new();
    let mut byte = [0u8; 1];
    while !response.ends_with(b"\r\n\r\n") {
        let read = stream.read(&mut byte).map_err(|error| {
            CourierError::with_source(
                ErrorCode::Imap,
                format!(
                    "failed while reading IMAP proxy {} response for {target}",
                    proxy.redacted_url()
                ),
                error,
            )
        })?;
        if read == 0 {
            return Err(CourierError::new(
                ErrorCode::Imap,
                format!(
                    "IMAP proxy {} closed the connection before CONNECT to {target} completed",
                    proxy.redacted_url()
                ),
            ));
        }
        response.push(byte[0]);
        if response.len() > HTTP_PROXY_RESPONSE_MAX_BYTES {
            return Err(CourierError::new(
                ErrorCode::Imap,
                format!(
                    "IMAP proxy {} sent too much HTTP response data while tunneling {target}",
                    proxy.redacted_url()
                ),
            ));
        }
    }

    Ok(String::from_utf8_lossy(&response).into_owned())
}

fn establish_http_connect_tunnel<S: Read + Write>(
    stream: &mut S,
    proxy: &ImapProxy,
    server: &str,
    port: u16,
) -> Result<()> {
    let target = format!("{server}:{port}");
    let request = format!(
        "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nProxy-Connection: Keep-Alive\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Imap,
            format!(
                "failed to send IMAP CONNECT request through proxy {} for {target}",
                proxy.redacted_url()
            ),
            error,
        )
    })?;
    stream.flush().map_err(|error| {
        CourierError::with_source(
            ErrorCode::Imap,
            format!(
                "failed to flush IMAP CONNECT request through proxy {} for {target}",
                proxy.redacted_url()
            ),
            error,
        )
    })?;

    let response = read_http_proxy_response(stream, proxy, &target)?;
    let status_line = response
        .lines()
        .next()
        .unwrap_or_default()
        .trim_end_matches('\r')
        .to_string();
    let mut parts = status_line.split_whitespace();
    let protocol = parts.next().unwrap_or_default();
    let status_code = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or_default();
    if !protocol.starts_with("HTTP/") || !(200..300).contains(&status_code) {
        return Err(CourierError::new(
            ErrorCode::Imap,
            format!(
                "IMAP proxy {} rejected CONNECT to {target}: {status_line}",
                proxy.redacted_url()
            ),
        ));
    }

    Ok(())
}

fn read_socks5_reply_address<S: Read>(stream: &mut S, proxy: &ImapProxy) -> Result<()> {
    let mut atyp = [0u8; 1];
    stream.read_exact(&mut atyp).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Imap,
            format!(
                "failed to read SOCKS5 reply type from IMAP proxy {}",
                proxy.redacted_url()
            ),
            error,
        )
    })?;
    let address_len = match atyp[0] {
        0x01 => 4usize,
        0x03 => {
            let mut length = [0u8; 1];
            stream.read_exact(&mut length).map_err(|error| {
                CourierError::with_source(
                    ErrorCode::Imap,
                    format!(
                        "failed to read SOCKS5 domain length from IMAP proxy {}",
                        proxy.redacted_url()
                    ),
                    error,
                )
            })?;
            length[0] as usize
        }
        0x04 => 16usize,
        value => {
            return Err(CourierError::new(
                ErrorCode::Imap,
                format!(
                    "IMAP proxy {} returned unsupported SOCKS5 address type 0x{value:02x}",
                    proxy.redacted_url()
                ),
            ));
        }
    };

    let mut discard = vec![0u8; address_len + 2];
    stream.read_exact(&mut discard).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Imap,
            format!(
                "failed to read SOCKS5 bind address from IMAP proxy {}",
                proxy.redacted_url()
            ),
            error,
        )
    })?;
    Ok(())
}

fn socks5_reply_text(code: u8) -> &'static str {
    match code {
        0x01 => "general SOCKS server failure",
        0x02 => "connection not allowed by ruleset",
        0x03 => "network unreachable",
        0x04 => "host unreachable",
        0x05 => "connection refused",
        0x06 => "TTL expired",
        0x07 => "command not supported",
        0x08 => "address type not supported",
        _ => "unknown SOCKS5 error",
    }
}

fn establish_socks5_tunnel<S: Read + Write>(
    stream: &mut S,
    proxy: &ImapProxy,
    server: &str,
    port: u16,
) -> Result<()> {
    stream.write_all(&[0x05, 0x01, 0x00]).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Imap,
            format!(
                "failed to send SOCKS5 greeting to IMAP proxy {}",
                proxy.redacted_url()
            ),
            error,
        )
    })?;

    let mut greeting_reply = [0u8; 2];
    stream.read_exact(&mut greeting_reply).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Imap,
            format!(
                "failed to read SOCKS5 greeting reply from IMAP proxy {}",
                proxy.redacted_url()
            ),
            error,
        )
    })?;
    if greeting_reply[0] != 0x05 {
        return Err(CourierError::new(
            ErrorCode::Imap,
            format!(
                "IMAP proxy {} returned invalid SOCKS5 version 0x{:02x}",
                proxy.redacted_url(),
                greeting_reply[0]
            ),
        ));
    }
    if greeting_reply[1] != 0x00 {
        return Err(CourierError::new(
            ErrorCode::Imap,
            format!(
                "IMAP proxy {} does not allow unauthenticated SOCKS5 connections",
                proxy.redacted_url()
            ),
        ));
    }

    let host = server.as_bytes();
    if host.len() > u8::MAX as usize {
        return Err(CourierError::new(
            ErrorCode::Imap,
            format!("IMAP server name '{server}' is too long for SOCKS5 proxying"),
        ));
    }

    let mut request = Vec::with_capacity(4 + 1 + host.len() + 2);
    request.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, host.len() as u8]);
    request.extend_from_slice(host);
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Imap,
            format!(
                "failed to send SOCKS5 CONNECT request through IMAP proxy {}",
                proxy.redacted_url()
            ),
            error,
        )
    })?;

    let mut reply = [0u8; 3];
    stream.read_exact(&mut reply).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Imap,
            format!(
                "failed to read SOCKS5 CONNECT status from IMAP proxy {}",
                proxy.redacted_url()
            ),
            error,
        )
    })?;
    if reply[0] != 0x05 {
        return Err(CourierError::new(
            ErrorCode::Imap,
            format!(
                "IMAP proxy {} returned invalid SOCKS5 version 0x{:02x}",
                proxy.redacted_url(),
                reply[0]
            ),
        ));
    }
    if reply[1] != 0x00 {
        return Err(CourierError::new(
            ErrorCode::Imap,
            format!(
                "IMAP proxy {} failed to connect to {server}:{port}: {}",
                proxy.redacted_url(),
                socks5_reply_text(reply[1])
            ),
        ));
    }
    read_socks5_reply_address(stream, proxy)?;

    Ok(())
}

fn connect_tcp_via_proxy(proxy: &ImapProxy, server: &str, port: u16) -> Result<TcpStream> {
    let target = format!("{server}:{port}");
    let stream = TcpStream::connect((proxy.host.as_str(), proxy.port)).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Imap,
            format!(
                "failed to connect to IMAP proxy {} for {target}",
                proxy.redacted_url()
            ),
            error,
        )
    })?;
    configure_tcp_timeouts(&stream, &format!("IMAP proxy {}", proxy.redacted_url()))?;

    let mut stream = stream;
    match proxy.scheme {
        ImapProxyScheme::Http => establish_http_connect_tunnel(&mut stream, proxy, server, port)?,
        ImapProxyScheme::Socks5 => establish_socks5_tunnel(&mut stream, proxy, server, port)?,
    }

    Ok(stream)
}

fn connect_tcp(config: &ImapConfig, server: &str, port: u16) -> Result<TcpStream> {
    if let Some(proxy_url) = config.proxy.as_deref() {
        let proxy = parse_imap_proxy(proxy_url)?;
        tracing::info!(
            op = "imap_connect",
            mode = "proxy",
            proxy = %proxy.redacted_url(),
            target = %format!("{server}:{port}")
        );
        connect_tcp_via_proxy(&proxy, server, port)
    } else {
        connect_direct_tcp(server, port)
    }
}

fn connect_tls(
    server: &str,
    stream: TcpStream,
) -> Result<StreamOwned<ClientConnection, TcpStream>> {
    let root_store = RootCertStore::from_iter(TLS_SERVER_ROOTS.iter().cloned());
    let client_config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|error| {
                CourierError::with_source(
                    ErrorCode::Imap,
                    "failed to configure TLS protocol versions for IMAP client",
                    error,
                )
            })?
            .with_root_certificates(root_store)
            .with_no_client_auth();
    let server_name = ServerName::try_from(server.to_string()).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Imap,
            format!("invalid IMAP server name '{server}' for TLS"),
            error,
        )
    })?;
    let connection =
        ClientConnection::new(Arc::new(client_config), server_name).map_err(|error| {
            CourierError::with_source(
                ErrorCode::Imap,
                format!("failed to initialize TLS session for IMAP server '{server}'"),
                error,
            )
        })?;

    Ok(StreamOwned::new(connection, stream))
}

fn quote_imap_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn ensure_tagged_ok(line: &str, kind: ImapErrorKind) -> Result<()> {
    let mut parts = line.split_whitespace();
    let _tag = parts.next();
    match parts
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase()
        .as_str()
    {
        "OK" => Ok(()),
        _ => Err(imap_error(kind, line.to_string())),
    }
}

fn parse_status_code_u64(line: &str, key: &str) -> Option<u64> {
    let needle = format!("[{key} ");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find(']')?;
    rest[..end].trim().parse::<u64>().ok()
}

fn parse_fetch_uid(line: &str) -> Option<u32> {
    parse_numeric_token(line, "UID ").and_then(|value| value.parse::<u32>().ok())
}

fn parse_fetch_modseq(line: &str) -> Option<u64> {
    parse_numeric_token(line, "MODSEQ (").and_then(|value| value.parse::<u64>().ok())
}

fn parse_numeric_token<'a>(line: &'a str, needle: &str) -> Option<&'a str> {
    let start = line.find(needle)? + needle.len();
    let rest = &line[start..];
    let end = rest
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(rest.len());
    if end == 0 { None } else { Some(&rest[..end]) }
}

fn parse_fetch_flags(line: &str) -> Vec<String> {
    let Some(start) = line.find("FLAGS (") else {
        return Vec::new();
    };
    let rest = &line[start + "FLAGS (".len()..];
    let Some(end) = rest.find(')') else {
        return Vec::new();
    };
    split_flags(&rest[..end])
}

fn parse_literal_len(line: &str) -> Option<usize> {
    let trimmed = line.trim_end();
    let suffix = trimmed.strip_suffix('}')?;
    let start = suffix.rfind('{')?;
    suffix[start + 1..].parse::<usize>().ok()
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

fn parse_gnu_archive_month_entries(html: &str) -> Result<Vec<GnuArchiveMonthEntry>> {
    let mut entries = Vec::new();
    let mut seen_months = HashSet::new();

    for line in html.lines() {
        let Some(anchor_start) = line.find("<a href=\"") else {
            continue;
        };
        let href_start = anchor_start + "<a href=\"".len();
        let Some(href_end) = line[href_start..].find('"') else {
            continue;
        };
        let href = &line[href_start..href_start + href_end];
        let Some((year, month)) = parse_year_month_key(href) else {
            continue;
        };

        let Some(anchor_end) = line.find("</a>") else {
            continue;
        };
        let mut parts = line[anchor_end + "</a>".len()..].split_whitespace();
        let (Some(date), Some(time)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Some(modseq) = parse_gnu_archive_listing_timestamp(date, time) else {
            continue;
        };

        let month_key = format!("{year:04}-{month:02}");
        if seen_months.insert(month_key.clone()) {
            entries.push(GnuArchiveMonthEntry { month_key, modseq });
        }
    }

    entries.sort_by(|left, right| {
        left.month_key
            .cmp(&right.month_key)
            .then_with(|| left.modseq.cmp(&right.modseq))
    });

    Ok(entries)
}

fn parse_year_month_key(value: &str) -> Option<(u32, u32)> {
    let trimmed = value.trim_matches('/');
    let (year, month) = trimmed.split_once('-')?;
    if year.len() != 4 || month.len() != 2 {
        return None;
    }

    let year = year.parse::<u32>().ok()?;
    let month = month.parse::<u32>().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }

    Some((year, month))
}

fn parse_gnu_archive_listing_timestamp(date: &str, time: &str) -> Option<u64> {
    let naive = NaiveDateTime::parse_from_str(&format!("{date} {time}"), "%Y-%m-%d %H:%M").ok()?;
    DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc)
        .timestamp()
        .try_into()
        .ok()
}

fn select_gnu_archive_months(
    months: &[GnuArchiveMonthEntry],
    since_modseq: Option<u64>,
) -> Vec<GnuArchiveMonthEntry> {
    if months.is_empty() {
        return Vec::new();
    }

    let mut selected = Vec::new();
    let mut seen = HashSet::new();

    if let Some(checkpoint) = since_modseq {
        for entry in months.iter().filter(|entry| entry.modseq > checkpoint) {
            if seen.insert(entry.month_key.clone()) {
                selected.push(entry.clone());
            }
        }

        if let Some(latest) = months.last()
            && seen.insert(latest.month_key.clone())
        {
            // GNU archive directory timestamps are only minute-resolution. Keep
            // polling the newest month even when the index timestamp has not
            // advanced yet so current-month mail is not missed.
            selected.push(latest.clone());
        }
    } else {
        for entry in months
            .iter()
            .rev()
            .take(GNU_ARCHIVE_INITIAL_MONTH_LIMIT)
            .rev()
        {
            if seen.insert(entry.month_key.clone()) {
                selected.push(entry.clone());
            }
        }
    }

    selected.sort_by(|left, right| left.month_key.cmp(&right.month_key));
    selected
}

fn parse_gnu_archive_mbox_messages(raw: &[u8]) -> Vec<Vec<u8>> {
    let mut messages = Vec::new();
    let mut current = Vec::new();

    for line in raw.split_inclusive(|byte| *byte == b'\n') {
        let normalized = trim_ascii_line_ending(line);
        if normalized.starts_with(b"From ") {
            if !current.is_empty() {
                messages.push(current);
                current = Vec::new();
            }
            continue;
        }

        if normalized.starts_with(b">From ") {
            current.extend_from_slice(&line[1..]);
        } else {
            current.extend_from_slice(line);
        }
    }

    if !current.is_empty() {
        messages.push(current);
    }

    messages
}

fn trim_ascii_line_ending(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

fn gnu_archive_message_uid(month_key: &str, index: usize) -> u32 {
    let Some((year, month)) = parse_year_month_key(month_key) else {
        return 0;
    };
    if year < 2000 || index + 1 >= GNU_ARCHIVE_UID_STRIDE as usize {
        return 0;
    }

    let month_ordinal = (year - 2000) * 12 + (month - 1);
    let uid = month_ordinal as u64 * GNU_ARCHIVE_UID_STRIDE as u64 + index as u64 + 1;
    u32::try_from(uid).unwrap_or(0)
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
    use std::io::{Read, Write};
    use std::path::PathBuf;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use crate::infra::config::{ImapConfig, ImapEncryption};

    use super::{
        FixtureImapClient, GnuArchiveMonthEntry, ImapClient, ImapErrorKind, RemoteImapClient,
        ensure_tagged_ok, establish_http_connect_tunnel, establish_socks5_tunnel,
        format_uid_sequence_set, gnu_archive_message_uid, lore_raw_url_candidates,
        parse_atom_timestamp, parse_fetch_flags, parse_fetch_modseq, parse_fetch_uid,
        parse_gnu_archive_mbox_messages, parse_gnu_archive_month_entries, parse_imap_proxy,
        parse_literal_len, parse_lore_atom_entries, parse_status_code_u64,
        select_gnu_archive_months,
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

    #[derive(Default)]
    struct MockStream {
        reads: Vec<u8>,
        read_offset: usize,
        writes: Vec<u8>,
    }

    impl MockStream {
        fn with_reads(reads: &[u8]) -> Self {
            Self {
                reads: reads.to_vec(),
                read_offset: 0,
                writes: Vec::new(),
            }
        }
    }

    impl Read for MockStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.read_offset >= self.reads.len() {
                return Ok(0);
            }
            let available = self.reads.len() - self.read_offset;
            let count = available.min(buf.len());
            buf[..count].copy_from_slice(&self.reads[self.read_offset..self.read_offset + count]);
            self.read_offset += count;
            Ok(count)
        }
    }

    impl Write for MockStream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.writes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
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
    fn remote_client_accepts_complete_config() {
        let client = RemoteImapClient::new(ImapConfig {
            email: Some("me@example.com".to_string()),
            user: Some("imap-user".to_string()),
            pass: Some("imap-pass".to_string()),
            server: Some("imap.example.com".to_string()),
            server_port: Some(993),
            encryption: Some(ImapEncryption::Tls),
            proxy: None,
        });

        assert!(client.is_ok());
    }

    #[test]
    fn remote_client_rejects_incomplete_config() {
        let error = RemoteImapClient::new(ImapConfig {
            email: None,
            user: Some("imap-user".to_string()),
            pass: None,
            server: Some("imap.example.com".to_string()),
            server_port: Some(993),
            encryption: Some(ImapEncryption::Tls),
            proxy: None,
        })
        .err()
        .expect("incomplete config should fail");

        assert!(error.to_string().contains("imap.pass"));
    }

    #[test]
    fn parses_imap_fetch_metadata() {
        let line = "* 2 FETCH (UID 2 FLAGS (\\Seen \\Answered) MODSEQ (20) BODY[] {123}";
        assert_eq!(parse_fetch_uid(line), Some(2));
        assert_eq!(parse_fetch_modseq(line), Some(20));
        assert_eq!(parse_literal_len(line), Some(123));
        assert_eq!(
            parse_fetch_flags(line),
            vec!["\\Seen".to_string(), "\\Answered".to_string()]
        );
    }

    #[test]
    fn parses_select_status_codes() {
        assert_eq!(
            parse_status_code_u64("* OK [UIDVALIDITY 77] UIDs valid", "UIDVALIDITY"),
            Some(77)
        );
        assert_eq!(
            parse_status_code_u64("* OK [UIDNEXT 42] Predicted next UID", "UIDNEXT"),
            Some(42)
        );
        assert_eq!(
            parse_status_code_u64("* OK [HIGHESTMODSEQ 9001] Highest", "HIGHESTMODSEQ"),
            Some(9001)
        );
    }

    #[test]
    fn tagged_errors_keep_imap_error_classification() {
        let error = ensure_tagged_ok(
            "A0002 NO [AUTHENTICATIONFAILED] invalid credentials",
            ImapErrorKind::Authentication,
        )
        .expect_err("tagged failure should surface");

        assert!(error.to_string().contains("authentication"));
    }

    #[test]
    fn tls_client_config_uses_explicit_crypto_provider() {
        let root_store =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let _config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("configure protocol versions")
        .with_root_certificates(root_store)
        .with_no_client_auth();
    }

    #[test]
    fn uid_sequence_set_compacts_contiguous_ranges() {
        assert_eq!(format_uid_sequence_set(&[1, 2, 3, 5, 8, 9]), "1:3,5,8:9");
        assert_eq!(format_uid_sequence_set(&[5]), "5");
        assert_eq!(format_uid_sequence_set(&[9, 7, 8, 8]), "7:9");
    }

    #[test]
    fn http_proxy_connect_tunnels_imap_socket() {
        let proxy = parse_imap_proxy("http://127.0.0.1:7890").expect("parse proxy");
        let mut stream = MockStream::with_reads(b"HTTP/1.1 200 Connection established\r\n\r\n");

        establish_http_connect_tunnel(&mut stream, &proxy, "imap.gmail.com", 993)
            .expect("proxy tunnel should connect");

        let request_text = String::from_utf8(stream.writes).expect("request utf8");
        assert!(request_text.starts_with("CONNECT imap.gmail.com:993 HTTP/1.1\r\n"));
        assert!(request_text.contains("\r\nHost: imap.gmail.com:993\r\n"));
    }

    #[test]
    fn socks5_proxy_connect_tunnels_imap_socket() {
        let proxy = parse_imap_proxy("socks5://127.0.0.1:7890").expect("parse proxy");
        let mut stream = MockStream::with_reads(&[
            0x05, 0x00, // greeting reply
            0x05, 0x00, 0x00, // connect reply status
            0x03, 0x0e, b'i', b'm', b'a', b'p', b'.', b'g', b'm', b'a', b'i', b'l', b'.', b'c',
            b'o', b'm', 0x03, 0xe1, // bound address + port
        ]);

        establish_socks5_tunnel(&mut stream, &proxy, "imap.gmail.com", 993)
            .expect("proxy tunnel should connect");

        assert_eq!(
            stream.writes,
            [
                0x05, 0x01, 0x00, // greeting
                0x05, 0x01, 0x00, 0x03, 14, b'i', b'm', b'a', b'p', b'.', b'g', b'm', b'a', b'i',
                b'l', b'.', b'c', b'o', b'm', 0x03, 0xe1
            ]
        );
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

    #[test]
    fn parses_gnu_archive_month_entries() {
        let html = r#"
<pre>
<img src="/icons/unknown.gif" alt="[   ]"> <a href="2026-02">2026-02</a>                 2026-02-26 09:12  855K
<img src="/icons/unknown.gif" alt="[   ]"> <a href="2026-03">2026-03</a>                 2026-03-07 06:37  341K
</pre>
"#;

        let entries = parse_gnu_archive_month_entries(html).expect("parse month entries");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].month_key, "2026-02");
        assert_eq!(entries[1].month_key, "2026-03");
        assert!(entries[1].modseq >= entries[0].modseq);
    }

    #[test]
    fn selects_latest_gnu_archive_month_and_recent_history() {
        let months = vec![
            GnuArchiveMonthEntry {
                month_key: "2026-01".to_string(),
                modseq: 10,
            },
            GnuArchiveMonthEntry {
                month_key: "2026-02".to_string(),
                modseq: 20,
            },
            GnuArchiveMonthEntry {
                month_key: "2026-03".to_string(),
                modseq: 30,
            },
        ];

        let initial = select_gnu_archive_months(&months, None);
        assert_eq!(
            initial
                .iter()
                .map(|entry| entry.month_key.as_str())
                .collect::<Vec<_>>(),
            vec!["2026-02", "2026-03"]
        );

        let incremental = select_gnu_archive_months(&months, Some(30));
        assert_eq!(
            incremental
                .iter()
                .map(|entry| entry.month_key.as_str())
                .collect::<Vec<_>>(),
            vec!["2026-03"]
        );
    }

    #[test]
    fn parses_gnu_archive_mbox_messages() {
        let raw = b"From MAILER-DAEMON Tue Mar 03 04:39:31 2026\nMessage-ID: <msg-a@example.com>\nSubject: one\n\nbody\n>From escaped\nFrom MAILER-DAEMON Tue Mar 03 04:40:31 2026\nMessage-ID: <msg-b@example.com>\nSubject: two\n\nbody two\n";

        let messages = parse_gnu_archive_mbox_messages(raw);
        assert_eq!(messages.len(), 2);
        assert!(String::from_utf8_lossy(&messages[0]).contains("Message-ID: <msg-a@example.com>"));
        assert!(String::from_utf8_lossy(&messages[0]).contains("\nFrom escaped\n"));
        assert!(String::from_utf8_lossy(&messages[1]).contains("Message-ID: <msg-b@example.com>"));
    }

    #[test]
    fn assigns_stable_gnu_archive_uids() {
        assert_eq!(gnu_archive_message_uid("2026-03", 0), 314_000_001);
        assert_eq!(gnu_archive_message_uid("2026-03", 41), 314_000_042);
    }
}
