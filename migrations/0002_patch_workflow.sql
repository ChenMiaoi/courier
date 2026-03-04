CREATE TABLE IF NOT EXISTS patch_series (
    id INTEGER PRIMARY KEY,
    mailbox TEXT NOT NULL,
    thread_id INTEGER NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    expected_total INTEGER NOT NULL DEFAULT 0,
    author TEXT NOT NULL DEFAULT '',
    subject TEXT NOT NULL DEFAULT '',
    anchor_message_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'new',
    integrity TEXT NOT NULL DEFAULT 'invalid',
    missing_seq TEXT NOT NULL DEFAULT '',
    duplicate_seq TEXT NOT NULL DEFAULT '',
    out_of_order INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    last_command TEXT,
    last_exit_code INTEGER,
    last_stdout TEXT,
    last_stderr TEXT,
    exported_path TEXT,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (mailbox, thread_id, version)
);

CREATE INDEX IF NOT EXISTS idx_patch_series_mailbox_status
ON patch_series (mailbox, status);

CREATE INDEX IF NOT EXISTS idx_patch_series_thread
ON patch_series (thread_id, version DESC);

CREATE TABLE IF NOT EXISTS patch_series_item (
    id INTEGER PRIMARY KEY,
    series_id INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    total INTEGER NOT NULL,
    mail_id INTEGER NOT NULL,
    message_id TEXT NOT NULL,
    subject TEXT NOT NULL,
    raw_path TEXT,
    sort_ord INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (series_id) REFERENCES patch_series(id) ON DELETE CASCADE,
    FOREIGN KEY (mail_id) REFERENCES mail(id) ON DELETE CASCADE,
    UNIQUE (series_id, seq, message_id)
);

CREATE INDEX IF NOT EXISTS idx_patch_series_item_series_seq
ON patch_series_item (series_id, seq);

CREATE TABLE IF NOT EXISTS patch_series_run (
    id INTEGER PRIMARY KEY,
    series_id INTEGER NOT NULL,
    action TEXT NOT NULL,
    command TEXT NOT NULL,
    status TEXT NOT NULL,
    exit_code INTEGER,
    timed_out INTEGER NOT NULL DEFAULT 0,
    summary TEXT,
    stdout TEXT,
    stderr TEXT,
    output_path TEXT,
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    finished_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (series_id) REFERENCES patch_series(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_patch_series_run_series
ON patch_series_run (series_id, id DESC);
