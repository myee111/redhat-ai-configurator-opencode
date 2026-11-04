//! Shared filesystem helpers: atomic writes and timestamped backups.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub fn timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

pub fn write_atomic(path: &Path, contents: &str, mode: u32) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
    }
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Copy a file aside before it is replaced. Returns the backup path.
pub fn backup(path: &Path) -> io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let base = format!("{}.bak-{}", path.display(), timestamp());
    let mut candidate = PathBuf::from(&base);
    let mut counter = 1;
    while candidate.exists() {
        candidate = PathBuf::from(format!("{base}-{counter}"));
        counter += 1;
    }
    fs::copy(path, &candidate)?;
    let mode = fs::metadata(path)?.permissions().mode() & 0o7777;
    fs::set_permissions(&candidate, fs::Permissions::from_mode(mode))?;
    Ok(Some(candidate))
}

pub fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let mode = fs::metadata(dir)?.permissions().mode() & 0o7777;
    if mode != 0o700 {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn is_private_file(path: &Path) -> io::Result<bool> {
    let meta = fs::metadata(path)?;
    Ok(meta.uid() == unsafe { libc::geteuid() } && meta.permissions().mode() & 0o077 == 0)
}
