//! One std-only HTTP/1.1 call to the loopback daemon, so a warm `ghd`
//! invocation builds no runtime and no TLS stack. When nothing answers, the
//! client starts the daemon (once, under a lock) and retries.

use crate::endpoint::{self, Endpoint};
use crate::{identity::Identity, platform};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const START_DEADLINE: Duration = Duration::from_secs(5);
const READ_TIMEOUT: Duration = Duration::from_secs(120);

/// `(status, body)` from the daemon, starting it if it is not running.
pub fn call(method: &str, path: &str, body: &[u8]) -> io::Result<(u16, Vec<u8>)> {
    call_host(None, method, path, body)
}

pub fn call_host(
    host: Option<&str>,
    method: &str,
    path: &str,
    body: &[u8],
) -> io::Result<(u16, Vec<u8>)> {
    let identity = Identity::resolve(host)?;
    match endpoint::read_for(&identity).and_then(|ep| send(&identity, &ep, method, path, body)) {
        Ok(reply) if reply.0 != 401 => Ok(reply),
        _ => send(&identity, &start_daemon(&identity)?, method, path, body),
    }
}

fn send(
    identity: &Identity,
    ep: &Endpoint,
    method: &str,
    path: &str,
    body: &[u8],
) -> io::Result<(u16, Vec<u8>)> {
    let mut stream = TcpStream::connect(("127.0.0.1", ep.port))?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nX-Ghd-Identity: {}\r\n\
         Content-Type: application/x-protobuf\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        ep.token,
        identity.key(),
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    parse_response(&raw)
}

/// Split a complete HTTP/1.1 response into status and (de-chunked) body.
pub fn parse_response(raw: &[u8]) -> io::Result<(u16, Vec<u8>)> {
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "malformed daemon response");
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(bad)?;
    let head = std::str::from_utf8(&raw[..split]).map_err(|_| bad())?;
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(bad)?;
    let body = &raw[split + 4..];
    let chunked = head.lines().any(|line| {
        let line = line.to_ascii_lowercase();
        line.starts_with("transfer-encoding:") && line.contains("chunked")
    });
    let body = if chunked {
        dechunk(body).ok_or_else(bad)?
    } else {
        body.to_vec()
    };
    Ok((status, body))
}

fn dechunk(mut data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = data.windows(2).position(|w| w == b"\r\n")?;
        let size =
            usize::from_str_radix(std::str::from_utf8(&data[..line_end]).ok()?.trim(), 16).ok()?;
        data = &data[line_end + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(data.get(..size)?);
        data = data.get(size + 2..)?;
    }
}

fn healthy(identity: &Identity) -> Option<Endpoint> {
    let ep = endpoint::read_for(identity).ok()?;
    matches!(send(identity, &ep, "GET", "/v1/health", &[]), Ok((200, _))).then_some(ep)
}

/// The `ghd` executable that runs the daemon: `$GHD_EXE`, this process when
/// it is `ghd` itself, else `ghd` on PATH (the Python binding's case).
fn daemon_exe() -> io::Result<PathBuf> {
    if let Some(exe) = std::env::var_os("GHD_EXE").filter(|e| !e.is_empty()) {
        return Ok(PathBuf::from(exe));
    }
    let current = std::env::current_exe()?;
    if current.file_stem().is_some_and(|s| s == "ghd") {
        return Ok(current);
    }
    let name = format!("ghd{}", platform::host::exe_suffix());
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(&name))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "ghd executable not found; set GHD_EXE",
            )
        })
}

/// Launch `ghd daemon` detached and wait, bounded, for it to answer. An
/// exclusive lock makes concurrent cold clients start exactly one daemon:
/// the rest wait on the lock, then find it healthy.
fn start_daemon(identity: &Identity) -> io::Result<Endpoint> {
    let dir = endpoint::directory(identity)?;
    platform::fs::create_private_dir(&dir)?;
    let lock = std::fs::File::create(dir.join("start.lock"))?;
    lock.lock()?;
    if let Some(ep) = healthy(identity) {
        return Ok(ep);
    }
    let log = std::fs::File::create(dir.join("daemon.log"))?;
    let mut command = Command::new(daemon_exe()?);
    identity.configure_child(&mut command);
    command
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    platform::process::spawn_detached(&mut command)?;
    let deadline = Instant::now() + START_DEADLINE;
    while Instant::now() < deadline {
        if let Some(ep) = healthy(identity) {
            return Ok(ep);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err(io::Error::other(format!(
        "ghd daemon did not come up within {START_DEADLINE:?}; see {}",
        dir.join("daemon.log").display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_content_length_response() {
        let (status, body) = parse_response(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
            .expect("test fixture");
        assert_eq!((status, body.as_slice()), (200, b"ok".as_slice()));
    }

    #[test]
    fn parses_a_chunked_response() {
        let raw = b"HTTP/1.1 404 Not Found\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        let (status, body) = parse_response(raw).expect("test fixture");
        assert_eq!((status, body.as_slice()), (404, b"abcde".as_slice()));
    }

    #[test]
    fn rejects_a_response_without_a_header_terminator() {
        assert!(parse_response(b"HTTP/1.1 200 OK\r\n").is_err());
    }
}
