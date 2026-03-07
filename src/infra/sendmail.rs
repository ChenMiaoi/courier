//! Reply delivery through `git send-email`.
//!
//! This module owns command discovery, draft generation, and timeout handling
//! so the rest of the application can treat mail sending as a structured
//! operation with recorded outcome metadata.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::Utc;

use crate::infra::config::RuntimeConfig;

const DEFAULT_SEND_TIMEOUT: Duration = Duration::from_secs(60);
const OUTBOX_DIR_NAME: &str = "reply-outbox";
const GIT_SENDEMAIL_FROM_ARGS: &[&str] = &["config", "sendemail.from"];
const GIT_USER_NAME_LOOKUP_ARGS: &[&str] = &["config", "user.name"];
const GIT_USER_EMAIL_LOOKUP_ARGS: &[&str] = &["config", "user.email"];

#[derive(Debug, Clone)]
pub struct GitSendEmailCheck {
    pub status: GitSendEmailStatus,
}

#[derive(Debug, Clone)]
pub enum GitSendEmailStatus {
    Available { path: PathBuf, version: String },
    Broken { path: PathBuf, reason: String },
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyIdentitySource {
    SendEmailFrom,
    UserNameEmail,
}

impl ReplyIdentitySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SendEmailFrom => "git config sendemail.from",
            Self::UserNameEmail => "git config user.name/user.email",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyIdentity {
    pub display: String,
    pub email: String,
    pub source: ReplyIdentitySource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendRequest {
    pub mail_id: i64,
    pub thread_id: i64,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub in_reply_to: String,
    pub references: Vec<String>,
    pub body: String,
    pub preview_confirmed_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendStatus {
    Sent,
    Failed,
    TimedOut,
}

#[derive(Debug, Clone)]
pub struct SendOutcome {
    pub transport: String,
    pub message_id: String,
    pub command_line: Option<String>,
    pub draft_path: Option<PathBuf>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub error_summary: Option<String>,
    pub started_at: String,
    pub finished_at: String,
    pub status: SendStatus,
}

pub fn check() -> GitSendEmailCheck {
    check_with_command_path(None)
}

pub fn resolve_reply_identity() -> std::result::Result<ReplyIdentity, String> {
    resolve_reply_identity_with_command_path(None)
}

pub fn send(runtime: &RuntimeConfig, request: &SendRequest) -> SendOutcome {
    send_with_command_path(runtime, request, None)
}

fn check_with_command_path(command_path: Option<&Path>) -> GitSendEmailCheck {
    let mut last_failure: Option<(PathBuf, String)> = None;

    for candidate in git_candidates(command_path) {
        // Stop at the first usable candidate, but remember the last broken one
        // so diagnostics can explain why discovery failed instead of only
        // saying "missing".
        match probe_send_email(&candidate) {
            Probe::Available { path, version, .. } => {
                return GitSendEmailCheck {
                    status: GitSendEmailStatus::Available { path, version },
                };
            }
            Probe::Broken { path, reason } => {
                last_failure = Some((path, reason));
            }
            Probe::Missing => {}
        }
    }

    if let Some((path, reason)) = last_failure {
        GitSendEmailCheck {
            status: GitSendEmailStatus::Broken { path, reason },
        }
    } else {
        GitSendEmailCheck {
            status: GitSendEmailStatus::Missing,
        }
    }
}

fn resolve_reply_identity_with_command_path(
    command_path: Option<&Path>,
) -> std::result::Result<ReplyIdentity, String> {
    let resolved = resolve_git_binary(command_path)?;

    // `sendemail.from` is the closest match to what `git send-email` will use,
    // so prefer it over the more general user.name/user.email pair.
    if let Some(value) = git_config_value(&resolved.command, GIT_SENDEMAIL_FROM_ARGS)? {
        let identity = parse_identity(&value).ok_or_else(|| {
            "git config sendemail.from is set but does not contain a valid email address"
                .to_string()
        })?;
        return Ok(ReplyIdentity {
            display: identity.display,
            email: identity.email,
            source: ReplyIdentitySource::SendEmailFrom,
        });
    }

    let email =
        git_config_value(&resolved.command, GIT_USER_EMAIL_LOOKUP_ARGS)?.ok_or_else(|| {
            "git email identity missing; set git config sendemail.from or user.email".to_string()
        })?;
    let name = git_config_value(&resolved.command, GIT_USER_NAME_LOOKUP_ARGS)?;
    let display = if let Some(name) = name {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            email.clone()
        } else {
            format!("{trimmed} <{email}>")
        }
    } else {
        email.clone()
    };

    Ok(ReplyIdentity {
        display,
        email,
        source: ReplyIdentitySource::UserNameEmail,
    })
}

fn send_with_command_path(
    runtime: &RuntimeConfig,
    request: &SendRequest,
    command_path: Option<&Path>,
) -> SendOutcome {
    let started_at = now_timestamp();
    let message_id = generate_message_id(&request.from);
    let draft_dir = runtime.data_dir.join(OUTBOX_DIR_NAME);
    let draft_name = format!(
        "reply-{}-{}.eml",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id()
    );
    let draft_path = draft_dir.join(draft_name);

    let resolved = match resolve_git_command(command_path) {
        Ok(resolved) => resolved,
        Err(error) => {
            return failed_outcome(
                message_id,
                started_at,
                None,
                None,
                format!("git send-email unavailable: {error}"),
            );
        }
    };

    if let Err(error) = fs::create_dir_all(&draft_dir) {
        return failed_outcome(
            message_id,
            started_at,
            None,
            None,
            format!(
                "failed to create reply outbox {}: {error}",
                draft_dir.display()
            ),
        );
    }

    let rendered = render_message_file(request, &message_id);
    if let Err(error) = fs::write(&draft_path, rendered) {
        return failed_outcome(
            message_id,
            started_at,
            None,
            None,
            format!(
                "failed to write reply draft {}: {error}",
                draft_path.display()
            ),
        );
    }

    let command_line = render_command_line(
        &resolved.display_name,
        &build_send_email_args(request, &draft_path),
    );

    let mut command = Command::new(&resolved.command);
    command.args(build_send_email_args(request, &draft_path));
    command
        .current_dir(resolve_working_dir(runtime))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Never let git prompt interactively inside the TUI path; failures must
        // surface as structured outcomes the UI can record and display.
        .env("GIT_TERMINAL_PROMPT", "0");

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            return failed_outcome(
                message_id,
                started_at,
                Some(command_line),
                Some(draft_path),
                format!("failed to start git send-email: {error}"),
            );
        }
    };

