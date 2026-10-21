//! Command line argument parsing for the hello world server.

use std::env;
use std::fmt;
use std::net::IpAddr;
use std::path::PathBuf;

/// Default port used when nothing else is specified.
pub const DEFAULT_PORT: u16 = 8000;

/// Default bind address: all interfaces.
pub const DEFAULT_HOST: &str = "0.0.0.0";

/// Options the server was started with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// Address the listening socket is bound to.
    pub host: IpAddr,
    /// Port we would like to use. It may end up being changed by the
    /// fallback search when the preferred port is busy.
    pub desired_port: u16,
    /// Directory the files of the site are served from.
    ///
    /// When it is empty the built in hello world page is served.
    pub root: Option<PathBuf>,
}

/// Failure while parsing the command line.
#[derive(Debug)]
pub enum ArgsError {
    /// `-h` / `--help` was given, print usage and exit successfully.
    HelpRequested,
    /// `-V` / `--version` was given, print the version and exit successfully.
    VersionRequested,
    /// Something was wrong with the arguments.
    Invalid(String),
}

impl fmt::Display for ArgsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArgsError::HelpRequested => write!(f, "help requested"),
            ArgsError::VersionRequested => write!(f, "version requested"),
            ArgsError::Invalid(msg) => write!(f, "{msg}"),
        }
    }
}

impl ArgsError {
    /// Process exit code that matches this error.
    pub fn exit_code(&self) -> i32 {
        match self {
            ArgsError::HelpRequested | ArgsError::VersionRequested => 0,
            ArgsError::Invalid(_) => 2,
        }
    }
}

impl Args {
    /// Parse arguments from the process environment.
    pub fn from_env() -> Result<Args, ArgsError> {
        Args::parse(env::args().skip(1))
    }

    /// Parse an iterator of command line arguments.
    pub fn parse<I, S>(args: I) -> Result<Args, ArgsError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let args: Vec<String> = args.into_iter().map(Into::into).collect();

        // The `PORT` / `HOST` environment variables are used only when the
        // matching flag is absent, so an explicit flag always wins.
        let env_port = env::var("PORT").ok();
        let env_host = env::var("HOST").ok();

        let mut port: Option<u16> = None;
        let mut host: Option<IpAddr> = None;
        let mut root: Option<PathBuf> = None;

        let mut idx = 0;
        while idx < args.len() {
            let arg = args[idx].clone();
            idx += 1;

            // Accepts `--flag=value`, `--flag value` and `-fvalue` spellings.
            let (flag, inline) = match arg.split_once('=') {
                Some((flag, value)) if flag.starts_with('-') => {
                    (flag.to_string(), Some(value.to_string()))
                }
                _ if arg.len() > 2 && arg.starts_with('-') && !arg.starts_with("--") => {
                    (arg[..2].to_string(), Some(arg[2..].to_string()))
                }
                _ => (arg.clone(), None),
            };

            // Pull the value of an option, either from the inline form or the
            // following argument.
            let mut take_value = |flag: &str| -> Result<String, ArgsError> {
                if let Some(value) = inline.clone() {
                    return Ok(value);
                }
                let value = args
                    .get(idx)
                    .cloned()
                    .ok_or_else(|| ArgsError::Invalid(format!("missing value for `{flag}`")))?;
                idx += 1;
                Ok(value)
            };

            match flag.as_str() {
                "-h" | "--help" => return Err(ArgsError::HelpRequested),
                "-V" | "--version" => return Err(ArgsError::VersionRequested),
                "-p" | "--port" => {
                    let value = take_value(&flag)?;
                    port = Some(parse_port(&value)?);
                }
                "-b" | "--host" => {
                    let value = take_value(&flag)?;
                    host = Some(parse_host(&value)?);
                }
                "-r" | "--root" => {
                    let value = take_value(&flag)?;
                    root = Some(PathBuf::from(value));
                }
                other => {
                    return Err(ArgsError::Invalid(format!("unknown argument `{other}`")));
                }
            }
        }

        let desired_port = match port {
            Some(port) => port,
            None => match env_port {
                Some(value) => parse_port(&value)?,
                None => DEFAULT_PORT,
            },
        };

        let host = match host {
            Some(host) => host,
            None => match env_host {
                Some(value) => parse_host(&value)?,
                None => parse_host(DEFAULT_HOST)?,
            },
        };

        Ok(Args {
            host,
            desired_port,
            root,
        })
    }
}

/// Parse a port number, rejecting values outside of `0..=65535`.
fn parse_port(value: &str) -> Result<u16, ArgsError> {
    value.trim().parse::<u16>().map_err(|_| {
        ArgsError::Invalid(format!(
            "invalid port `{value}`: expected a number between 0 and 65535"
        ))
    })
}

/// Parse a bind address. `localhost` is a shortcut for `127.0.0.1`.
fn parse_host(value: &str) -> Result<IpAddr, ArgsError> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("localhost") {
        return Ok(IpAddr::from([127, 0, 0, 1]));
    }
    value.parse::<IpAddr>().map_err(|_| {
        ArgsError::Invalid(format!(
            "invalid host `{value}`: expected an IP address such as 127.0.0.1 or 0.0.0.0"
        ))
    })
}

/// The text printed for `--help`.
pub fn usage() -> String {
    format!(
        "hello-server {version} - a tiny HTTP server that says Hello, World!

USAGE:
    hello-server [OPTIONS]

OPTIONS:
    -p, --port <PORT>    Preferred port to listen on [default: {default_port}]
    -b, --host <HOST>    Address to bind to [default: {default_host}]
    -r, --root <DIR>     Directory the files of the site are served from
    -h, --help           Print this help and exit
    -V, --version        Print version information and exit

ENVIRONMENT:
    PORT                 Preferred port, used when --port is not given
    HOST                 Bind address, used when --host is not given

If the preferred port is already taken, the nearest free port is selected
automatically and the URL that is actually in use is printed on startup.
",
        version = env!("CARGO_PKG_VERSION"),
        default_port = DEFAULT_PORT,
        default_host = DEFAULT_HOST,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, ArgsError> {
        Args::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults_to_port_8000() {
        let args = parse(&[]).unwrap();
        assert_eq!(args.desired_port, 8000);
        assert_eq!(args.host, IpAddr::from([0, 0, 0, 0]));
    }

    #[test]
    fn accepts_custom_port_in_both_spellings() {
        assert_eq!(parse(&["--port", "3000"]).unwrap().desired_port, 3000);
        assert_eq!(parse(&["--port=3000"]).unwrap().desired_port, 3000);
        assert_eq!(parse(&["-p", "3000"]).unwrap().desired_port, 3000);
        assert_eq!(parse(&["-p3000"]).unwrap().desired_port, 3000);
    }

    #[test]
    fn accepts_custom_host() {
        assert_eq!(
            parse(&["--host", "127.0.0.1"]).unwrap().host,
            IpAddr::from([127, 0, 0, 1])
        );
        assert_eq!(
            parse(&["--host=127.0.0.1"]).unwrap().host,
            IpAddr::from([127, 0, 0, 1])
        );
        assert_eq!(
            parse(&["--host", "localhost"]).unwrap().host,
            IpAddr::from([127, 0, 0, 1])
        );
    }

    #[test]
    fn rejects_out_of_range_port() {
        assert!(parse(&["--port", "70000"]).is_err());
        assert!(parse(&["--port", "abc"]).is_err());
    }

    #[test]
    fn rejects_unknown_argument() {
        assert!(parse(&["--nope"]).is_err());
    }

    #[test]
    fn reports_missing_value() {
        assert!(parse(&["--port"]).is_err());
    }
}
