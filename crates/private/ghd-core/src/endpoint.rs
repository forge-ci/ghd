//! Where the daemon listens: `<state dir>/endpoint` holds `port token`,
//! written owner-private by the daemon and read by every client.

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

fn path() -> io::Result<PathBuf> {
    Ok(state_dir()?.join("endpoint"))
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
    fn rejects_a_missing_token() {
        assert!(parse("4123").is_err());
        assert!(parse("").is_err());
    }
}
