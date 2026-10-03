use super::{canonical_uuid, Marker, MARKER};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub(in crate::backend) fn create_directory(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("Development profile needs a parent directory")?;
    if !path.is_absolute()
        || parent
            .canonicalize()
            .map_err(|_| "Development profile parent is unavailable")?
            != parent
        || path.file_name().is_none()
        || path.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err("Development profile needs an absolute canonical path without symlinks".into());
    }
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|_| "Development profile creation requires a new directory")?;
    validate_directory(path)
}

fn validate_directory(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .canonicalize()
            .map_err(|_| "Development profile is unavailable")?
            != path
    {
        return Err("Development profile must be canonical without symlinks".into());
    }
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| "Development profile is unavailable")?;
    if !metadata.is_dir() {
        return Err("Development profile is not a directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o777 != 0o700 || metadata.uid() != unsafe { libc::geteuid() } {
            return Err("Development profile must be owned by this user with mode 0700".into());
        }
    }
    Ok(())
}

fn validate_file(file: &File) -> Result<(), String> {
    let metadata = file
        .metadata()
        .map_err(|_| "Development profile file is unavailable")?;
    if !metadata.is_file() {
        return Err("Development profile contains a foreign file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1
            || metadata.mode() & 0o777 != 0o600
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(
                "Development profile files must be private, owned and not hard-linked".into(),
            );
        }
    }
    Ok(())
}

fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    options
}

pub(in crate::backend) fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = options()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "Development profile file already exists or cannot be created")?;
    validate_file(&file)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Development profile file could not be committed".into())
}

pub(in crate::backend) fn lock(path: &Path) -> Result<File, String> {
    let file = options()
        .read(true)
        .write(true)
        .create(true)
        .open(path.join(".dbunk-native-lock"))
        .map_err(|_| "Development profile lock is unavailable")?;
    validate_file(&file)?;
    file.try_lock()
        .map_err(|_| "Development profile is already in use")?;
    Ok(file)
}

/// Shared filesystem primitives; each constructor supplies exactly its own marker name.
pub(in crate::backend) fn validate_files(path: &Path, marker_name: &str) -> Result<File, String> {
    validate_directory(path)?;
    for entry in std::fs::read_dir(path).map_err(|_| "Development profile is unavailable")? {
        let entry = entry.map_err(|_| "Development profile is unavailable")?;
        if !entry
            .file_type()
            .map_err(|_| "Development profile file is unavailable")?
            .is_file()
            || !entry.file_name().to_str().is_some_and(|name| {
                [
                    marker_name,
                    ".dbunk-native-lock",
                    "launch.json",
                    "dbunk.sqlite",
                    "dbunk.sqlite-wal",
                    "dbunk.sqlite-shm",
                ]
                .contains(&name)
            })
        {
            return Err("Development profile contains a foreign file or symlink".into());
        }
        let file = options()
            .read(true)
            .open(entry.path())
            .map_err(|_| "Development profile file is unavailable")?;
        validate_file(&file)?;
    }
    lock(path)
}

pub(in crate::backend) fn read_marker(path: &Path, marker_name: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    options()
        .read(true)
        .open(path.join(marker_name))
        .map_err(|_| "Native ownership marker missing")?
        .take(8193)
        .read_to_end(&mut bytes)
        .map_err(|_| "Development marker could not be read")?;
    if bytes.len() > 8192 {
        return Err("Development marker is too large".into());
    }
    Ok(bytes)
}

pub(super) fn validate(path: &Path) -> Result<(Marker, File), String> {
    let lock = validate_files(path, MARKER)?;
    let bytes = read_marker(path, MARKER)?;
    let marker: Marker =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid development marker")?;
    if marker.version != 1 || marker.path != path {
        return Err("Development marker version or path does not match".into());
    }
    canonical_uuid(&marker.profile_id)?;
    canonical_uuid(&marker.credential_namespace)?;
    if marker.profile_id == marker.credential_namespace {
        return Err("Development credential namespace must be independently generated".into());
    }
    marker.fixtures.validate()?;
    Ok((marker, lock))
}

pub(in crate::backend) fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|_| "Development directory could not be committed".into())
}
