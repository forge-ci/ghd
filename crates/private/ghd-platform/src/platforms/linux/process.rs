//! Detached spawn: a new process group, so the caller's Ctrl-C or session
//! teardown does not reach the daemon.

use std::io;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus};

pub(crate) fn spawn_detached(command: &mut Command) -> io::Result<Child> {
    command.process_group(0).spawn()
}

pub(crate) fn exit_status_code(status: ExitStatus) -> i32 {
    status.code().or_else(|| status.signal().map(|signal| 128 + signal)).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_status_uses_the_shell_exit_convention() {
        assert_eq!(exit_status_code(ExitStatus::from_raw(9)), 137);
    }
}

// Deliberately mirrored in the other Unix adapter: this is OS status meaning,
// not portable command policy. Windows retains the full signed native word.
pub(crate) fn bounded_exit_code(raw: i32) -> io::Result<i32> {
    match raw {
        0..=255 => Ok(raw),
        -127..=-1 => Ok(128 - raw),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "invalid bounded Unix termination status")),
    }
}

#[cfg(test)]
mod bounded_status_tests {
    use super::*;
    #[test]
    fn bounded_status_preserves_exits_and_normalizes_signals() {
        assert_eq!(bounded_exit_code(7).expect("exit"), 7);
        assert_eq!(bounded_exit_code(-9).expect("signal"), 137);
        assert!(bounded_exit_code(i32::MIN).is_err());
    }
}
