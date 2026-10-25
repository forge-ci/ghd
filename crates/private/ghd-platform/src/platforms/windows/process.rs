//! Detached spawn: no console and a new process group, so closing the
//! caller's console does not take the daemon with it.

use std::io;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};

const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

pub(crate) fn spawn_detached(command: &mut Command) -> io::Result<Child> {
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP).spawn()
}
