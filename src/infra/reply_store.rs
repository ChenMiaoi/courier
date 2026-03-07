use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::infra::error::{CourierError, ErrorCode, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplySendStatus {
    Sent,
    Failed,
    TimedOut,
}

impl ReplySendStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
        }
    }

    #[allow(dead_code)]
    fn from_db(value: &str) -> Self {
        match value {
            "sent" => Self::Sent,
            "timed_out" => Self::TimedOut,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReplySendRecordRequest {
    pub thread_id: i64,
    pub mail_id: i64,
    pub transport: String,
    pub message_id: String,
    pub from_addr: String,
    pub to_addrs: String,
    pub cc_addrs: String,
    pub subject: String,
    pub preview_confirmed_at: String,
    pub status: ReplySendStatus,
    pub command: Option<String>,
    pub draft_path: Option<PathBuf>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub error_summary: Option<String>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub started_at: String,
    pub finished_at: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ReplySendRecord {
    pub id: i64,
    pub thread_id: i64,
    pub mail_id: i64,
    pub transport: String,
    pub message_id: String,
    pub from_addr: String,
    pub to_addrs: String,
    pub cc_addrs: String,
    pub subject: String,
    pub preview_confirmed_at: String,
    pub status: ReplySendStatus,
    pub command: Option<String>,
    pub draft_path: Option<PathBuf>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub error_summary: Option<String>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub started_at: String,
    pub finished_at: String,
}

pub fn insert_reply_send(path: &Path, request: &ReplySendRecordRequest) -> Result<i64> {
    let connection = open_connection(path)?;
    let draft_path = request
        .draft_path
        .as_ref()
        .map(|path| path.display().to_string());

    connection
        .execute(
            "
INSERT INTO reply_send(
    thread_id, mail_id, transport, message_id, from_addr, to_addrs, cc_addrs, subject,
    preview_confirmed_at, status, command, draft_path, exit_code, timed_out, error_summary,
    stdout, stderr, started_at, finished_at
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
",
            params![
                request.thread_id,
                request.mail_id,
                request.transport,
                request.message_id,
                request.from_addr,
                request.to_addrs,
                request.cc_addrs,
                request.subject,
                request.preview_confirmed_at,
                request.status.as_str(),
                request.command,
                draft_path,
                request.exit_code,
                bool_to_i64(request.timed_out),
                request.error_summary,
                request.stdout,
                request.stderr,
                request.started_at,
                request.finished_at,
            ],
        )
        .map_err(|error| {
            CourierError::with_source(
                ErrorCode::Database,
                format!(
                    "failed to persist reply send record for mail {} thread {}",
                    request.mail_id, request.thread_id
                ),
                error,
            )
        })?;

    Ok(connection.last_insert_rowid())
}

#[allow(dead_code)]
pub fn latest_reply_send_for_mail(path: &Path, mail_id: i64) -> Result<Option<ReplySendRecord>> {
    let connection = open_connection(path)?;
    connection
        .query_row(
            "
SELECT
    id, thread_id, mail_id, transport, message_id, from_addr, to_addrs, cc_addrs, subject,
    preview_confirmed_at, status, command, draft_path, exit_code, timed_out, error_summary,
    stdout, stderr, started_at, finished_at
FROM reply_send
WHERE mail_id = ?1
ORDER BY id DESC
LIMIT 1
",
            params![mail_id],
            |row| {
                Ok(ReplySendRecord {
                    id: row.get::<_, i64>(0)?,
                    thread_id: row.get::<_, i64>(1)?,
                    mail_id: row.get::<_, i64>(2)?,
                    transport: row.get::<_, String>(3)?,
                    message_id: row.get::<_, String>(4)?,
                    from_addr: row.get::<_, String>(5)?,
                    to_addrs: row.get::<_, String>(6)?,
                    cc_addrs: row.get::<_, String>(7)?,
                    subject: row.get::<_, String>(8)?,
                    preview_confirmed_at: row.get::<_, String>(9)?,
                    status: ReplySendStatus::from_db(&row.get::<_, String>(10)?),
                    command: row.get::<_, Option<String>>(11)?,
                    draft_path: row.get::<_, Option<String>>(12)?.map(PathBuf::from),
                    exit_code: row.get::<_, Option<i32>>(13)?,
                    timed_out: row.get::<_, i64>(14)? != 0,
                    error_summary: row.get::<_, Option<String>>(15)?,
                    stdout: row.get::<_, Option<String>>(16)?,
                    stderr: row.get::<_, Option<String>>(17)?,
                    started_at: row.get::<_, String>(18)?,
                    finished_at: row.get::<_, String>(19)?,
                })
            },
        )
        .optional()
        .map_err(|error| {
            CourierError::with_source(
                ErrorCode::Database,
                format!("failed to load latest reply send record for mail {mail_id}"),
                error,
            )
        })
}

fn open_connection(path: &Path) -> Result<Connection> {
    Connection::open(path).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Database,
            format!("failed to open sqlite database {}", path.display()),
            error,
        )
    })
}

fn bool_to_i64(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::Connection;

    use crate::infra::db;

    use super::{
        ReplySendRecordRequest, ReplySendStatus, insert_reply_send, latest_reply_send_for_mail,
    };

    fn temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("courier-reply-store-{label}-{nonce}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn persists_and_loads_latest_reply_send_record() {
        let root = temp_dir("latest");
        let db_path = root.join("courier.db");
        db::initialize(&db_path).expect("initialize db");
        let connection = Connection::open(&db_path).expect("open db");
        connection
            .execute(
                "INSERT INTO mail(id, message_id, subject, from_addr) VALUES (11, 'patch@example.com', '[PATCH] demo', 'tester@example.com')",
                [],
            )
            .expect("insert mail");
        connection
            .execute(
                "INSERT INTO thread(id, root_mail_id, subject_norm, message_count) VALUES (7, 11, '[patch] demo', 1)",
                [],
            )
            .expect("insert thread");

        insert_reply_send(
            &db_path,
            &ReplySendRecordRequest {
                thread_id: 7,
                mail_id: 11,
                transport: "git-send-email".to_string(),
                message_id: "msg-1@example.com".to_string(),
                from_addr: "Tester <tester@example.com>".to_string(),
                to_addrs: "maintainer@example.com".to_string(),
                cc_addrs: "list@example.com".to_string(),
                subject: "Re: [PATCH] demo".to_string(),
                preview_confirmed_at: "2026-03-07T10:00:00Z".to_string(),
                status: ReplySendStatus::Sent,
                command: Some("git send-email /tmp/reply.eml".to_string()),
                draft_path: Some(PathBuf::from("/tmp/reply.eml")),
                exit_code: Some(0),
                timed_out: false,
                error_summary: None,
                stdout: Some("ok".to_string()),
                stderr: Some(String::new()),
                started_at: "2026-03-07T10:00:01Z".to_string(),
                finished_at: "2026-03-07T10:00:02Z".to_string(),
            },
        )
        .expect("persist reply send");

        let record = latest_reply_send_for_mail(&db_path, 11)
            .expect("load latest reply send")
            .expect("reply send record");
        assert_eq!(record.thread_id, 7);
        assert_eq!(record.status, ReplySendStatus::Sent);
        assert_eq!(record.message_id, "msg-1@example.com");
        assert_eq!(
            record.command.as_deref(),
            Some("git send-email /tmp/reply.eml")
        );
        assert_eq!(record.draft_path, Some(PathBuf::from("/tmp/reply.eml")));

        let _ = fs::remove_dir_all(root);
    }
}
