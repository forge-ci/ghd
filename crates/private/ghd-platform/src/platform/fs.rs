//! Host-neutral per-user state facade.

use std::io;
use std::path::{Path, PathBuf};

/// The per-user directory ghd keeps its endpoint file and daemon log in.
pub fn state_dir() -> io::Result<PathBuf> {
    Ok(crate::platform_imp::fs::state_base()?.join("ghd"))
}

/// Create `dir` (and parents) readable only by the current user.
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    crate::platform_imp::fs::create_private_dir(dir)
}

/// Atomically replace `path` with `bytes`, readable only by the current user.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    crate::platform_imp::fs::write_private(path, bytes)
}
