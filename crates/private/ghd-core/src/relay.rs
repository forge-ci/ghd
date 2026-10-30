//! The `ghd gh ...` fallback (ghd#8): run the real gh with the caller's
//! stdin, stdout, stderr and terminal, and return its exit code. Commands
//! ghd cannot serve natively still work, so ghd can stand in for gh.

use std::env;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// The real gh binary. A launcher that puts a `gh` shim on PATH (clud) sets
/// this so the relay never resolves back to the shim.
pub const REAL_GH_ENV: &str = "GHD_REAL_GH";

/// Set on the relayed child. Seeing it on entry means the "real" gh led
/// back into ghd: fail instead of looping.
const RELAYING_ENV: &str = "GHD_RELAYING";

/// Run the real gh with `args`.
pub fn run(args: &[&str]) -> io::Result<ExitCode> {
    if env::var_os(RELAYING_ENV).is_some() {
        return Err(io::Error::other(format!(
            "relay loop: the real gh resolved back to ghd; set {REAL_GH_ENV} to the gh binary"
        )));
    }
    let gh = real_gh(
        env::var_os(REAL_GH_ENV),
        env::var_os("PATH"),
        env::current_exe().ok(),
    )?;
    let status = Command::new(&gh)
        .args(args)
        .env(RELAYING_ENV, "1")
        .status()?;
    Ok(status.code().map_or(ExitCode::FAILURE, |code| {
        ExitCode::from(u8::try_from(code).unwrap_or(1))
    }))
}

/// `GHD_REAL_GH` when set, else the first `gh` on `path` that is not this
/// ghd binary.
pub fn real_gh(
    explicit: Option<OsString>,
    path: Option<OsString>,
    this_exe: Option<PathBuf>,
) -> io::Result<PathBuf> {
    if let Some(gh) = explicit.filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(gh));
    }
    let name = format!("gh{}", env::consts::EXE_SUFFIX);
    let this = this_exe.and_then(|exe| exe.canonicalize().ok());
    env::split_paths(&path.unwrap_or_default())
        .map(|dir| dir.join(&name))
        .find(|candidate| candidate.is_file() && !same_file(candidate, this.as_deref()))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no gh on PATH; install gh or set {REAL_GH_ENV}"),
            )
        })
}

fn same_file(candidate: &Path, this: Option<&Path>) -> bool {
    this.is_some_and(|this| candidate.canonicalize().is_ok_and(|real| real == this))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_real_gh_wins() {
        let gh = real_gh(Some("/opt/gh".into()), Some("/usr/bin".into()), None).unwrap();
        assert_eq!(gh, PathBuf::from("/opt/gh"));
    }

    #[test]
    fn path_search_skips_this_binary_and_reports_a_missing_gh() {
        let dir = std::env::temp_dir().join(format!("ghd-relay-{}", std::process::id()));
        let first = dir.join("first");
        let second = dir.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let name = format!("gh{}", env::consts::EXE_SUFFIX);
        std::fs::write(first.join(&name), "").unwrap();
        std::fs::write(second.join(&name), "").unwrap();
        let path = env::join_paths([&first, &second]).unwrap();

        let found = real_gh(None, Some(path.clone()), Some(first.join(&name))).unwrap();
        assert_eq!(found, second.join(&name));

        let empty = env::join_paths([dir.join("none")]).unwrap();
        let err = real_gh(None, Some(empty), None).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
