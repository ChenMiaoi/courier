use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    match run(env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<OsString>) -> Result<(), XtaskError> {
    match ParseOutcome::parse(args)? {
        ParseOutcome::Help => {
            println!("{}", WikiCommand::help());
            Ok(())
        }
        ParseOutcome::Invocation(invocation) => invocation.run(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WikiCommand {
    Build,
    Check,
    Lint,
    Prepare,
    Serve,
}

impl WikiCommand {
    fn parse(value: &OsStr) -> Option<Self> {
        match value.to_str()? {
            "build" => Some(Self::Build),
            "check" => Some(Self::Check),
            "lint" => Some(Self::Lint),
            "prepare" => Some(Self::Prepare),
            "serve" => Some(Self::Serve),
            _ => None,
        }
    }

    fn help() -> &'static str {
        "\
Usage: cargo wiki <build|check|lint|prepare|serve> [args...]

build    Run ./scripts/wiki-site.sh build [mkdocs args...]
check    Run wiki lint and then wiki build
lint     Run ./scripts/wiki-lint.sh [autocorrect args...]
prepare  Run ./scripts/wiki-site.sh prepare
serve    Run ./scripts/wiki-site.sh serve [mkdocs args...]"
    }
}

#[derive(Debug, Eq, PartialEq)]
struct WikiInvocation {
    command: WikiCommand,
    extra_args: Vec<OsString>,
}

#[derive(Debug, Eq, PartialEq)]
enum ParseOutcome {
    Help,
    Invocation(WikiInvocation),
}

impl ParseOutcome {
    fn parse(args: Vec<OsString>) -> Result<Self, UsageError> {
        let mut args = args.into_iter();
        let Some(command_name) = args.next() else {
            return Err(UsageError::new("missing wiki command"));
        };

        if command_name == "help" || command_name == "--help" || command_name == "-h" {
            return Ok(Self::Help);
        }

        let Some(command) = WikiCommand::parse(&command_name) else {
            let invalid_name = command_name.to_string_lossy().into_owned();
            return Err(UsageError::new(format!(
                "unknown wiki command '{invalid_name}'"
            )));
        };

        Ok(Self::Invocation(WikiInvocation {
            command,
            extra_args: args.collect(),
        }))
    }
}

impl WikiInvocation {
    fn run(self) -> Result<(), XtaskError> {
        let repo_root = repo_root()?;

        match self.command {
            WikiCommand::Lint => run_script(
                repo_root.join("scripts/wiki-lint.sh"),
                self.extra_args,
                &repo_root,
            ),
            WikiCommand::Prepare => run_script_with_prefix(
                "scripts/wiki-site.sh",
                ["prepare"],
                self.extra_args,
                &repo_root,
            ),
            WikiCommand::Build => run_script_with_prefix(
                "scripts/wiki-site.sh",
                ["build"],
                self.extra_args,
                &repo_root,
            ),
            WikiCommand::Serve => run_script_with_prefix(
                "scripts/wiki-site.sh",
                ["serve"],
                self.extra_args,
                &repo_root,
            ),
            WikiCommand::Check => {
                if !self.extra_args.is_empty() {
                    return Err(XtaskError::Usage(UsageError::new(
                        "cargo wiki check does not accept extra arguments",
                    )));
                }

                run_script(repo_root.join("scripts/wiki-lint.sh"), [], &repo_root)?;
                run_script_with_prefix("scripts/wiki-site.sh", ["build"], [], &repo_root)
            }
        }
    }
}

fn run_script_with_prefix<const N: usize>(
    script_relative_path: &str,
    prefix_args: [&str; N],
    extra_args: impl IntoIterator<Item = OsString>,
    repo_root: &Path,
) -> Result<(), XtaskError> {
    let args = prefix_args
        .into_iter()
        .map(OsString::from)
        .chain(extra_args)
        .collect::<Vec<_>>();
    run_script(repo_root.join(script_relative_path), args, repo_root)
}

fn run_script(
    script_path: PathBuf,
    args: impl IntoIterator<Item = OsString>,
    repo_root: &Path,
) -> Result<(), XtaskError> {
    let status = Command::new(&script_path)
        .args(args)
        .current_dir(repo_root)
        .status()
        .map_err(|source| XtaskError::CommandSpawn {
            command: script_path.display().to_string(),
            source,
        })?;

    if status.success() {
        return Ok(());
    }

    Err(XtaskError::CommandFailed {
        command: script_path.display().to_string(),
        status,
    })
}

fn repo_root() -> Result<PathBuf, XtaskError> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().map(Path::to_path_buf).ok_or_else(|| {
        XtaskError::RepoRoot(format!(
            "xtask manifest directory '{}' does not have a repository root parent",
            manifest_dir.display()
        ))
    })
}

#[derive(Debug)]
enum XtaskError {
    CommandFailed {
        command: String,
        status: std::process::ExitStatus,
    },
    CommandSpawn {
        command: String,
        source: std::io::Error,
    },
    RepoRoot(String),
    Usage(UsageError),
}

impl fmt::Display for XtaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandFailed { command, status } => {
                write!(f, "command '{command}' exited with status {status}")
            }
            Self::CommandSpawn { command, source } => {
                write!(f, "failed to run '{command}': {source}")
            }
            Self::RepoRoot(message) => f.write_str(message),
            Self::Usage(error) => write!(f, "{error}"),
        }
    }
}

impl From<UsageError> for XtaskError {
    fn from(value: UsageError) -> Self {
        Self::Usage(value)
    }
}

#[derive(Debug, Eq, PartialEq)]
struct UsageError {
    message: String,
}

impl UsageError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\n\n{}", self.message, WikiCommand::help())
    }
}

#[cfg(test)]
mod tests {
    use super::{ParseOutcome, UsageError, WikiCommand};
    use std::ffi::OsString;

    #[test]
    fn parses_build_command_with_extra_args() {
        let outcome = ParseOutcome::parse(vec![
            OsString::from("build"),
            OsString::from("--strict"),
            OsString::from("--dirtyreload"),
        ])
        .unwrap();
        let ParseOutcome::Invocation(invocation) = outcome else {
            panic!("expected invocation parse outcome");
        };

        assert_eq!(invocation.command, WikiCommand::Build);
        assert_eq!(
            invocation.extra_args,
            vec![OsString::from("--strict"), OsString::from("--dirtyreload")]
        );
    }

    #[test]
    fn rejects_missing_command() {
        let error = ParseOutcome::parse(Vec::new()).unwrap_err();
        assert_eq!(error, UsageError::new("missing wiki command"));
    }

    #[test]
    fn rejects_unknown_command() {
        let error = ParseOutcome::parse(vec![OsString::from("deploy")]).unwrap_err();
        assert_eq!(error, UsageError::new("unknown wiki command 'deploy'"));
    }

    #[test]
    fn parses_help_without_error() {
        let outcome = ParseOutcome::parse(vec![OsString::from("--help")]).unwrap();
        assert_eq!(outcome, ParseOutcome::Help);
    }
}
