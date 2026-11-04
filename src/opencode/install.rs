//! Installing the OpenCode CLI when it is not already on PATH.
//!
//! OpenCode publishes standalone Linux tarballs for each architecture. The
//! installer downloads the current release to a temporary directory, extracts
//! the `opencode` binary, installs it (default `~/.local/bin`, no privileges
//! needed), and then verifies the installed binary reports a supported version.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::{Fail, Result, code};
use crate::platform::{self, ToolInfo};

/// Overridden in tests and for a mirror. The installer appends the platform
/// file name, e.g. `opencode-linux-x64.tar.gz`.
pub const DEFAULT_BASE: &str = "https://opencode.ai/update/api/latest/cli/opencode";

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Clone, Debug)]
pub struct Outcome {
    pub info: ToolInfo,
    pub installed_now: bool,
    pub warnings: Vec<String>,
}

/// Make sure a supported `opencode` is available, installing it when missing.
pub fn ensure(install_dir: Option<&Path>, allow_install: bool) -> Result<Outcome> {
    if let Some(info) = platform::opencode_info()
        && info.meets_min()
    {
        return Ok(Outcome {
            info,
            installed_now: false,
            warnings: Vec::new(),
        });
    }

    if let Some(info) = platform::opencode_info()
        && !info.meets_min()
    {
        return Err(Fail::new(
            code::TOOL,
            format!(
                "opencode {} is older than the required {}.",
                info.version_string(),
                version_text(platform::MIN_OPENCODE)
            ),
        )
        .hint("Run `opencode upgrade`, or remove that binary and run this installer again."));
    }

    if !allow_install {
        return Err(Fail::new(code::TOOL, "opencode is not installed.")
            .hint("Drop --no-install to let this tool download the current release.")
            .hint("Or install it yourself: curl -fsSL https://opencode.ai/v2/install | bash"));
    }

    install(install_dir)
}

fn install(install_dir: Option<&Path>) -> Result<Outcome> {
    if std::env::consts::OS != "linux" {
        return Err(Fail::new(
            code::TOOL,
            format!(
                "Automatic installation supports Linux; this host is {}.",
                std::env::consts::OS
            ),
        )
        .hint("Install OpenCode from https://opencode.ai and run this tool again."));
    }

    let dir = install_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(platform::default_install_dir);
    let file = platform_file()?;
    let url = format!(
        "{}/{}",
        std::env::var("OPENCODE_INSTALL_BASE").unwrap_or_else(|_| DEFAULT_BASE.to_string()),
        file
    );

    eprintln!("Installing OpenCode from {url}");

    let work = work_dir();
    fs::create_dir_all(&work).map_err(|error| {
        Fail::new(
            code::WRITE,
            format!("Could not create {}: {error}", work.display()),
        )
    })?;

    let archive = work.join(&file);
    let result = (|| -> Result<ToolInfo> {
        download(&url, &archive)?;
        extract(&archive, &work)?;
        let binary = find_binary(&work).ok_or_else(|| {
            Fail::new(
                code::TOOL,
                "The OpenCode download did not contain an `opencode` binary.",
            )
            .hint("Try again, or install OpenCode from https://opencode.ai.")
        })?;

        fs::create_dir_all(&dir).map_err(|error| {
            Fail::new(
                code::WRITE,
                format!("Could not create {}: {error}", dir.display()),
            )
            .hint("Pass --install-dir with a directory you can write to.")
        })?;
        let target = dir.join("opencode");
        fs::copy(&binary, &target).map_err(|error| {
            Fail::new(
                code::WRITE,
                format!("Could not install to {}: {error}", target.display()),
            )
            .hint("Pass --install-dir with a directory you can write to.")
        })?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).map_err(|error| {
            Fail::new(
                code::WRITE,
                format!("Could not make {} executable: {error}", target.display()),
            )
        })?;

        let info = ToolInfo {
            path: target,
            version: None,
            raw_version: String::new(),
        };
        let output = std::process::Command::new(&info.path)
            .arg("--version")
            .output()
            .map_err(|error| {
                Fail::new(
                    code::TOOL,
                    format!("Could not run the installed OpenCode: {error}"),
                )
            })?;
        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let info = ToolInfo {
            path: info.path,
            version: platform::parse_version(&raw),
            raw_version: raw,
        };
        if !info.meets_min() {
            return Err(Fail::new(
                code::TOOL,
                format!(
                    "The installed OpenCode reported '{}', not a supported version.",
                    info.raw_version
                ),
            ));
        }
        Ok(info)
    })();

    let _ = fs::remove_dir_all(&work);
    let info = result?;

    let mut warnings = Vec::new();
    if !on_path(&info.path) {
        warnings.push(format!(
            "{} is not on PATH; add {} to PATH to run `opencode` directly.",
            info.path.display(),
            info.path
                .parent()
                .map(|parent| parent.display().to_string())
                .unwrap_or_default()
        ));
    }

    Ok(Outcome {
        info,
        installed_now: true,
        warnings,
    })
}

