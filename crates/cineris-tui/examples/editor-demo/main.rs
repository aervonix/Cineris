//! The editor demo: `cargo run --release -p cineris-tui --example editor-demo
//! --features demo -- rate=2000 auto`. See `app.rs`.

#![allow(
    clippy::print_stderr,
    reason = "a program of its own, which says why it could not start"
)]

mod app;

fn main() -> std::process::ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let options = match app::Options::parse(arguments.iter().map(String::as_str)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("editor-demo: {error}");
            return std::process::ExitCode::from(2);
        }
    };
    match app::run(&options) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("editor-demo: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
