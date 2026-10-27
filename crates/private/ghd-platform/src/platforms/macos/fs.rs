//! macOS state directory (`~/Library/Application Support`) and mode-0600
//! atomic writes.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub(crate) fn state_base() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    Ok(PathBuf::from(home).join("Library/Application Support"))
}

pub(crate) fn create_private_dir(dir: &Path) -> io::Result<()> {
    // Creation and validation are separate kernel contracts. Existing paths
    // must also satisfy the strict state reader's current-user policy.
    kernal_api::platform::fs::create_dir_all_private(dir)?;
    kernal_api::platform::ipc::ensure_owner_private_directory(dir)?;
    Ok(())
}

pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // Deliberately shared publication policy across the native adapters;
    // kernel primitives enforce each host's private ownership and permissions.
    let parent = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "private state file needs a parent"))?;
    let stage = kernal_api::platform::fs::TemporaryDirectory::in_directory(parent, ".ghd-write-")?;
    create_private_dir(stage.path())?;
    let staged = stage.path().join("data");
    let mut file = kernal_api::platform::fs::create_private_file(&staged)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&staged, path)?;
    stage.close()
}