fn platform_file() -> Result<String> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => {
            return Err(Fail::new(
                code::TOOL,
                format!("No OpenCode release is published for {other}."),
            )
            .hint("Install OpenCode from https://opencode.ai and run this tool again."));
        }
    };
    let musl = if is_musl() { "-musl" } else { "" };
    Ok(format!("opencode-linux-{arch}{musl}.tar.gz"))
}

fn is_musl() -> bool {
    std::process::Command::new("ldd")
        .arg("--version")
        .output()
        .ok()
        .map(|output| {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            text.to_ascii_lowercase().contains("musl")
        })
        .unwrap_or(false)
}

fn download(url: &str, path: &Path) -> Result<()> {
    // ureq has no request timeout on the body stream; the process-wide default
    // keeps a stalled mirror from hanging the installer forever.
    let mut response = ureq::get(url)
        .config()
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .build()
        .call()
        .map_err(|error| {
            Fail::new(
                code::NETWORK,
                format!("Could not download OpenCode: {error}"),
            )
            .hint("Check the network or proxy, then run the installer again.")
        })?;
    let mut file = fs::File::create(path).map_err(|error| {
        Fail::new(
            code::WRITE,
            format!("Could not write {}: {error}", path.display()),
        )
    })?;
    let mut reader = response.body_mut().as_reader();
    std::io::copy(&mut reader, &mut file).map_err(|error| {
        Fail::new(
            code::NETWORK,
            format!("The OpenCode download was interrupted: {error}"),
        )
    })?;
    file.flush()
        .map_err(|error| Fail::new(code::WRITE, format!("Could not save the download: {error}")))
}

fn extract(archive: &Path, dir: &Path) -> Result<()> {
    let file = fs::File::open(archive).map_err(|error| {
        Fail::new(
            code::WRITE,
            format!("Could not open {}: {error}", archive.display()),
        )
    })?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    tar.unpack(dir).map_err(|error| {
        Fail::new(
            code::TOOL,
            format!("Could not unpack the OpenCode download: {error}"),
        )
    })
}

fn find_binary(dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_binary(&path) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|name| name == "opencode") {
            return Some(path);
        }
    }
    None
}

fn work_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "rhai-opencode-install-{}-{nanos}",
        std::process::id()
    ))
}

fn on_path(binary: &Path) -> bool {
    let Some(dir) = binary.parent() else {
        return false;
    };
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|entry| entry == dir))
        .unwrap_or(false)
}

fn version_text((a, b, c): (u32, u32, u32)) -> String {
    format!("{a}.{b}.{c}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_files_cover_the_two_supported_arches() {
        let name = platform_file().unwrap();
        assert!(name.starts_with("opencode-linux-"));
        assert!(name.ends_with(".tar.gz"));
        assert!(name.contains("x64") || name.contains("arm64"));
    }

    #[test]
    fn work_directories_are_unique() {
        assert_ne!(work_dir(), work_dir());
    }
}
