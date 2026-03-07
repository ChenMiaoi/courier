use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

use crate::infra::error::{CourierError, ErrorCode, Result};

pub const CURRENT_SCHEMA_VERSION: i64 = 3;

const CREATE_SCHEMA_VERSION_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY,
    description TEXT NOT NULL,
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
"#;

struct Migration {
    version: i64,
    description: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "initial schema",
        sql: include_str!("../../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        description: "patch workflow schema",
        sql: include_str!("../../migrations/0002_patch_workflow.sql"),
    },
    Migration {
        version: 3,
        description: "reply send workflow schema",
        sql: include_str!("../../migrations/0003_reply_send_workflow.sql"),
    },
];

#[derive(Debug, Clone)]
pub struct DatabaseState {
    pub path: PathBuf,
    pub schema_version: i64,
    pub created: bool,
    pub applied_migrations: Vec<i64>,
}

pub fn initialize(path: &Path) -> Result<DatabaseState> {
    let created = !path.exists();
    let mut connection = Connection::open(path).map_err(|error| {
        CourierError::with_source(
            ErrorCode::Database,
            format!("failed to open sqlite database {}", path.display()),
            error,
        )
    })?;

    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|error| {
            CourierError::with_source(
                ErrorCode::Database,
                "failed to enable sqlite foreign key support",
                error,
            )
        })?;

    connection
        .execute_batch(CREATE_SCHEMA_VERSION_TABLE)
        .map_err(|error| {
            CourierError::with_source(
                ErrorCode::Database,
                "failed to create schema_version table",
                error,
            )
        })?;

    let current = current_version(&connection)?;
    let mut applied = Vec::new();

    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version > current)
    {
        let tx = connection.transaction().map_err(|error| {
            CourierError::with_source(
                ErrorCode::Database,
                "failed to open migration transaction",
                error,
            )
        })?;

        tx.execute_batch(migration.sql).map_err(|error| {
            CourierError::with_source(
                ErrorCode::Database,
                format!("failed to run migration {}", migration.version),
                error,
            )
        })?;

        tx.execute(
            "INSERT INTO schema_version(version, description) VALUES (?1, ?2)",
            params![migration.version, migration.description],
        )
        .map_err(|error| {
            CourierError::with_source(
                ErrorCode::Database,
                format!("failed to register migration {}", migration.version),
                error,
            )
        })?;

        tx.commit().map_err(|error| {
            CourierError::with_source(
                ErrorCode::Database,
                format!("failed to commit migration {}", migration.version),
                error,
            )
        })?;

        applied.push(migration.version);
    }

    let schema_version = current_version(&connection)?;

    Ok(DatabaseState {
        path: path.to_path_buf(),
        schema_version,
        created,
        applied_migrations: applied,
    })
}

fn current_version(connection: &Connection) -> Result<i64> {
    connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            CourierError::with_source(ErrorCode::Database, "failed to query schema version", error)
        })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::Connection;

    use super::{CURRENT_SCHEMA_VERSION, initialize};

    fn temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("courier-{label}-{nonce}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn initialize_runs_initial_migration() {
        let root = temp_dir("db-init");
        let db_path = root.join("courier.db");

        let state = initialize(&db_path).expect("initialize db");
        assert!(state.created);
        assert_eq!(state.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(state.applied_migrations, vec![1, 2, CURRENT_SCHEMA_VERSION]);

        let connection = Connection::open(&db_path).expect("open sqlite");
        let version = connection
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("query version");
        assert_eq!(version, CURRENT_SCHEMA_VERSION);

        let _ = fs::remove_dir_all(root);
    }
}
