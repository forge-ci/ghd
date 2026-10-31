//! Where the daemon listens: `<state dir>/endpoint-p<PROTOCOL>` holds
//! `port token`, written owner-private by the daemon and read by every client.
//!
//! The file name carries the wire protocol, so a client never even finds a
//! daemon built for another protocol (forge-ci/ghd#31): two builds that
//! disagree run side by side, and nobody has to kill the other's daemon.
//! Daemons from before the handshake wrote the bare `endpoint` file.

use crate::platform;
use std::io;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
}

pub fn state_dir() -> io::Result<PathBuf> {
    platform::fs::state_dir()
}

/// The client/daemon wire protocol. Bump it on ANY change to a request or
/// response field or its meaning (e.g. 2: `PrViewRequest.compact`, #23).
pub const PROTOCOL: u32 = 2;

/// `<name>-p<PROTOCOL>`: per-protocol state files (endpoint, lock, log).
pub fn versioned(name: &str) -> String {
    format!("{name}-p{PROTOCOL}")
}

fn file_name() -> String {
    versioned("endpoint")
}

/// What `GET /v1/health` answers; the client's handshake checks it exactly.
pub fn health_body() -> String {
    format!("ok protocol={PROTOCOL}")
}

fn path() -> io::Result<PathBuf> {
    Ok(state_dir()?.join(file_name()))
}

pub fn parse(text: &str) -> io::Result<Endpoint> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "bad endpoint file");
    let mut parts = text.split_whitespace();
    let port = parts.next().and_then(|p| p.parse().ok()).ok_or_else(bad)?;
    let token = parts
        .next()
        .filter(|t| !t.is_empty())
        .ok_or_else(bad)?
        .to_string();
    Ok(Endpoint { port, token })
}

pub fn read() -> io::Result<Endpoint> {
    parse(&std::fs::read_to_string(path()?)?)
}

pub fn write(endpoint: &Endpoint) -> io::Result<()> {
    let dir = state_dir()?;
    platform::fs::create_private_dir(&dir)?;
    platform::fs::write_private(
        &path()?,
        format!("{} {}\n", endpoint.port, endpoint.token).as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_port_and_token() {
        assert_eq!(
            parse("4123 abcd\n").expect("test fixture"),
            Endpoint {
                port: 4123,
                token: "abcd".into()
            }
        );
    }

    #[test]
    fn endpoint_file_is_keyed_by_protocol_so_older_daemons_are_invisible() {
        let name = file_name();
        assert_ne!(
            name, "endpoint",
            "pre-handshake daemons write the bare `endpoint` file"
        );
        assert!(name.ends_with(&PROTOCOL.to_string()));
    }

    #[test]
    fn rejects_a_missing_token() {
        assert!(parse("4123").is_err());
        assert!(parse("").is_err());
    }
}
