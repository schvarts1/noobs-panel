//! The benchmarker binary.
//!
//! It is the second preset of the panel. Where the hello server only proves
//! that an instance runs, the benchmarker proves that the limits of the
//! instance are real.

use std::process::ExitCode;

use hello_server::args::{usage, Args, ArgsError};
use hello_server::port;

mod limits;
mod server;
mod stress;

fn main() -> ExitCode {
    let args = match Args::from_env() {
        Ok(args) => args,
        Err(ArgsError::HelpRequested) => {
            print!("{}", usage());
            return ExitCode::SUCCESS;
        }
        Err(ArgsError::VersionRequested) => {
            println!("benchmarker {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(err) => {
            eprintln!("error: {err}");
            eprintln!();
            eprint!("{}", usage());
            return ExitCode::from(err.exit_code() as u8);
        }
    };

    // Bind the preferred port, or the nearest free one when it is busy.
    let bound = match port::bind_with_fallback(args.host, args.desired_port) {
        Ok(bound) => bound,
        Err(err) => {
            eprintln!("error: could not bind a port: {err}");
            return ExitCode::FAILURE;
        }
    };

    // The files the disk test writes are kept out of the way of the panel.
    let directory = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    if let Err(err) = server::serve(bound, args.host, directory) {
        eprintln!("error: server failed: {err}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
