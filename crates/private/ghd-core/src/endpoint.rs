//! Where the daemon listens: `<state dir>/endpoint` holds `port token`,
//! written owner-private by the daemon and read by every client.

use crate::{identity::Identity, platform};
use std::io;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
}

pub fn state_dir() -> io::Result<PathBuf> {
    directory(&Identity::resolve(None)?)
}

pub fn directory(identity: &Identity) -> io::Result<PathBuf> {
    Ok(identity.directory(&platform::fs::state_dir()?))
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
    read_for(&Identity::resolve(None)?)
}

pub fn read_for(identity: &Identity) -> io::Result<Endpoint> {
    let bytes = platform::fs::read_private_file(&directory(identity)?.join("endpoint"), 4096)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    parse(text)
}

pub fn write(endpoint: &Endpoint) -> io::Result<()> {
    write_for(&Identity::resolve(None)?, endpoint)
}

pub fn write_for(identity: &Identity, endpoint: &Endpoint) -> io::Result<()> {
    let dir = directory(identity)?;
    platform::fs::create_private_dir(&dir)?;
    platform::fs::write_private(
        &dir.join("endpoint"),
        format!("{} {}\n", endpoint.port, endpoint.token).as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_child() {
        if std::env::var_os("GHD_IDENTITY_TEST").is_some() {
            println!(
                "GHD_TEST_PATH={}",
                state_dir().expect("state directory").display()
            );
        }
    }

    fn child_path(host: &str, token: &str) -> String {
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "endpoint::tests::identity_child", "--nocapture"])
            .env("GHD_IDENTITY_TEST", "1")
            .env("GH_HOST", host)
            .env("GH_TOKEN", token)
            .env("GH_ENTERPRISE_TOKEN", token)
            .output()
            .expect("child test");
        assert!(output.status.success(), "child failed");
        String::from_utf8(output.stdout)
            .expect("utf8")
            .lines()
            .find_map(|line| line.strip_prefix("GHD_TEST_PATH="))
            .expect("directory marker")
            .to_string()
    }

    #[test]
    fn credentials_and_hosts_cannot_share_a_daemon_directory() {
        let first = child_path("github.com", "test-identity-a");
        let rotated = child_path("github.com", "test-identity-b");
        let enterprise = child_path("github.example.com", "test-identity-a");
        assert_ne!(
            first, rotated,
            "token rotation must select a different daemon"
        );
        assert_ne!(first, enterprise, "hosts must not share a daemon");
        assert!(
            !first.contains("test-identity-a"),
            "do not expose tokens in paths"
        );
    }

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
