//! `ghd`: the native binary bundled into the wheel.
//!
//! `ghd daemon` runs the long-lived daemon (`ghd-daemon`, behind the
//! amalgam's `daemon` feature); every other subcommand is a short-lived
//! client call (`ghd-core`'s CLI).

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("daemon") => ghd::run_daemon().map(|()| ExitCode::SUCCESS),
        _ => ghd::cli::run(&args),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("ghd: {e}");
            ExitCode::FAILURE
        }
    }
}
