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
    // Delegate ownership/DACL enforcement to the existing native facade.
    // In particular a GHD_STATE_DIR override need not inherit profile ACLs.
    kernal_api::platform::fs::create_dir_all_private(dir)
}

pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(tmp, path)
}
