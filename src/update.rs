//! `tirith update`: replace the running binary with a newer release.
//!
//! Releases are published by cargo-dist with an installer script per
//! platform. Updating means re-running that installer for the chosen tag,
//! pinned to the directory the running binary lives in. The download is
//! delegated to the system's `curl` (or PowerShell on Windows) so Tirith
//! itself ships no TLS stack.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Deserialize;
use thiserror::Error;
use tokio::process::Command;

/// The GitHub repository releases are published to.
pub const REPO: &str = "eabz/tirith";
/// The installer asset name, without extension. Matches the crate name.
pub const INSTALLER: &str = "tirith-mcp-installer";

/// Why an update failed.
#[derive(Debug, Error)]
pub enum UpdateError {
    /// The downloader the platform relies on is missing.
    #[error("{program} is required for updates and was not found on PATH")]
    MissingTool {
        /// `curl` or `powershell`.
        program: &'static str,
    },
    /// The downloader ran but reported failure.
    #[error("{program} failed ({status}): {stderr}")]
    ToolFailed {
        /// `curl` or `powershell`.
        program: &'static str,
        /// Exit status.
        status: String,
        /// Captured stderr, trimmed.
        stderr: String,
    },
    /// GitHub's response did not contain a release tag.
    #[error("cannot read the latest release from GitHub: {0}")]
    Parse(String),
    /// The path of the running binary could not be determined.
    #[error("cannot locate the running binary: {0}")]
    CurrentExe(#[source] std::io::Error),
    /// The installer script exited with an error.
    #[error("installer exited with {0}")]
    Installer(String),
}

/// A published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The git tag, for example `v0.1.3`.
    pub tag: String,
    /// The version without the `v`, for example `0.1.3`.
    pub version: String,
}

impl Release {
    /// Builds a release from either `0.1.3` or `v0.1.3`.
    pub fn from_tag(tag_or_version: &str) -> Self {
        let version = tag_or_version.trim().trim_start_matches('v').to_owned();
        Self {
            tag: format!("v{version}"),
            version,
        }
    }
}

#[derive(Deserialize)]
struct LatestRelease {
    tag_name: String,
}

/// Parses GitHub's "latest release" JSON into a [`Release`].
pub fn parse_latest(json: &str) -> Result<Release, UpdateError> {
    let latest: LatestRelease =
        serde_json::from_str(json).map_err(|e| UpdateError::Parse(e.to_string()))?;
    Ok(Release::from_tag(&latest.tag_name))
}

/// URL of the installer script for `tag` on this platform.
pub fn installer_url(tag: &str) -> String {
    let ext = if cfg!(windows) { "ps1" } else { "sh" };
    format!("https://github.com/{REPO}/releases/download/{tag}/{INSTALLER}.{ext}")
}

