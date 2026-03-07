//! Process entrypoint.
//!
//! Keep `main` intentionally small so every real startup policy, config lookup,
//! and exit-path decision lives in the application layer instead of getting
//! duplicated between CLI and TUI modes.

mod app;
mod domain;
mod infra;
mod ui;

fn main() {
    if let Err(error) = app::run() {
        eprintln!("error({}): {}", error.code(), error);
        std::process::exit(error.code().exit_status());
    }
}
