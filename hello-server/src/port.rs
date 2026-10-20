//! Finding a usable TCP port.
//!
//! The preferred port is tried first. If it is already taken, the search
//! walks outwards from it (up first, then down, then up further, ...) and
//! takes the first candidate that can actually be bound. Binding the socket
//! and keeping it is what makes this race free: the port is never released
//! between the availability check and the server starting on it.

use std::io;
use std::net::{IpAddr, SocketAddr, TcpListener};

/// Lowest port the downward search is allowed to reach. Ports below this
/// range are privileged on unix systems.
pub const MIN_PORT: u16 = 1024;

/// Highest port the upward search is allowed to reach.
pub const MAX_PORT: u16 = 65535;

/// How far away from the preferred port the search is willing to look.
pub const MAX_SEARCH_RANGE: u16 = 100;

/// A port that was successfully bound, plus the port that was originally
/// asked for.
#[derive(Debug)]
pub struct BoundPort {
    /// The live listening socket. It is never closed by this module.
    pub listener: TcpListener,
    /// The port the socket is actually listening on.
    pub port: u16,
    /// The port that was requested before the fallback search kicked in.
    pub requested: u16,
}

impl BoundPort {
    /// True when the fallback search had to move away from the requested port.
    pub fn is_fallback(&self) -> bool {
        self.port != self.requested
    }
}

/// The ordered list of ports to try after the preferred one failed.
///
/// The order is by increasing distance from the preferred port, preferring
/// the higher port when both directions are equally close.
pub fn candidate_ports(preferred: u16, range: u16) -> Vec<u16> {
    let mut candidates = Vec::new();
    for distance in 1..=range {
        // The arithmetic is done in u32 so that the bounds checks below are
        // meaningful and cannot overflow.
        let up = u32::from(preferred) + u32::from(distance);
        let down = u32::from(preferred).saturating_sub(u32::from(distance));

        if up <= u32::from(MAX_PORT) {
            candidates.push(up as u16);
        }
        if down >= u32::from(MIN_PORT) {
            candidates.push(down as u16);
        }
    }
    candidates
}

/// Bind `host` on `preferred`, falling back to the nearest free port.
///
/// Returns the live listener together with the port it ended up on.
pub fn bind_with_fallback(host: IpAddr, preferred: u16) -> io::Result<BoundPort> {
    match try_bind(host, preferred)? {
        Some(listener) => Ok(BoundPort {
            listener,
            port: preferred,
            requested: preferred,
        }),
        None => {
            // The preferred port is taken. Look for the closest one that is
            // not, keeping the socket we manage to bind.
            for candidate in candidate_ports(preferred, MAX_SEARCH_RANGE) {
                if let Some(listener) = try_bind(host, candidate)? {
                    return Ok(BoundPort {
                        listener,
                        port: candidate,
                        requested: preferred,
                    });
                }
            }

            Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                format!(
                    "no free port found near {preferred} (searched {} candidates)",
                    candidate_ports(preferred, MAX_SEARCH_RANGE).len()
                ),
            ))
        }
    }
}

/// Try to bind `host` on `port`.
///
/// * `Ok(Some(..))` - the port was free and is now held by us.
/// * `Ok(None)`    - the port is in use by somebody else.
/// * `Err(..)`     - a different failure that retrying will not fix.
fn try_bind(host: IpAddr, port: u16) -> io::Result<Option<TcpListener>> {
    let addr = SocketAddr::new(host, port);
    match TcpListener::bind(addr) {
        Ok(listener) => Ok(Some(listener)),
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => Ok(None),
        Err(err) => Err(err),
    }
}

/// The URL a user should open to see the hello world page.
pub fn display_url(host: IpAddr, port: u16) -> String {
    match host {
        // A wildcard bind is reachable through the loopback address.
        IpAddr::V4(v4) if v4.is_unspecified() => format!("http://localhost:{port}"),
        IpAddr::V6(v6) if v6.is_unspecified() => format!("http://localhost:{port}"),
        IpAddr::V6(_) => format!("http://[{}]:{port}", host),
        _ => format!("http://{host}:{port}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_start_above_then_below() {
        assert_eq!(
            candidate_ports(8000, 3),
            vec![8001, 7999, 8002, 7998, 8003, 7997]
        );
    }

    #[test]
    fn candidates_never_go_below_min_port() {
        let candidates = candidate_ports(1025, 10);
        assert!(candidates
            .iter()
            .all(|&p| u32::from(p) >= u32::from(MIN_PORT)));
        assert!(!candidates.contains(&1023));
    }

    #[test]
    fn candidates_never_go_above_max_port() {
        let candidates = candidate_ports(65535, 10);
        assert!(candidates
            .iter()
            .all(|&p| u32::from(p) <= u32::from(MAX_PORT)));
        assert!(!candidates.contains(&0));
    }

    #[test]
    fn candidates_are_ordered_by_distance() {
        let candidates = candidate_ports(8000, 5);
        let distances: Vec<u16> = candidates.iter().map(|p| p.abs_diff(8000)).collect();
        let mut sorted = distances.clone();
        sorted.sort_unstable();
        assert_eq!(distances, sorted);
    }

    #[test]
    fn binds_the_preferred_port_when_it_is_free() {
        let free = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = free.local_addr().unwrap().port();
        drop(free);

        let bound = bind_with_fallback(IpAddr::from([127, 0, 0, 1]), port).unwrap();
        assert_eq!(bound.port, port);
        assert!(!bound.is_fallback());
    }

    #[test]
    fn falls_back_when_the_preferred_port_is_taken() {
        let taken = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = taken.local_addr().unwrap().port();

        let bound = bind_with_fallback(IpAddr::from([127, 0, 0, 1]), port).unwrap();
        assert_ne!(bound.port, port);
        assert!(bound.is_fallback());
        assert_eq!(bound.requested, port);
        // The fallback port must actually be usable.
        assert!(bound.listener.local_addr().is_ok());
    }

    #[test]
    fn fallback_is_the_nearest_free_port() {
        // Occupy a port and its immediate neighbours, so the search has to
        // walk further out.
        let blocker = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = blocker.local_addr().unwrap().port();

        let bound = bind_with_fallback(IpAddr::from([127, 0, 0, 1]), port).unwrap();
        assert_eq!(bound.port, port + 1);
    }

    #[test]
    fn display_url_uses_localhost_for_wildcard_binds() {
        assert_eq!(
            display_url(IpAddr::from([0, 0, 0, 0]), 8000),
            "http://localhost:8000"
        );
        assert_eq!(
            display_url(IpAddr::from([127, 0, 0, 1]), 8000),
            "http://127.0.0.1:8000"
        );
        assert_eq!(
            display_url(IpAddr::from([0, 0, 0, 0, 0, 0, 0, 1]), 8000),
            "http://[::1]:8000"
        );
    }
}
