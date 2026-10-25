//! Host-neutral process facade.

use std::io;
use std::process::{Child, Command};

/// Spawn `command` so it outlives the caller and is not signalled with it.
pub fn spawn_detached(command: &mut Command) -> io::Result<Child> {
    crate::platform_imp::process::spawn_detached(command)
}
