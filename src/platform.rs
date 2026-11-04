//! Host discovery: OS, privileges, goose installation and config paths.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const MIN_GOOSE: (u32, u32, u32) = (1, 38, 0);

#[derive(Clone, Debug)]
pub struct GooseInfo {
    pub path: PathBuf,
    pub version: Option<(u32, u32, u32)>,
    pub raw_version: String,
}

impl GooseInfo {
    pub fn version_string(&self) -> String {
        match self.version {
            Some((a, b, c)) => format!("{a}.{b}.{c}"),
            None => self.raw_version.clone(),
        }
    }

    pub fn meets_min(&self) -> bool {
        match self.version {
            Some(v) => v >= MIN_GOOSE,
            None => false,
        }
    }
}

pub fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

pub fn home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// goose's config directory: `GOOSE_PATH_ROOT/config`, else XDG.
pub fn default_config_dir() -> PathBuf {
    if let Some(root) = env::var_os("GOOSE_PATH_ROOT") {
        let root = PathBuf::from(root);
        if root.is_absolute() {
            return root.join("config");
        }
    }
    if let Some(xdg) = env::var_os("XDG_CONFIG_HOME") {
        let xdg = PathBuf::from(xdg);
        if xdg.is_absolute() {
            return xdg.join("goose");
        }
    }
    home().join(".config").join("goose")
}

pub fn config_dir(override_dir: Option<&Path>) -> PathBuf {
    match override_dir {
        Some(dir) => dir.to_path_buf(),
        None => default_config_dir(),
    }
}

pub fn os_pretty() -> String {
    let Ok(release) = fs::read_to_string("/etc/os-release") else {
        return format!("{} {}", env::consts::OS, env::consts::ARCH);
    };
    for line in release.lines() {
        if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
            return value.trim_matches('"').to_string();
        }
    }
    format!("{} {}", env::consts::OS, env::consts::ARCH)
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn goose_info() -> Option<GooseInfo> {
    let path = find_in_path("goose")?;
    let output = Command::new(&path).arg("--version").output().ok();
    let raw = output
        .as_ref()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    Some(GooseInfo {
        path,
        version: parse_version(&raw),
        raw_version: raw,
    })
}

/// Pull the first `x.y.z` token out of `goose 1.38.0` or similar.
pub fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    for token in text.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() >= 2 && parts.iter().take(2).all(|p| p.parse::<u32>().is_ok()) {
            let major = parts[0].parse().ok()?;
            let minor = parts[1].parse().ok()?;
            let patch = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);
            return Some((major, minor, patch));
        }
    }
    None
}

pub fn vpn_state() -> VpnState {
    let mut detail = Vec::new();
    let mut probed = false;

    if let Ok(output) = Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE", "connection", "show", "--active"])
        .output()
        && output.status.success()
    {
        probed = true;
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let mut fields = line.split(':');
            let name = fields.next().unwrap_or("");
            let kind = fields.next().unwrap_or("");
            if matches!(kind, "vpn" | "wireguard" | "tun" | "ipsec")
                || name.to_ascii_lowercase().contains("vpn")
                || name.to_ascii_lowercase().contains("tailscale")
            {
                detail.push(name.to_string());
            }
        }
    }

    if detail.is_empty()
        && let Ok(output) = Command::new("ip").args(["-o", "link", "show"]).output()
        && output.status.success()
    {
        probed = true;
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some(iface) = line.split(':').nth(1) {
                let iface = iface.split('@').next().unwrap_or("").trim();
                if iface.starts_with("tun")
                    || iface.starts_with("wg")
                    || iface.starts_with("tailscale")
                {
                    detail.push(iface.to_string());
                }
            }
        }
    }

    if detail.is_empty() {
        if probed {
            VpnState::Disconnected
        } else {
            VpnState::Unknown
        }
    } else {
        detail.sort();
        detail.dedup();
        VpnState::Connected(detail.join(", "))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VpnState {
    Connected(String),
    Disconnected,
    Unknown,
}

impl VpnState {
    pub fn text(&self) -> String {
        match self {
            VpnState::Connected(name) => format!("VPN active ({name})"),
            VpnState::Disconnected => "VPN not detected".to_string(),
            VpnState::Unknown => "VPN state unknown".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing() {
        assert_eq!(parse_version("goose 1.38.0"), Some((1, 38, 0)));
        assert_eq!(parse_version("goose-cli 1.45.2-beta"), Some((1, 45, 2)));
        assert_eq!(parse_version("goose 2.0"), Some((2, 0, 0)));
        assert_eq!(parse_version("no version"), None);
    }

    #[test]
    fn minimum_version() {
        let info = GooseInfo {
            path: PathBuf::from("/usr/bin/goose"),
            version: Some((1, 38, 0)),
            raw_version: "goose 1.38.0".into(),
        };
        assert!(info.meets_min());
        let old = GooseInfo {
            version: Some((1, 37, 9)),
            ..info
        };
        assert!(!old.meets_min());
    }
}
