CREATE TABLE IF NOT EXISTS reply_send (
    id INTEGER PRIMARY KEY,
    thread_id INTEGER NOT NULL,
    mail_id INTEGER NOT NULL,
    transport TEXT NOT NULL,
    message_id TEXT NOT NULL,
    from_addr TEXT NOT NULL,
    to_addrs TEXT NOT NULL DEFAULT '',
    cc_addrs TEXT NOT NULL DEFAULT '',
    subject TEXT NOT NULL DEFAULT '',
    preview_confirmed_at TEXT NOT NULL,
    status TEXT NOT NULL,
    command TEXT,
    draft_path TEXT,
    exit_code INTEGER,
    timed_out INTEGER NOT NULL DEFAULT 0,
    error_summary TEXT,
    stdout TEXT,
    stderr TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    FOREIGN KEY (thread_id) REFERENCES thread(id) ON DELETE CASCADE,
    FOREIGN KEY (mail_id) REFERENCES mail(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_reply_send_mail
ON reply_send (mail_id, id DESC);

CREATE INDEX IF NOT EXISTS idx_reply_send_thread
ON reply_send (thread_id, id DESC);

CREATE INDEX IF NOT EXISTS idx_reply_send_status
ON reply_send (status, id DESC);
