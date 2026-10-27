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
