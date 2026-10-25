//! Windows state directory (`%LOCALAPPDATA%`). Files there inherit the
//! user profile's owner-only ACL, so no extra DACL work is done here.

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
    fs::create_dir_all(dir)
}

pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(tmp, path)
}
