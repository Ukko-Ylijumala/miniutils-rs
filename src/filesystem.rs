// Copyright (c) 2024-2026 Mikko Tanner. All rights reserved.

use std::{
    fs::{metadata, read_dir, Metadata},
    io,
    path::{Path, PathBuf},
};

use tracing::error;

/**
Checks if the given directory path is readable.

## Arguments
* `path` - a reference to the directory path to check

## Returns
A canonicalized (absolute, resolved) path to the directory.

## Errors
This function will return an error if the given path does not exist,
is not a directory, cannot be listed (f.ex. lacking permissions), or if
it fails to get metadata for the directory.
*/
pub fn check_readable_dir<P: AsRef<Path>>(path: P) -> Result<PathBuf, io::Error> {
    let path: &Path = path.as_ref();

    /*
    A single metadata() call instead of exists() + metadata(): exists() maps
    every error to `false`, so f.ex. EACCES on a parent directory used to be
    reported as "does not exist".
    */
    let md: Metadata = match metadata(path) {
        Ok(md) => md,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let errmsg: String = format!("Directory {} does not exist", path.display());
            return Err(log_err(io::ErrorKind::NotFound, errmsg));
        }
        Err(e) => {
            let errmsg: String = format!("Failed to get metadata for: {}: {e}", path.display());
            return Err(log_err(e.kind(), errmsg));
        }
    };

    if !md.is_dir() {
        let errmsg: String = format!("Not a directory: {}", path.display());
        return Err(log_err(io::ErrorKind::InvalidInput, errmsg));
    }

    // metadata says nothing about permissions, so actually try to list the directory
    if let Err(e) = read_dir(path) {
        let errmsg: String = format!("Directory {} is not readable: {e}", path.display());
        return Err(log_err(e.kind(), errmsg));
    }

    path.canonicalize()
}

/// Log an error message and wrap it into an [io::Error] of the given kind.
fn log_err(kind: io::ErrorKind, errmsg: String) -> io::Error {
    error!("{errmsg}");
    io::Error::new(kind, errmsg)
}

/* ######################################################################### */

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::{env, fs, process};

    /// Fresh scratch dir under the system temp dir, unique per test and process.
    fn scratch(name: &str) -> PathBuf {
        let dir: PathBuf = env::temp_dir().join(format!("miniutils-fs-{}-{name}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_readable_dir_ok() {
        let dir: PathBuf = scratch("ok");
        let res: PathBuf = check_readable_dir(&dir).unwrap();
        assert_eq!(res, dir.canonicalize().unwrap());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_readable_dir_missing() {
        let dir: PathBuf = scratch("missing");
        let err: io::Error = check_readable_dir(dir.join("nope")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_readable_dir_not_a_dir() {
        let dir: PathBuf = scratch("file");
        let file: PathBuf = dir.join("file");
        fs::write(&file, b"").unwrap();
        let err: io::Error = check_readable_dir(&file).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn test_readable_dir_no_permission() {
        let dir: PathBuf = scratch("noperm");
        let locked: PathBuf = dir.join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

        // root ignores permission bits, so there is nothing to test there
        if read_dir(&locked).is_err() {
            let err: io::Error = check_readable_dir(&locked).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
            let err: io::Error = check_readable_dir(locked.join("child")).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        }

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }
}
