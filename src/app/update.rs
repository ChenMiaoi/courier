//! Self-update orchestration.
//!
//! The first supported update path is intentionally conservative: delegate to
//! Cargo's crates.io installer instead of replacing binaries directly.

use std::io;
use std::process::Command as ProcessCommand;

use crate::infra::error::{CriewError, ErrorCode, Result};

const CARGO_PROGRAM: &str = "cargo";
const CARGO_INSTALL_ARGS: &[&str] = &["install", "--locked", "criew", "--force"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateRequest {
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSummary {
    pub command_line: String,
    pub status: UpdateStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateStatus {
    DryRun,
    Updated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UpdateCommandOutput {
    is_success: bool,
    exit_code: Option<i32>,
}

pub fn run(request: UpdateRequest) -> Result<UpdateSummary> {
    run_with_runner(request, run_cargo_install)
}

fn run_with_runner<F>(request: UpdateRequest, runner: F) -> Result<UpdateSummary>
where
    F: FnOnce(&str, &[&str]) -> io::Result<UpdateCommandOutput>,
{
    let command_line = update_command_line();
    if request.dry_run {
        return Ok(UpdateSummary {
            command_line,
            status: UpdateStatus::DryRun,
        });
    }

    let output = runner(CARGO_PROGRAM, CARGO_INSTALL_ARGS).map_err(|error| {
        CriewError::with_source(
            ErrorCode::Command,
            format!("failed to run update command '{command_line}'"),
            error,
        )
    })?;

    if !output.is_success {
        return Err(CriewError::new(
            ErrorCode::Command,
            format!(
                "update command '{}' failed with {}",
                command_line,
                exit_status_label(output.exit_code)
            ),
        ));
    }

    Ok(UpdateSummary {
        command_line,
        status: UpdateStatus::Updated,
    })
}

fn run_cargo_install(program: &str, args: &[&str]) -> io::Result<UpdateCommandOutput> {
    let status = ProcessCommand::new(program).args(args).status()?;
    Ok(UpdateCommandOutput {
        is_success: status.success(),
        exit_code: status.code(),
    })
}

fn update_command_line() -> String {
    std::iter::once(CARGO_PROGRAM)
        .chain(CARGO_INSTALL_ARGS.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
}

fn exit_status_label(exit_code: Option<i32>) -> String {
    exit_code
        .map(|code| format!("exit status {code}"))
        .unwrap_or_else(|| "signal termination".to_string())
}

#[cfg(test)]
mod tests {
    use std::io;

    use crate::infra::error::ErrorCode;

    use super::{
        UpdateCommandOutput, UpdateRequest, UpdateStatus, exit_status_label, run_with_runner,
    };

    #[test]
    fn dry_run_reports_command_without_running_cargo() {
        let summary = run_with_runner(UpdateRequest { dry_run: true }, |_, _| {
            panic!("dry run should not invoke cargo")
        })
        .expect("dry-run update");

        assert_eq!(summary.command_line, "cargo install --locked criew --force");
        assert_eq!(summary.status, UpdateStatus::DryRun);
    }

    #[test]
    fn successful_update_invokes_cargo_install() {
        let summary = run_with_runner(UpdateRequest { dry_run: false }, |program, args| {
            assert_eq!(program, "cargo");
            assert_eq!(args, ["install", "--locked", "criew", "--force"]);
            Ok(UpdateCommandOutput {
                is_success: true,
                exit_code: Some(0),
            })
        })
        .expect("successful update");

        assert_eq!(summary.status, UpdateStatus::Updated);
    }

    #[test]
    fn update_reports_spawn_failure() {
        let error = run_with_runner(UpdateRequest { dry_run: false }, |_, _| {
            Err(io::Error::new(io::ErrorKind::NotFound, "cargo missing"))
        })
        .expect_err("missing cargo should fail");

        assert_eq!(error.code(), ErrorCode::Command);
        assert!(error.to_string().contains("failed to run update command"));
    }

    #[test]
    fn update_reports_nonzero_status() {
        let error = run_with_runner(UpdateRequest { dry_run: false }, |_, _| {
            Ok(UpdateCommandOutput {
                is_success: false,
                exit_code: Some(101),
            })
        })
        .expect_err("failed cargo install should fail");

        assert_eq!(error.code(), ErrorCode::Command);
        assert!(error.to_string().contains("failed with exit status 101"));
    }

    #[test]
    fn exit_status_label_handles_signal_termination() {
        assert_eq!(exit_status_label(None), "signal termination");
    }
}
