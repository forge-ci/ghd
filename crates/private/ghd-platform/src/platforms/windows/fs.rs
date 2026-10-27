//! Windows state directory (`%LOCALAPPDATA%`). Files there inherit the
//! profile by default; explicit owner/SYSTEM DACLs protect custom roots too.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub(crate) fn state_base() -> io::Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is not set"))
}

pub(crate) fn create_private_dir(dir: &Path) -> io::Result<()> {
    // The strict reader requires TokenUser ownership too. General filesystem
    // creation alone can retain an elevated token's Administrators owner.
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
