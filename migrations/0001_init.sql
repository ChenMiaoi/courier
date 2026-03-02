CREATE TABLE IF NOT EXISTS mail (
    id INTEGER PRIMARY KEY,
    message_id TEXT NOT NULL UNIQUE,
    subject TEXT NOT NULL DEFAULT '',
    from_addr TEXT NOT NULL DEFAULT '',
    date TEXT,
    raw_path TEXT,
    in_reply_to TEXT,
    list_id TEXT,
    flags TEXT,
    imap_mailbox TEXT,
    imap_uid INTEGER,
    modseq INTEGER,
    is_expunged INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_mail_mailbox_uid
ON mail (imap_mailbox, imap_uid)
WHERE imap_mailbox IS NOT NULL AND imap_uid IS NOT NULL;

CREATE TABLE IF NOT EXISTS mail_ref (
    mail_id INTEGER NOT NULL,
    ref_message_id TEXT NOT NULL,
    ord INTEGER NOT NULL,
    PRIMARY KEY (mail_id, ord),
    FOREIGN KEY (mail_id) REFERENCES mail(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_mail_ref_ref_message_id
ON mail_ref (ref_message_id);

CREATE TABLE IF NOT EXISTS thread (
    id INTEGER PRIMARY KEY,
    root_mail_id INTEGER,
    subject_norm TEXT NOT NULL DEFAULT '',
    last_activity_at TEXT,
    message_count INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (root_mail_id) REFERENCES mail(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS thread_node (
    mail_id INTEGER PRIMARY KEY,
    thread_id INTEGER NOT NULL,
    parent_mail_id INTEGER,
    root_mail_id INTEGER,
    depth INTEGER NOT NULL DEFAULT 0,
    sort_ts TEXT,
    FOREIGN KEY (mail_id) REFERENCES mail(id) ON DELETE CASCADE,
    FOREIGN KEY (thread_id) REFERENCES thread(id) ON DELETE CASCADE,
    FOREIGN KEY (parent_mail_id) REFERENCES mail(id) ON DELETE SET NULL,
    FOREIGN KEY (root_mail_id) REFERENCES mail(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_thread_node_thread_sort
ON thread_node (thread_id, sort_ts);

CREATE INDEX IF NOT EXISTS idx_thread_node_parent
ON thread_node (parent_mail_id);

CREATE TABLE IF NOT EXISTS imap_mailbox_state (
    mailbox TEXT PRIMARY KEY,
    uidvalidity INTEGER NOT NULL,
    last_seen_uid INTEGER NOT NULL DEFAULT 0,
    highest_modseq INTEGER,
    synced_at TEXT
);
