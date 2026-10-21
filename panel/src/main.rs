//! The panel binary.

use std::net::IpAddr;
use std::process::ExitCode;
use std::sync::Arc;

use hello_server::port;
use tiny_http::Server;

use panel::app::{self, App};
use panel::config::Config;
use panel::db::Database;
use panel::instances::InstanceManager;
use panel::web::WebRequest;

/// Command line options.
struct Options {
    config: Config,
    help: bool,
    version: bool,
}

impl Options {
    /// Parse the command line.
    fn parse(args: Vec<String>) -> Result<Options, String> {
        let mut config = Config::from_env();
        let mut help = false;
        let mut version = false;

        let mut index = 0;
        while index < args.len() {
            let arg = args[index].clone();
            index += 1;

            let (flag, inline) = match arg.split_once('=') {
                Some((flag, value)) if flag.starts_with('-') => {
                    (flag.to_string(), Some(value.to_string()))
                }
                _ if arg.len() > 2 && arg.starts_with('-') && !arg.starts_with("--") => {
                    (arg[..2].to_string(), Some(arg[2..].to_string()))
                }
                _ => (arg.clone(), None),
            };

            let mut take_value = |flag: &str| -> Result<String, String> {
                if let Some(value) = inline.clone() {
                    return Ok(value);
                }
                let value = args
                    .get(index)
                    .cloned()
                    .ok_or_else(|| format!("missing value for `{flag}`"))?;
                index += 1;
                Ok(value)
            };

            match flag.as_str() {
                "-h" | "--help" => help = true,
                "-V" | "--version" => version = true,
                "-p" | "--port" => {
                    let value = take_value(&flag)?;
                    config.port = value
                        .trim()
                        .parse::<u16>()
                        .map_err(|_| format!("invalid port `{value}`"))?;
                }
                "-b" | "--host" => {
                    let value = take_value(&flag)?;
                    config.host = value
                        .trim()
                        .parse::<IpAddr>()
                        .map_err(|_| format!("invalid host `{value}`"))?;
                }
                "-d" | "--data-dir" => {
                    let value = take_value(&flag)?;
                    let dir = std::path::PathBuf::from(value);
                    config.database = dir.join("panel.db");
                    config.data_dir = dir.join("instances");
                }
                other => return Err(format!("unknown argument `{other}`")),
            }
        }

        Ok(Options {
            config,
            help,
            version,
        })
    }
}

/// The text printed for `--help`.
fn usage() -> String {
    format!(
        "noobs-panel {version} - a simple panel for managing web server instances

USAGE:
    noobs-panel [OPTIONS]

OPTIONS:
    -p, --port <PORT>       Port the panel listens on [default: {default_port}]
    -b, --host <HOST>       Address the panel binds to [default: {default_host}]
    -d, --data-dir <DIR>    Where the database and instance data are stored
    -h, --help              Print this help and exit
    -V, --version           Print version information and exit

ENVIRONMENT:
    NOOBS_PANEL_PORT            Same as --port
    NOOBS_PANEL_HOST            Same as --host
    NOOBS_PANEL_DATA            Same as --data-dir
    NOOBS_PANEL_INSTANCE_PORTS  Port range for instances, as START-END
    NOOBS_INSTANCE_BIN          Path of the binary used to run instances
",
        version = env!("CARGO_PKG_VERSION"),
        default_port = panel::config::DEFAULT_PORT,
        default_host = "127.0.0.1",
    )
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match Options::parse(args) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("error: {message}");
            eprintln!();
            eprint!("{}", usage());
            return ExitCode::from(2);
        }
    };

    if options.help {
        print!("{}", usage());
        return ExitCode::SUCCESS;
    }
    if options.version {
        println!("noobs-panel {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    let config = options.config;

    let db = match Database::open(&config.database) {
        Ok(db) => db,
        Err(err) => {
            eprintln!("error: cannot open the database: {err}");
            return ExitCode::FAILURE;
        }
    };

    let manager = Arc::new(InstanceManager::new());

    // Instances that were running when the panel stopped are no longer
    // running, so the records are corrected before anything else happens.
    if let Ok(instances) = db.list_instances(None) {
        if let Err(err) = panel::instances::reconcile(&db, &instances) {
            eprintln!("warning: cannot reconcile instance state: {err}");
        }
    }

    // Sessions and csrf tokens that have expired are of no use any more.
    let _ = db.prune_sessions(panel::app::now());
    let _ = db.prune_csrf_tokens(panel::app::now());

    let bound = match port::bind_with_fallback(config.host, config.port) {
        Ok(bound) => bound,
        Err(err) => {
            eprintln!("error: cannot bind a port: {err}");
            return ExitCode::FAILURE;
        }
    };

    let url = port::display_url(config.host, bound.port);

    let app = App {
        config: config.clone(),
        db,
        manager: manager.clone(),
    };

    let is_fallback = bound.is_fallback();
    let requested_port = bound.requested;

    let server = match Server::from_listener(bound.listener, None) {
        Ok(server) => server,
        Err(err) => {
            eprintln!("error: cannot start the server: {err}");
            return ExitCode::FAILURE;
        }
    };

    println!("  noobs-panel is running");
    println!("  -----------------------");
    println!("  URL:        {url}");
    println!("  Port:       {}", bound.port);
    println!("  Data dir:   {}", config.database.display());
    if is_fallback {
        println!(
            "  Note:       port {} was busy, using {} instead",
            requested_port, bound.port
        );
    }
    if manager.instance_binary().is_none() {
        println!("  Warning:    the hello-server binary was not found, instances cannot start.");
    }
    println!("  Stop with:  Ctrl+C");
    println!();

    for request in server.incoming_requests() {
        let request = WebRequest::new(request);
        let response = app::handle(&app, &request);
        let _ = request.raw.respond(response);
    }

    // Any instance that is still running is stopped together with the panel.
    manager.stop_all();

    ExitCode::SUCCESS
}
