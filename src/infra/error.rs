use std::error::Error as StdError;
use std::fmt;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, CourierError>;

type BoxError = Box<dyn StdError + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ErrorCode {
    ConfigRead,
    ConfigParse,
    LoggingInit,
    Io,
    Database,
    B4,
    Tui,
    Command,
    Imap,
    MailParse,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConfigRead => "E1001",
            Self::ConfigParse => "E1002",
            Self::LoggingInit => "E1003",
            Self::Io => "E1004",
            Self::Database => "E1005",
            Self::B4 => "E1006",
            Self::Tui => "E1007",
            Self::Command => "E1008",
            Self::Imap => "E1009",
            Self::MailParse => "E1010",
        }
    }

    pub fn exit_status(self) -> i32 {
        match self {
            Self::ConfigRead | Self::ConfigParse => 2,
            Self::LoggingInit => 3,
            Self::Io => 4,
            Self::Database => 5,
            Self::B4 => 6,
            Self::Tui => 7,
            Self::Command => 8,
            Self::Imap => 9,
            Self::MailParse => 10,
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Error)]
pub enum CourierError {
    #[error("{code}: {message}")]
    Message { code: ErrorCode, message: String },
    #[error("{code}: {message}: {source}")]
    WithSource {
        code: ErrorCode,
        message: String,
        #[source]
        source: BoxError,
    },
}

impl CourierError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Message {
            code,
            message: message.into(),
        }
    }

    pub fn with_source<E>(code: ErrorCode, message: impl Into<String>, source: E) -> Self
    where
        E: StdError + Send + Sync + 'static,
    {
        Self::WithSource {
            code,
            message: message.into(),
            source: Box::new(source),
        }
    }

    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Message { code, .. } | Self::WithSource { code, .. } => *code,
        }
    }
}