    let start = Instant::now();
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() >= DEFAULT_SEND_TIMEOUT {
                    timed_out = true;
                    let _ = child.kill();
                    break;
                }
                thread::sleep(Duration::from_millis(30));
            }
            Err(error) => {
                return failed_outcome(
                    message_id,
                    started_at,
                    Some(command_line),
                    Some(draft_path),
                    format!("failed while waiting for git send-email: {error}"),
                );
            }
        }
    }

    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(error) => {
            return failed_outcome(
                message_id,
                started_at,
                Some(command_line),
                Some(draft_path),
                format!("failed to collect git send-email output: {error}"),
            );
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let finished_at = now_timestamp();

    if timed_out {
        return SendOutcome {
            transport: "git-send-email".to_string(),
            message_id,
            command_line: Some(command_line),
            draft_path: Some(draft_path),
            exit_code: output.status.code(),
            timed_out: true,
            stdout,
            stderr,
            error_summary: Some(format!(
                "git send-email timed out after {}s",
                DEFAULT_SEND_TIMEOUT.as_secs()
            )),
            started_at,
            finished_at,
            status: SendStatus::TimedOut,
        };
    }

    if output.status.success() {
        // Remove the draft only after a confirmed send so failures leave behind
        // an inspectable artifact the user can reuse or resend manually.
        let _ = fs::remove_file(&draft_path);
        return SendOutcome {
            transport: "git-send-email".to_string(),
            message_id,
            command_line: Some(command_line),
            draft_path: None,
            exit_code: output.status.code(),
            timed_out: false,
            stdout,
            stderr,
            error_summary: None,
            started_at,
            finished_at,
            status: SendStatus::Sent,
        };
    }

    SendOutcome {
        transport: "git-send-email".to_string(),
        message_id,
        command_line: Some(command_line),
        draft_path: Some(draft_path),
        exit_code: output.status.code(),
        timed_out: false,
        stdout: stdout.clone(),
        stderr: stderr.clone(),
        error_summary: summarize_failure(output.status.code(), &stdout, &stderr),
        started_at,
        finished_at,
        status: SendStatus::Failed,
    }
}

