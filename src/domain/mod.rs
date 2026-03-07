//! Domain models and static subscription data.
//!
//! The domain layer stays intentionally small: it defines Courier's core
//! concepts without coupling them to SQLite, IMAP, or ratatui details.

pub mod models;
pub mod subscriptions;
