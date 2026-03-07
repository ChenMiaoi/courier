//! Discovery and execution helpers for the external `b4` tool.
//!
//! Higher layers care about patch download/apply outcomes, not about PATH
//! probing or timeout loops, so this module isolates those process-management
//! details behind a small result type.

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::infra::error::{CourierError, ErrorCode, Result};

#[derive(Debug, Clone)]
pub struct B4Check {
    pub status: B4Status,
}

#[derive(Debug, Clone)]
pub enum B4Status {
    Available { path: PathBuf, version: String },
    Broken { path: PathBuf, reason: String },
    Missing,
}

#[derive(Debug, Clone)]
pub struct B4CommandResult {
    pub command_line: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
}

enum Candidate {
    Path(PathBuf),
    Program(String),
}

#[derive(Debug, Clone)]
struct ResolvedCommand {
    command: String,
    display_path: PathBuf,
}

pub fn check(configured_path: Option<&Path>) -> B4Check {
    let candidates = candidates(configured_path);
    let mut last_failure: Option<(PathBuf, String)> = None;

    for candidate in candidates {
        match probe(&candidate) {
            Probe::Available { path, version, .. } => {
                return B4Check {
                    status: B4Status::Available { path, version },
                };
            }
            Probe::Missing => continue,
            Probe::Broken { path, reason } => {
                last_failure = Some((path, reason));
            }
        }
    }

    if let Some((path, reason)) = last_failure {
        B4Check {
            status: B4Status::Broken { path, reason },
        }
    } else {
        B4Check {
            status: B4Status::Missing,
        }
    }
}

pub fn run(
    configured_path: Option<&Path>,
    subcommand: &str,
    args: &[String],
    timeout: Duration,
    working_dir: Option<&Path>,
) -> Result<B4CommandResult> {
    let resolved = resolve_command(configured_path)?;

    let mut command = Command::new(&resolved.command);
    if let Some(working_dir) = working_dir {
        command.current_dir(working_dir);
    }
    command.arg(subcommand);
    for arg in args {
        command.arg(arg);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    let command_line = render_command_line(&resolved.command, subcommand, args);
    let mut child = command.spawn().map_err(|error| {
        CourierError::with_source(
            ErrorCode::B4,
            format!(
                "failed to spawn b4 command '{}' ({})",
                command_line,
                resolved.display_path.display()
            ),
            error,
        )
    })?;

    let started_at = Instant::now();
    let mut timed_out = false;

    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                // Poll with an explicit timeout so a wedged `b4` process cannot
                // freeze the TUI or patch workflow indefinitely.
                if started_at.elapsed() >= timeout {
                    timed_out = true;
                    let _ = child.kill();
                    break;
                }
                thread::sleep(Duration::from_millis(30));
            }
            Err(error) => {
                return Err(CourierError::with_source(
                    ErrorCode::B4,
                    format!("failed while waiting for b4 command '{}'", command_line),
                    error,
                ));
            }
        }
    }

    let output = child.wait_with_output().map_err(|error| {
        CourierError::with_source(
            ErrorCode::B4,
            format!("failed to collect output for b4 command '{}'", command_line),
            error,
        )
    })?;

    Ok(B4CommandResult {
        command_line,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code(),
        timed_out,
    })
}

fn candidates(configured_path: Option<&Path>) -> Vec<Candidate> {
    let mut values = Vec::new();

    // Discovery order is from most explicit to most implicit so user config
    // wins over environment hints and PATH-based fallback.
    if let Some(path) = configured_path {
        values.push(Candidate::Path(path.to_path_buf()));
    }

    if let Ok(env_path) = env::var("COURIER_B4_PATH") {
        let path = PathBuf::from(env_path);
        if !path.as_os_str().is_empty() {
            values.push(Candidate::Path(path));
        }
    }

    if let Ok(cwd) = env::current_dir() {
        values.push(Candidate::Path(cwd.join("vendor/b4/b4.sh")));
    }

    values.push(Candidate::Program("b4".to_string()));

    values
}

enum Probe {
    Available {
        command: String,
        path: PathBuf,
        version: String,
    },
    Broken {
        path: PathBuf,
        reason: String,
    },
    Missing,
}

fn probe(candidate: &Candidate) -> Probe {
    match candidate {
        Candidate::Path(path) => {
            if !path.exists() {
                return Probe::Missing;
            }
            run_probe(path, path, path.display().to_string())
        }
        Candidate::Program(program) => {
            let label = PathBuf::from(format!("{program} (PATH)"));
            run_probe(program, &label, program.clone())
        }
    }
}

fn run_probe<T>(command: T, label: &Path, command_value: String) -> Probe
where
    T: AsRef<std::ffi::OsStr>,
{
    match Command::new(command).arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = normalize_output(&output.stdout)
                .or_else(|| normalize_output(&output.stderr))
                .unwrap_or_else(|| "unknown".to_string());

            Probe::Available {
                command: command_value,
                path: label.to_path_buf(),
                version,
            }
        }
        Ok(output) => {
            let reason = normalize_output(&output.stderr)
                .or_else(|| normalize_output(&output.stdout))
                .unwrap_or_else(|| format!("exit status {}", output.status));

            Probe::Broken {
                path: label.to_path_buf(),
                reason,
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Probe::Missing,
        Err(error) => Probe::Broken {
            path: label.to_path_buf(),
            reason: error.to_string(),
        },
    }
}

fn resolve_command(configured_path: Option<&Path>) -> Result<ResolvedCommand> {
    let mut last_failure: Option<(PathBuf, String)> = None;
    for candidate in candidates(configured_path) {
        match probe(&candidate) {
            Probe::Available { command, path, .. } => {
                return Ok(ResolvedCommand {
                    command,
                    display_path: path,
                });
            }
            Probe::Broken { path, reason } => {
                last_failure = Some((path, reason));
            }
            Probe::Missing => {}
        }
    }

    if let Some((path, reason)) = last_failure {
        // Report the last broken candidate explicitly because "not found" would
        // hide a misconfigured path that the user can actually fix.
        return Err(CourierError::new(
            ErrorCode::B4,
            format!("b4 executable '{}' is broken: {}", path.display(), reason),
        ));
    }

    Err(CourierError::new(
        ErrorCode::B4,
        "b4 executable not found (checked config path, COURIER_B4_PATH, vendor/b4/b4.sh and PATH)",
    ))
}

fn render_command_line(command: &str, subcommand: &str, args: &[String]) -> String {
    let mut pieces = Vec::with_capacity(2 + args.len());
    pieces.push(render_shell_token(command));
    pieces.push(render_shell_token(subcommand));
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
