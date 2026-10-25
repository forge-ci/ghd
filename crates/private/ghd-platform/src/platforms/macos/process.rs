//! Detached spawn: a new process group, so the caller's Ctrl-C or session
//! teardown does not reach the daemon.

use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

pub(crate) fn spawn_detached(command: &mut Command) -> io::Result<Child> {
    command.process_group(0).spawn()
}