/// Compares two versions numerically by `major.minor.patch`, treating a
/// pre-release suffix as lower than the plain version.
///
/// ```
/// use std::cmp::Ordering;
/// use tirith::update::compare;
///
/// assert_eq!(compare("0.1.10", "0.1.9"), Ordering::Greater);
/// assert_eq!(compare("1.0.0-rc.1", "1.0.0"), Ordering::Less);
/// assert_eq!(compare("v0.2.0", "0.2.0"), Ordering::Equal);
/// ```
pub fn compare(a: &str, b: &str) -> Ordering {
    fn split(v: &str) -> (Vec<u64>, Option<&str>) {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = v.split_once('-').map_or((v, None), |(c, p)| (c, Some(p)));
        let nums = core
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect();
        (nums, pre)
    }
    let (na, pa) = split(a);
    let (nb, pb) = split(b);
    let width = na.len().max(nb.len());
    for i in 0..width {
        let ordering = na
            .get(i)
            .copied()
            .unwrap_or(0)
            .cmp(&nb.get(i).copied().unwrap_or(0));
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    match (pa, pb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => x.cmp(y),
    }
}

/// The directory the running binary lives in; where the update goes.
pub fn install_dir() -> Result<PathBuf, UpdateError> {
    let exe = std::env::current_exe().map_err(UpdateError::CurrentExe)?;
    let exe = exe.canonicalize().unwrap_or(exe);
    exe.parent().map(Path::to_path_buf).ok_or_else(|| {
        UpdateError::CurrentExe(std::io::Error::other("binary has no parent directory"))
    })
}

async fn fetch(url: &str) -> Result<String, UpdateError> {
    let (program, mut command) = downloader(url);
    let output = command
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => UpdateError::MissingTool { program },
            _ => UpdateError::ToolFailed {
                program,
                status: "failed to start".into(),
                stderr: e.to_string(),
            },
        })?;
    if !output.status.success() {
        return Err(UpdateError::ToolFailed {
            program,
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(not(windows))]
fn downloader(url: &str) -> (&'static str, Command) {
    let mut command = Command::new("curl");
    command.args(["-fsSL", "-H", "User-Agent: tirith-update", url]);
    ("curl", command)
}

#[cfg(windows)]
fn downloader(url: &str) -> (&'static str, Command) {
    let mut command = Command::new("powershell");
    command.args([
        "-NoProfile",
        "-Command",
        &format!("(Invoke-WebRequest -UseBasicParsing -Uri '{url}').Content"),
    ]);
    ("powershell", command)
}

/// Asks GitHub for the latest published release.
pub async fn latest_release() -> Result<Release, UpdateError> {
    let json = fetch(&format!(
        "https://api.github.com/repos/{REPO}/releases/latest"
    ))
    .await?;
    parse_latest(&json)
}

/// Runs the release installer for `tag`, installing into `dir` and leaving
/// `PATH` and shell profiles untouched. The installer's output is shown.
pub async fn install(tag: &str, dir: &Path) -> Result<(), UpdateError> {
    let url = installer_url(tag);
    let (program, mut command) = installer_command(&url, dir);
    let status = command
        .stdin(Stdio::null())
        .status()
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => UpdateError::MissingTool { program },
            _ => UpdateError::ToolFailed {
                program,
                status: "failed to start".into(),
                stderr: e.to_string(),
            },
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(UpdateError::Installer(status.to_string()))
    }
}

#[cfg(not(windows))]
fn installer_command(url: &str, dir: &Path) -> (&'static str, Command) {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(format!("curl -fsSL '{url}' | sh"))
        .env("TIRITH_MCP_UNMANAGED_INSTALL", dir);
    ("curl", command)
}

#[cfg(windows)]
fn installer_command(url: &str, dir: &Path) -> (&'static str, Command) {
    let mut command = Command::new("powershell");
    command
        .args([
            "-ExecutionPolicy",
            "Bypass",
            "-NoProfile",
            "-Command",
            &format!("irm '{url}' | iex"),
        ])
        .env("CARGO_DIST_FORCE_INSTALL_DIR", dir)
        .env("INSTALLER_NO_MODIFY_PATH", "1");
    ("powershell", command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_latest_release() {
        let release = parse_latest(r#"{"tag_name":"v0.1.3","name":"v0.1.3","assets":[]}"#).unwrap();
        assert_eq!(release.tag, "v0.1.3");
        assert_eq!(release.version, "0.1.3");
        assert!(parse_latest(r#"{"message":"Not Found"}"#).is_err());
    }

    #[test]
    fn release_from_tag_normalizes_prefix() {
        assert_eq!(Release::from_tag("0.2.0"), Release::from_tag("v0.2.0"));
        assert_eq!(Release::from_tag(" v0.2.0 ").tag, "v0.2.0");
    }

    #[test]
    fn version_comparison() {
        assert_eq!(compare("0.1.2", "0.1.2"), Ordering::Equal);
        assert_eq!(compare("0.1.3", "0.1.2"), Ordering::Greater);
        assert_eq!(compare("0.2.0", "0.1.9"), Ordering::Greater);
        assert_eq!(compare("1.0.0", "0.9.9"), Ordering::Greater);
        assert_eq!(compare("0.1", "0.1.0"), Ordering::Equal);
        assert_eq!(compare("1.0.0-rc.1", "1.0.0"), Ordering::Less);
        assert_eq!(compare("1.0.0-rc.2", "1.0.0-rc.1"), Ordering::Greater);
    }

    #[test]
    fn installer_url_points_at_the_tag() {
        let url = installer_url("v0.1.3");
        assert!(url.starts_with(
            "https://github.com/eabz/tirith/releases/download/v0.1.3/tirith-mcp-installer."
        ));
    }
}
