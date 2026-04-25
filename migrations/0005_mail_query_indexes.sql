CREATE INDEX IF NOT EXISTS idx_mail_mailbox_expunged
ON mail (imap_mailbox, is_expunged);

CREATE INDEX IF NOT EXISTS idx_mail_in_reply_to
ON mail (in_reply_to);

CREATE INDEX IF NOT EXISTS idx_mail_expunged
ON mail (is_expunged);