#[derive(Debug, Clone)]
struct ParsedIdentity {
    display: String,
    email: String,
}

#[derive(Debug, Clone)]
struct ResolvedGitCommand {
    command: String,
    display_name: String,
}

enum GitCandidate {
    Path(PathBuf),
    Program(&'static str),
}

enum Probe {
    Available {
        path: PathBuf,
        version: String,
        command: String,
    },
    Broken {
        path: PathBuf,
        reason: String,
    },
    Missing,
}

fn git_candidates(command_path: Option<&Path>) -> Vec<GitCandidate> {
    let mut candidates = Vec::new();
    if let Some(path) = command_path {
        candidates.push(GitCandidate::Path(path.to_path_buf()));
    }
    candidates.push(GitCandidate::Program("git"));
    candidates
}

fn probe_send_email(candidate: &GitCandidate) -> Probe {
    match candidate {
        GitCandidate::Path(path) => {
            if !path.exists() {
                return Probe::Missing;
            }
            run_send_email_probe(path, path, path.display().to_string())
        }
        GitCandidate::Program(program) => run_send_email_probe(
            program,
            &PathBuf::from(format!("{program} (PATH)")),
            (*program).to_string(),
        ),
    }
}

fn probe_git_binary(candidate: &GitCandidate) -> Probe {
    match candidate {
        GitCandidate::Path(path) => {
            if !path.exists() {
                return Probe::Missing;
            }
            run_probe(path, &["--version"], path, path.display().to_string())
        }
        GitCandidate::Program(program) => run_probe(
            program,
            &["--version"],
            &PathBuf::from(format!("{program} (PATH)")),
            (*program).to_string(),
        ),
    }
}

fn run_probe<T>(command: T, args: &[&str], display_path: &Path, command_text: String) -> Probe
where
    T: AsRef<std::ffi::OsStr>,
{
    match Command::new(command).args(args).output() {
        Ok(output) if output.status.success() => Probe::Available {
            path: display_path.to_path_buf(),
            version: normalize_output(&output.stdout)
                .or_else(|| normalize_output(&output.stderr))
                .unwrap_or_else(|| "unknown".to_string()),
            command: command_text,
        },
        Ok(output) => Probe::Broken {
            path: display_path.to_path_buf(),
            reason: normalize_output(&output.stderr)
                .or_else(|| normalize_output(&output.stdout))
                .unwrap_or_else(|| format!("exit status {}", output.status)),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Probe::Missing,
        Err(error) => Probe::Broken {
            path: display_path.to_path_buf(),
            reason: error.to_string(),
        },
    }
}

fn run_send_email_probe<T>(command: T, display_path: &Path, command_text: String) -> Probe
where
    T: AsRef<std::ffi::OsStr> + Copy,
{
    match Command::new(command).args(["send-email", "-h"]).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{stdout}\n{stderr}");

            // `git send-email -h` is a lightweight capability probe that works
            // both for real binaries and wrapper scripts without sending mail.
            if looks_like_send_email_help(&combined) {
                let version = probe_git_version(command)
                    .unwrap_or_else(|| "git send-email (version unavailable)".to_string());
                Probe::Available {
                    path: display_path.to_path_buf(),
                    version,
                    command: command_text,
                }
            } else {
                Probe::Broken {
                    path: display_path.to_path_buf(),
                    reason: normalize_output(combined.as_bytes())
                        .unwrap_or_else(|| format!("exit status {}", output.status)),
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Probe::Missing,
        Err(error) => Probe::Broken {
            path: display_path.to_path_buf(),
            reason: error.to_string(),
        },
    }
}

fn looks_like_send_email_help(output: &str) -> bool {
    let lowered = output.to_ascii_lowercase();
    lowered.contains("git send-email")
        && (lowered.contains("usage:")
            || lowered.contains("send patches")
            || lowered.contains("<file|directory>"))
        && !lowered.contains("not a git command")
}

fn probe_git_version<T>(command: T) -> Option<String>
where
    T: AsRef<std::ffi::OsStr>,
{
    let output = Command::new(command).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    normalize_output(&output.stdout).or_else(|| normalize_output(&output.stderr))
}

fn resolve_git_command(
    command_path: Option<&Path>,
) -> std::result::Result<ResolvedGitCommand, String> {
    let mut last_failure: Option<(PathBuf, String)> = None;
    for candidate in git_candidates(command_path) {
        match probe_send_email(&candidate) {
            Probe::Available { path, command, .. } => {
                return Ok(ResolvedGitCommand {
                    display_name: path.display().to_string(),
                    command,
                });
            }
            Probe::Broken { path, reason } => {
                last_failure = Some((path, reason));
            }
            Probe::Missing => {}
        }
    }

    if let Some((path, reason)) = last_failure {
        return Err(format!(
            "git send-email probe failed for {}: {}",
            path.display(),
            reason
        ));
    }

    Err("git send-email executable not found".to_string())
}

fn resolve_git_binary(
    command_path: Option<&Path>,
) -> std::result::Result<ResolvedGitCommand, String> {
    let mut last_failure: Option<(PathBuf, String)> = None;
    for candidate in git_candidates(command_path) {
        match probe_git_binary(&candidate) {
            Probe::Available { path, command, .. } => {
                return Ok(ResolvedGitCommand {
                    display_name: path.display().to_string(),
                    command,
                });
            }
            Probe::Broken { path, reason } => {
                last_failure = Some((path, reason));
            }
            Probe::Missing => {}
        }
    }

    if let Some((path, reason)) = last_failure {
        return Err(format!(
            "git probe failed for {}: {}",
            path.display(),
            reason
        ));
    }

    Err("git executable not found".to_string())
}

fn git_config_value(command: &str, args: &[&str]) -> std::result::Result<Option<String>, String> {
    let output = Command::new(command)
        .args(args)
        .output()
        .map_err(|error| format!("failed to run git {}: {error}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Ok(None);
        }
        return Err(format!("git {} failed: {stderr}", args.join(" ")));
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

fn parse_identity(value: &str) -> Option<ParsedIdentity> {
    let display = normalize_header_value(value);
    let email = extract_email_address(&display)?;
    Some(ParsedIdentity { display, email })
}

fn normalize_header_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_email_address(value: &str) -> Option<String> {
    if let Some((_, tail)) = value.rsplit_once('<')
        && let Some((email, _)) = tail.split_once('>')
    {
        let normalized = normalize_message_id(email);
        if !normalized.is_empty() {
            return Some(normalized);
        }
    }

    let candidate = value
        .split_whitespace()
        .find(|token| token.contains('@'))
        .map(normalize_message_id)?;
    if candidate.is_empty() {
        None
    } else {
        Some(candidate)
    }
}

fn normalize_message_id(value: &str) -> String {
    value
        .trim()
        .trim_matches('<')
        .trim_matches('>')
        .trim_matches('"')
        .trim_matches(',')
        .trim()
        .to_string()
}

fn render_message_file(request: &SendRequest, message_id: &str) -> String {
    // Emit a complete RFC822-style draft so `git send-email` handles transport
    // and SMTP concerns while Courier keeps ownership of message content.
    let mut lines = vec![
        format!("From: {}", request.from),
        format!("To: {}", request.to.join(", ")),
    ];
    if !request.cc.is_empty() {
        lines.push(format!("Cc: {}", request.cc.join(", ")));
    }
    lines.push(format!("Subject: {}", request.subject));
    lines.push(format!("Date: {}", Utc::now().to_rfc2822()));
    lines.push(format!("Message-ID: <{message_id}>"));
    lines.push(format!("In-Reply-To: <{}>", request.in_reply_to));
    if !request.references.is_empty() {
        lines.push(format!(
            "References: {}",
            request
                .references
                .iter()
                .map(|value| format!("<{}>", normalize_message_id(value)))
                .collect::<Vec<String>>()
                .join(" ")
        ));
    }
    lines.push("MIME-Version: 1.0".to_string());
    lines.push("Content-Type: text/plain; charset=UTF-8".to_string());
    lines.push("Content-Transfer-Encoding: 8bit".to_string());
    lines.push(String::new());
    lines.push(request.body.trim_end_matches('\n').to_string());
    lines.push(String::new());
    lines.join("\n")
}

fn build_send_email_args(request: &SendRequest, draft_path: &Path) -> Vec<String> {
    let mut args = vec![
        "send-email".to_string(),
        "--confirm=never".to_string(),
        "--quiet".to_string(),
        "--from".to_string(),
        request.from.clone(),
        "--subject".to_string(),
        request.subject.clone(),
        "--in-reply-to".to_string(),
        format!("<{}>", request.in_reply_to),
    ];

    for to in &request.to {
        args.push("--to".to_string());
        args.push(to.clone());
    }
    for cc in &request.cc {
        args.push("--cc".to_string());
        args.push(cc.clone());
    }

    args.push(draft_path.display().to_string());
    args
}

fn generate_message_id(from: &str) -> String {
    // Reuse the sender domain when possible so generated ids look like normal
    // outbound mail and are easier to correlate in archives and mail clients.
    let domain = extract_email_address(from)
        .and_then(|email| email.split('@').nth(1).map(ToOwned::to_owned))
        .unwrap_or_else(|| "localhost".to_string());
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("courier-{nonce}-{}@{domain}", std::process::id())
}

fn resolve_working_dir(runtime: &RuntimeConfig) -> PathBuf {
    // Prefer the kernel tree so git config, hooks, and relative includes match
    // the repository context where the user is reviewing patches.
    runtime
        .kernel_trees
        .iter()
        .find(|path| path.is_dir())
        .cloned()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| runtime.data_dir.clone())
}

fn render_command_line(command: &str, args: &[String]) -> String {
    let mut pieces = Vec::with_capacity(args.len() + 1);
    pieces.push(render_shell_token(command));
    for arg in args {
        pieces.push(render_shell_token(arg));
    }
    pieces.join(" ")
}

fn render_shell_token(token: &str) -> String {
    if token.is_empty() {
        return "''".to_string();
    }
    if token
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_-./:@".contains(character))
    {
        return token.to_string();
    }
    format!("'{}'", token.replace('\'', "'\\''"))
}

fn normalize_output(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

fn summarize_failure(exit_code: Option<i32>, stdout: &str, stderr: &str) -> Option<String> {
    // Prefer stderr because it usually carries the actionable SMTP or auth
    // error, then fall back to stdout for older scripts that log there.
    normalize_output(stderr.as_bytes())
        .or_else(|| normalize_output(stdout.as_bytes()))
        .or_else(|| exit_code.map(|code| format!("git send-email exited with {code}")))
}

fn failed_outcome(
    message_id: String,
    started_at: String,
    command_line: Option<String>,
    draft_path: Option<PathBuf>,
    error_summary: String,
) -> SendOutcome {
    SendOutcome {
        transport: "git-send-email".to_string(),
        message_id,
        command_line,
        draft_path,
        exit_code: None,
        timed_out: false,
        stdout: String::new(),
        stderr: String::new(),
        error_summary: Some(error_summary),
        started_at,
        finished_at: now_timestamp(),
        status: SendStatus::Failed,
    }
}

fn now_timestamp() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::infra::config::RuntimeConfig;

    use super::{
        GitSendEmailStatus, SendRequest, SendStatus, check_with_command_path,
        resolve_reply_identity_with_command_path, send_with_command_path,
    };

    fn temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("courier-sendmail-{label}-{nonce}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn test_runtime_in(root: &Path) -> RuntimeConfig {
        RuntimeConfig {
            config_path: root.join("config.toml"),
            data_dir: root.join("data"),
            database_path: root.join("data/courier.db"),
            raw_mail_dir: root.join("data/raw"),
            patch_dir: root.join("data/patches"),
            log_dir: root.join("data/logs"),
            b4_path: None,
            log_filter: "info".to_string(),
            source_mailbox: "linux-kernel".to_string(),
            imap: crate::infra::config::ImapConfig::default(),
            lore_base_url: "https://lore.kernel.org".to_string(),
            startup_sync: true,
            inbox_auto_sync_interval_secs:
                crate::infra::config::DEFAULT_INBOX_AUTO_SYNC_INTERVAL_SECS,
            kernel_trees: Vec::new(),
        }
    }

    fn write_fake_git(root: &Path, body: &str) -> PathBuf {
        let path = root.join("fake-git.sh");
        fs::write(&path, body).expect("write fake git");
        let mut permissions = fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("chmod");
        path
    }

    fn sample_request() -> SendRequest {
        SendRequest {
            mail_id: 3,
            thread_id: 9,
            from: "Courier Test <courier@example.com>".to_string(),
            to: vec!["maintainer@example.com".to_string()],
            cc: vec!["list@example.com".to_string()],
            subject: "Re: [PATCH] demo".to_string(),
            in_reply_to: "patch@example.com".to_string(),
            references: vec!["patch@example.com".to_string()],
            body: "reply body\n".to_string(),
            preview_confirmed_at: "2026-03-07T10:00:00Z".to_string(),
        }
    }

    #[test]
    fn check_reports_available_send_email() {
        let root = temp_dir("check-ok");
        let fake_git = write_fake_git(
            &root,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'git version 2.51.0'\n  exit 0\nfi\nif [ \"$1\" = \"send-email\" ] && [ \"$2\" = \"-h\" ]; then\n  echo 'usage: git send-email [<options>] <file|directory>...'\n  exit 129\nfi\nexit 1\n",
        );

        let check = check_with_command_path(Some(&fake_git));
        match check.status {
            GitSendEmailStatus::Available { version, .. } => {
                assert_eq!(version, "git version 2.51.0");
            }
            other => panic!("unexpected status: {other:?}"),
        }

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn check_accepts_single_line_send_email_help_banner() {
        let root = temp_dir("check-help-banner");
        let fake_git = write_fake_git(
            &root,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'git version 2.51.0'\n  exit 0\nfi\nif [ \"$1\" = \"send-email\" ] && [ \"$2\" = \"-h\" ]; then\n  echo 'git send-email [<options>] <file|directory>'\n  exit 129\nfi\nexit 1\n",
        );

        let check = check_with_command_path(Some(&fake_git));
        match check.status {
            GitSendEmailStatus::Available { version, .. } => {
                assert_eq!(version, "git version 2.51.0");
            }
            other => panic!("unexpected status: {other:?}"),
        }

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resolve_identity_prefers_sendemail_from() {
        let root = temp_dir("identity");
        let fake_git = write_fake_git(
            &root,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'git version 2.51.0'\n  exit 0\nfi\nif [ \"$1\" = \"config\" ] && [ \"$2\" = \"sendemail.from\" ]; then\n  echo 'Courier Test <courier@example.com>'\n  exit 0\nfi\nif [ \"$1\" = \"config\" ] && [ \"$2\" = \"user.email\" ]; then\n  echo 'fallback@example.com'\n  exit 0\nfi\nif [ \"$1\" = \"config\" ] && [ \"$2\" = \"user.name\" ]; then\n  echo 'Fallback User'\n  exit 0\nfi\nexit 1\n",
        );

        let identity = resolve_reply_identity_with_command_path(Some(&fake_git))
            .expect("resolve reply identity");
        assert_eq!(identity.display, "Courier Test <courier@example.com>");
        assert_eq!(identity.email, "courier@example.com");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn send_success_removes_draft_and_keeps_generated_message_id() {
        let root = temp_dir("send-success");
        let capture = root.join("captured.eml");
        let capture_args = root.join("captured-args.txt");
        let fake_git = write_fake_git(
            &root,
            &format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'git version 2.51.0'\n  exit 0\nfi\nif [ \"$1\" = \"send-email\" ] && [ \"$2\" = \"-h\" ]; then\n  echo 'usage: git send-email [<options>] <file|directory>...'\n  exit 129\nfi\nif [ \"$1\" = \"send-email\" ]; then\n  printf '%s\n' \"$@\" > '{}'\n  last=''\n  for arg in \"$@\"; do\n    last=\"$arg\"\n  done\n  cp \"$last\" '{}'\n  echo 'sent'\n  exit 0\nfi\nexit 1\n",
                capture_args.display(),
                capture.display()
            ),
        );
        let runtime = test_runtime_in(&root);

        let outcome = send_with_command_path(&runtime, &sample_request(), Some(&fake_git));
        assert_eq!(outcome.status, SendStatus::Sent);
        assert!(outcome.message_id.contains('@'));
        assert!(outcome.draft_path.is_none());

        let captured = fs::read_to_string(&capture).expect("read captured message");
        assert!(captured.contains("From: Courier Test <courier@example.com>"));
        assert!(captured.contains("To: maintainer@example.com"));
        assert!(captured.contains("Cc: list@example.com"));
        assert!(captured.contains("Subject: Re: [PATCH] demo"));
        assert!(captured.contains("In-Reply-To: <patch@example.com>"));
        assert!(captured.contains("References: <patch@example.com>"));
        assert!(captured.contains("reply body"));
        let captured_args = fs::read_to_string(&capture_args).expect("read captured args");
        assert!(captured_args.contains("--confirm=never"));
        assert!(captured_args.contains("--from"));
        assert!(captured_args.contains("Courier Test <courier@example.com>"));
        assert!(captured_args.contains("--to"));
        assert!(captured_args.contains("maintainer@example.com"));
        assert!(captured_args.contains("--cc"));
        assert!(captured_args.contains("list@example.com"));
        assert!(captured_args.contains("--subject"));
        assert!(captured_args.contains("Re: [PATCH] demo"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn send_failure_keeps_draft_and_summary() {
        let root = temp_dir("send-fail");
        let fake_git = write_fake_git(
            &root,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'git version 2.51.0'\n  exit 0\nfi\nif [ \"$1\" = \"send-email\" ] && [ \"$2\" = \"-h\" ]; then\n  echo 'usage: git send-email [<options>] <file|directory>...'\n  exit 129\nfi\nif [ \"$1\" = \"send-email\" ]; then\n  echo 'smtp auth failed' >&2\n  exit 1\nfi\nexit 1\n",
        );
        let runtime = test_runtime_in(&root);

        let outcome = send_with_command_path(&runtime, &sample_request(), Some(&fake_git));
        assert_eq!(outcome.status, SendStatus::Failed);
        assert_eq!(outcome.error_summary.as_deref(), Some("smtp auth failed"));
        assert!(
            outcome
                .draft_path
                .as_ref()
                .is_some_and(|path| path.exists())
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn send_prefers_configured_kernel_tree_as_working_dir() {
        let root = temp_dir("send-working-dir");
        let kernel_tree = root.join("linux");
        fs::create_dir_all(&kernel_tree).expect("create kernel tree");
        let current_dir = std::env::current_dir().expect("current dir");
        let capture_pwd = root.join("captured-pwd.txt");
        let fake_git = write_fake_git(
            &root,
            &format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'git version 2.51.0'\n  exit 0\nfi\nif [ \"$1\" = \"send-email\" ] && [ \"$2\" = \"-h\" ]; then\n  echo 'usage: git send-email [<options>] <file|directory>...'\n  exit 129\nfi\nif [ \"$1\" = \"send-email\" ]; then\n  pwd > '{}'\n  echo 'sent'\n  exit 0\nfi\nexit 1\n",
                capture_pwd.display()
            ),
        );
        let mut runtime = test_runtime_in(&root);
        runtime.kernel_trees = vec![kernel_tree.clone()];

        let outcome = send_with_command_path(&runtime, &sample_request(), Some(&fake_git));

        assert_eq!(outcome.status, SendStatus::Sent);
        let invoked_pwd = fs::read_to_string(&capture_pwd).expect("read captured pwd");
        assert_eq!(PathBuf::from(invoked_pwd.trim()), kernel_tree);
        assert_ne!(PathBuf::from(invoked_pwd.trim()), current_dir);

        let _ = fs::remove_dir_all(root);
    }
}
