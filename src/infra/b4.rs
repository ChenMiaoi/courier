use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

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

enum Candidate {
    Path(PathBuf),
    Program(String),
}

pub fn check(configured_path: Option<&Path>) -> B4Check {
    let candidates = candidates(configured_path);
    let mut last_failure: Option<(PathBuf, String)> = None;

    for candidate in candidates {
        match probe(&candidate) {
            Probe::Available { path, version } => {
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

fn candidates(configured_path: Option<&Path>) -> Vec<Candidate> {
    let mut values = Vec::new();

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
    Available { path: PathBuf, version: String },
    Broken { path: PathBuf, reason: String },
    Missing,
}

fn probe(candidate: &Candidate) -> Probe {
    match candidate {
        Candidate::Path(path) => {
            if !path.exists() {
                return Probe::Missing;
            }
            run_probe(path, path)
        }
        Candidate::Program(program) => {
            let label = PathBuf::from(format!("{program} (PATH)"));
            run_probe(program, &label)
        }
    }
}

fn run_probe<T>(command: T, label: &Path) -> Probe
where
    T: AsRef<std::ffi::OsStr>,
{
    match Command::new(command).arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = normalize_output(&output.stdout)
                .or_else(|| normalize_output(&output.stderr))
                .unwrap_or_else(|| "unknown".to_string());

            Probe::Available {
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

fn normalize_output(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}
