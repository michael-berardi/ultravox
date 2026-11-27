//! Owner-only permissions for directories and files that hold user content.
//!
//! On Unix, user content is created and repaired to 0700/0600. Other platforms
//! keep their native per-user access controls; these helpers are no-ops there.

use std::path::Path;

/// Restrict an existing directory to its owner. Missing paths and symlinked
/// directories (the user's own layout) are left alone; non-directories are rejected.
pub fn protect_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::{Error, ErrorKind};
        use std::os::unix::fs::PermissionsExt;

        if !path.exists() {
            return Ok(());
        }
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            return Ok(());
        }
        if !metadata.is_dir() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("not a private directory: {}", path.display()),
            ));
        }
        if metadata.permissions().mode() & 0o777 != 0o700 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Restrict an existing regular file to its owner. Symlinks are rejected so a
/// caller can never widen the permissions of a file outside its own store.
pub fn protect_file(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::{Error, ErrorKind};
        use std::os::unix::fs::PermissionsExt;

        if !path.exists() {
            return Ok(());
        }
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("not a private file: {}", path.display()),
            ));
        }
        if metadata.permissions().mode() & 0o777 != 0o600 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn private_paths_are_owner_only_and_repaired() {
        let root = std::env::temp_dir().join(format!("uv-private-fs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("data.txt");
        std::fs::write(&file, "private").unwrap();

        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        protect_dir(&root).unwrap();
        protect_file(&file).unwrap();
        assert_eq!(std::fs::metadata(&root).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn symlinks_are_never_chmodded() {
        let root = std::env::temp_dir().join(format!("uv-private-fs-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("target.txt");
        let link = root.join("link.txt");
        std::fs::write(&target, "private").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(protect_file(&link).is_err());
        let dir = root.join("dir");
        let dir_link = root.join("dir-link");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink(&dir, &dir_link).unwrap();
        protect_dir(&dir_link).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o755);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
