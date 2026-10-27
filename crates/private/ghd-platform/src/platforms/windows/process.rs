//! Detached spawn: no console and a new process group, so closing the
//! caller's console does not take the daemon with it.

use std::io;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, ExitStatus};

const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

pub(crate) fn spawn_detached(command: &mut Command) -> io::Result<Child> {
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP).spawn()
}

pub(crate) fn exit_status_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

#[expect(clippy::unnecessary_wraps, reason = "matches the fallible Unix facade signature")]
pub(crate) fn bounded_exit_code(raw: i32) -> io::Result<i32> {
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_status_retains_windows_crash_bits() {
        let access_violation = 0xc0000005_u32.cast_signed();
        assert_eq!(bounded_exit_code(access_violation).expect("native code"), access_violation);
        assert_eq!(bounded_exit_code(i32::MIN).expect("native word"), i32::MIN);
    }
}
