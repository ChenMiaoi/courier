//! User-interface entrypoints.
//!
//! The application layer only depends on coarse-grained UI actions such as
//! exit or restart; the ratatui implementation details stay behind this module.

pub mod tui;

pub use tui::{TuiAction, run};
