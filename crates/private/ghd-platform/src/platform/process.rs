//! Host-neutral process facade.

use std::io;
use std::process::{Child, Command, ExitStatus};

/// Spawn `command` so it outlives the caller and is not signalled with it.
pub fn spawn_detached(command: &mut Command) -> io::Result<Child> {
    crate::platform_imp::process::spawn_detached(command)
}

/// Preserve native command exit codes, including Unix signal conventions.
pub fn exit_status_code(status: ExitStatus) -> i32 {
    crate::platform_imp::process::exit_status_code(status)
}
