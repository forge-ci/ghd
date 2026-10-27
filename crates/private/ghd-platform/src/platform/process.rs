//! Host-neutral process facade.

use std::io;
use std::process::{Command, ExitStatus};

/// Spawn `command` so it outlives the caller and is not signalled with it.
pub fn spawn_detached(
    command: &mut Command,
    log: &std::fs::File,
) -> io::Result<kernal_api::platform::process::DaemonChild> {
    use kernal_api::platform::process::{DaemonStdio, DaemonStdioSource, SyncEnvironment};
    // The native boundary inherits only these explicit file handles. In
    // particular a Windows daemon must not keep a launcher's capture pipes open.
    kernal_api::spawn_sync_daemon(
        command,
        DaemonStdio {
            stdout: DaemonStdioSource::File(log),
            stderr: DaemonStdioSource::File(log),
        },
        SyncEnvironment::Inherit,
        false,
    )
}

/// Preserve native command exit codes, including Unix signal conventions.
pub fn exit_status_code(status: ExitStatus) -> i32 {
    crate::platform_imp::process::exit_status_code(status)
}

/// Convert the bounded runner's native status without confusing Windows codes
/// with Unix's negative signal representation.
pub fn bounded_exit_code(raw: i32) -> io::Result<i32> {
    crate::platform_imp::process::bounded_exit_code(raw)
}
