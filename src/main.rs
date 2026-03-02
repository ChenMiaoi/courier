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
