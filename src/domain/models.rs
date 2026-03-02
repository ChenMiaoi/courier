#![allow(dead_code)]

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    pub message_id: String,
    pub subject: String,
    pub from_addr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadSummary {
    pub root_message_id: String,
    pub title: String,
    pub message_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchSeriesStatus {
    New,
    Reviewing,
    Applied,
    Failed,
    Conflict,
}
