//! Host-neutral per-user state facade.

use std::io;
use std::path::{Path, PathBuf};

/// The per-user directory ghd keeps its endpoint file and daemon log in.
pub fn state_dir() -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("GHD_STATE_DIR") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "GHD_STATE_DIR must be absolute",
            ));
        }
        return Ok(path);
    }
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

/// Read a bounded regular file only after validating native owner privacy.
pub fn read_private_file(path: &Path, max_bytes: usize) -> io::Result<Vec<u8>> {
    kernal_api::platform::fs::read_private_regular_file_bounded(path, max_bytes)
}
