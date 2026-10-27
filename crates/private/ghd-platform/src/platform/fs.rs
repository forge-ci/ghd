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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_directories_and_leftover_staging_cannot_break_private_publication() {
        let root = std::env::temp_dir().join(format!(
            "ghd-private-roundtrip-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        assert!(!root.exists(), "owns a new directory");
        std::fs::create_dir(&root).expect("pre-existing state directory");
        create_private_dir(&root).expect("current-user private directory");
        assert!(
            kernal_api::platform::ipc::owner_private_directory(&root).expect("directory custody")
        );
        let path = root.join("state");
        let leftover = path.with_extension("tmp");
        {
            use std::io::Write as _;
            let mut old =
                kernal_api::platform::fs::create_private_file(&leftover).expect("old stage");
            old.write_all(b"leftover from old process")
                .expect("old staged bytes");
        }
        write_private(&path, b"first").expect("first publish");
        assert_eq!(
            read_private_file(&path, 32).expect("strict first read"),
            b"first"
        );
        write_private(&path, b"second").expect("replace");
        assert_eq!(
            read_private_file(&path, 32).expect("strict replacement read"),
            b"second"
        );
        assert_eq!(
            read_private_file(&leftover, 64).expect("leftover unchanged"),
            b"leftover from old process"
        );
        assert_eq!(
            std::fs::read_dir(&root).expect("state directory").count(),
            2,
            "new stages cleaned after publication"
        );
        let blocked = root.join("cannot-replace-directory");
        std::fs::create_dir(&blocked).expect("existing destination directory");
        assert!(write_private(&blocked, b"not published").is_err());
        assert!(
            std::fs::read_dir(&root)
                .expect("after failed publication")
                .all(|entry| !entry
                    .expect("directory entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".ghd-write-")),
            "failed publications clean their private stages"
        );
        std::fs::remove_dir_all(root).expect("clean roundtrip fixture");
    }
}
