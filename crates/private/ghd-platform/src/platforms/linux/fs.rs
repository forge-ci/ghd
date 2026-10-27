//! Linux state directory (`$XDG_STATE_HOME` or `~/.local/state`) and
//! mode-0600 atomic writes.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub(crate) fn state_base() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_STATE_HOME").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
    Ok(PathBuf::from(home).join(".local/state"))
}

pub(crate) fn create_private_dir(dir: &Path) -> io::Result<()> {
    // Delegate ownership/DACL enforcement to the existing native facade.
    // In particular a GHD_STATE_DIR override need not inherit profile ACLs.
    kernal_api::platform::fs::create_dir_all_private(dir)
}

pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new().create(true).write(true).truncate(true).mode(0o600).open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(tmp, path)
}
