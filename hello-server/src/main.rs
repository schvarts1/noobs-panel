use std::process::ExitCode;

use hello_server::args::{usage, Args, ArgsError};
use hello_server::port;
use hello_server::server;

fn main() -> ExitCode {
    let args = match Args::from_env() {
        Ok(args) => args,
        Err(ArgsError::HelpRequested) => {
            print!("{}", usage());
            return ExitCode::SUCCESS;
        }
        Err(ArgsError::VersionRequested) => {
            println!("hello-server {}", env!("CARGO_PKG_VERSION"));
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

    if let Err(err) = server::serve_with_root(bound, args.host, args.root.clone()) {
        eprintln!("error: server failed: {err}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
