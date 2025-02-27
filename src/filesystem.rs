// Copyright (c) 2024-2025 Mikko Tanner. All rights reserved.

use std::{fs::metadata, io, path::Path};

use tracing::error;

/**
Checks if the given directory path is readable.

## Arguments
* `path` - a reference to the directory path to check

## Returns
A canonicalized (absolute, resolved) path to the directory.

## Errors
This function will return an error if the given path does not exist,
is not a directory or if it fails to get metadata for the directory.
*/
pub fn check_readable_dir(path: &String) -> Result<String, io::Error> {
    let path: &Path = Path::new(path);

    if !path.exists() {
        let errmsg: String = format!("Directory {} does not exist", path.display());
        error!(errmsg);
        return Err(io::Error::new(io::ErrorKind::NotFound, errmsg));
    }

    if let Ok(metadata) = metadata(path) {
        if !metadata.is_dir() {
            let errmsg: String = format!("Not a directory: {}", path.display());
            error!(errmsg);
            return Err(io::Error::new(io::ErrorKind::InvalidInput, errmsg));
        }
    } else {
        let errmsg: String = format!("Failed to get metadata for: {}", path.display());
        error!(errmsg);
        return Err(io::Error::new(io::ErrorKind::Other, errmsg));
    };

    Ok(path.canonicalize()?.to_str().unwrap().to_string())
}
