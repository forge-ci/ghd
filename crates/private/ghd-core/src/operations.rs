//! Authenticated lifecycle and redacted cache operations; never signal a PID.
use crate::{client, endpoint, identity::Identity};
use std::{
    fs::File,
    io,
    io::Write,
    time::{Duration, Instant},
};

pub fn custody(identity: &Identity, role: &str) -> io::Result<File> {
    if !matches!(role, "owner" | "maintenance") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid custody role",
        ));
    }
    crate::durable::lock_directory(&endpoint::directory(identity)?.join(role)).map(|(file, _)| file)
}
fn existing(identity: &Identity, method: &str, path: &str) -> io::Result<(u16, Vec<u8>)> {
    let ep = endpoint::read_for(identity)?;
    client::send_existing(identity, &ep, method, path, &[])
}
pub fn run(args: &[&str]) -> io::Result<()> {
    let identity = Identity::resolve(None)?;
    match args {
        ["status"] => {
            let status = match endpoint::read_for(&identity) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => "missing",
                Err(e) => return Err(e),
                Ok(_) => {
                    if matches!(existing(&identity, "GET", "/v1/health"), Ok((200, _))) {
                        "healthy"
                    } else {
                        "stale"
                    }
                }
            };
            println!("{{\"status\":\"{status}\"}}");
            if status != "healthy" {
                return Err(crate::compat::exit_error(3));
            }
        }
        [command @ ("start" | "restart" | "stop")] => {
            crate::config::validate()?;
            let _maintenance = maintenance(&identity)?;
            if *command != "start" {
                stop(&identity)?;
            }
            if *command != "stop" {
                client::start_daemon(&identity)?;
            }
            println!(
                "{{\"status\":\"{}\"}}",
                if *command == "stop" {
                    "stopped"
                } else {
                    "healthy"
                }
            );
        }
        ["cache", command @ ("keys" | "flush" | "stats")] => {
            let (method, path) = match *command {
                "flush" => ("POST", "/v1/admin/flush"),
                "keys" => ("GET", "/v1/admin/keys"),
                _ => ("GET", "/v1/stats"),
            };
            let (status, body) = existing(&identity, method, path)?;
            if status != 200 {
                return Err(io::Error::other(
                    String::from_utf8_lossy(&body).into_owned(),
                ));
            }
            io::stdout().lock().write_all(&body)?;
            println!();
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected daemon start|stop|restart|status or cache stats|keys|flush",
            ));
        }
    }
    Ok(())
}
fn stop(identity: &Identity) -> io::Result<()> {
    match existing(identity, "POST", "/v1/admin/stop") {
        Ok((200, _)) => {}
        Ok((status, body)) => {
            return Err(io::Error::other(format!(
                "stop refused (HTTP {status}): {}",
                String::from_utf8_lossy(&body)
            )));
        }
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) => {}
        Err(e) => return Err(e),
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match custody(identity, "owner") {
            Ok(_released) => return Ok(()),
            Err(e) if Instant::now() >= deadline => {
                return Err(io::Error::other(format!(
                    "daemon ownership not released: {e}"
                )));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

fn maintenance(identity: &Identity) -> io::Result<File> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match custody(identity, "maintenance") {
            Ok(lock) => return Ok(lock),
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}
