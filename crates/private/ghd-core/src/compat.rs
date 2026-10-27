//! Execute real gh with inherited streams and conservative write fencing.

use crate::{coherence::Journal, platform};
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Read,
    Write,
    Local,
}

/// Unknown commands may be aliases/extensions that mutate GitHub. Fence them
/// rather than assuming that a previously unseen action is a safe read.
pub fn classify(args: &[&str]) -> Kind {
    if args
        .first()
        .is_some_and(|a| matches!(*a, "auth" | "config" | "help"))
    {
        return Kind::Local;
    }
    match args {
        ["pr", "list" | "view" | "status" | "checks" | "diff", ..]
        | ["issue", "list" | "view" | "status", ..]
        | ["run", "list" | "view" | "watch", ..]
        | ["repo", "list" | "view", ..]
        | ["workflow", "list" | "view", ..]
        | ["release", "list" | "view", ..]
        | [
            "search",
            "repos" | "issues" | "prs" | "commits" | "code",
            ..,
        ]
        | [
            "label" | "gist" | "cache" | "org" | "secret" | "variable",
            "list",
            ..,
        ]
        | ["gist" | "project" | "ruleset", "view", ..]
        | ["project", "list" | "field-list" | "item-list", ..]
        | ["ruleset", "list" | "check", ..]
        | ["variable", "get", ..] => Kind::Read,
        ["--version" | "--help" | "-h"] | [] => Kind::Local,
        _ => Kind::Write,
    }
}

/// Shared real-gh lookup for passthrough and host-bound auth fallback.
pub fn command() -> io::Result<Command> {
    if std::env::var_os("GHD_PASSTHROUGH").is_some() {
        return Err(io::Error::other(
            "gh shim recursion detected; set GHD_GH_PATH to the real GitHub CLI",
        ));
    }
    let name = format!("gh{}", platform::host::exe_suffix());
    let path = match std::env::var_os("GHD_GH_PATH").filter(|v| !v.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => std::env::var_os("PATH")
            .into_iter()
            .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .map(|dir| dir.join(&name))
            .find(|path| path.is_file())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "real gh not found; install GitHub CLI or set GHD_GH_PATH",
                )
            })?,
    };
    if path.canonicalize()? == std::env::current_exe()?.canonicalize()? {
        return Err(io::Error::other(
            "gh resolves to ghd; set GHD_GH_PATH to the real GitHub CLI",
        ));
    }
    let mut command = Command::new(path);
    command.env("GHD_PASSTHROUGH", "1");
    Ok(command)
}

#[derive(Debug)]
struct CommandExit(i32);

impl fmt::Display for CommandExit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gh exited with status {}", self.0)
    }
}
impl std::error::Error for CommandExit {}

pub fn exit_code(error: &io::Error) -> Option<i32> {
    error
        .get_ref()?
        .downcast_ref::<CommandExit>()
        .map(|exit| exit.0)
}

pub fn run(args: &[&str]) -> io::Result<()> {
    let mut command = command()?;
    command
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let intent = if classify(args) == Kind::Write {
        Some(Journal::global()?.begin()?)
    } else {
        None
    };
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            // spawn failed before dispatch; there is no possibly-live writer.
            if let Some(intent) = intent {
                intent.finish()?;
            }
            return Err(error);
        }
    };
    // A failed wait leaves the write fenced: the child may still be alive.
    let status = child.wait()?;
    if status.success() {
        if let Some(intent) = intent {
            intent.finish()?;
        }
        Ok(())
    } else {
        Err(io::Error::other(CommandExit(
            platform::process::exit_status_code(status),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_read_and_local_shapes_avoid_write_fencing() {
        assert_eq!(
            classify(&["pr", "view", "12", "--jq", ".title"]),
            Kind::Read
        );
        assert_eq!(classify(&["auth", "login"]), Kind::Local);
        assert_eq!(classify(&["pr", "merge", "12"]), Kind::Write);
        assert_eq!(classify(&["my-alias"]), Kind::Write);
        assert_eq!(classify(&["api", "graphql", "--input", "-"]), Kind::Write);
    }
}
