//! The per-user daemon registry: every daemon on this machine records
//! itself here on start and removes itself on clean shutdown, so a menu
//! bar tray (or a human with `cat`) can find all of them without knowing
//! which repositories exist. One JSON file, one entry per daemon.
//!
//! The file lives outside any repository, in the user's state directory:
//! `~/Library/Application Support/tirith/daemons.json` on macOS,
//! `$XDG_STATE_HOME/tirith/daemons.json` (default `~/.local/state`)
//! elsewhere. Entries for daemons that died without cleaning up are
//! dropped by [`Registry::prune`], which the tray runs on every poll.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The registry file name inside the state directory.
const FILE: &str = "daemons.json";

/// One running daemon, as it registered itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonEntry {
    /// The repository root the daemon serves, canonical.
    pub root: PathBuf,
    /// The MCP endpoint.
    pub url: String,
    /// The dashboard page.
    pub dashboard_url: String,
    /// The daemon's process id.
    pub pid: u32,
    /// The daemon's crate version.
    pub version: String,
    /// When it started.
    pub started_at: DateTime<Utc>,
}

impl DaemonEntry {
    /// The last component of the root, which is what a menu shows.
    ///
    /// ```
    /// use std::path::PathBuf;
    /// use chrono::Utc;
    /// use tirith::registry::DaemonEntry;
    ///
    /// let entry = DaemonEntry {
    ///     root: PathBuf::from("/Users/me/code/tirith"),
    ///     url: "http://127.0.0.1:7477/mcp".into(),
    ///     dashboard_url: "http://127.0.0.1:7477/".into(),
    ///     pid: 1,
    ///     version: "1.0.0".into(),
    ///     started_at: Utc::now(),
    /// };
    /// assert_eq!(entry.folder_name(), "tirith");
    /// ```
    pub fn folder_name(&self) -> String {
        self.root.file_name().map_or_else(
            || self.root.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }
}

/// Why the registry could not be read or written.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// No home or state directory could be determined.
    #[error("no state directory: none of XDG_STATE_HOME, LOCALAPPDATA, or HOME is set")]
    NoStateDir,
    /// A filesystem operation failed.
    #[error("failed to {action} {path}: {source}")]
    Io {
        /// What was being done.
        action: &'static str,
        /// The file involved.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
    /// The file was not valid JSON for a registry.
    #[error("{path} is not a daemon registry: {source}")]
    Parse {
        /// The file involved.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: serde_json::Error,
    },
}

/// The registry file and the entries it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    path: PathBuf,
    entries: Vec<DaemonEntry>,
}

impl Registry {
    /// The per-user registry path for this platform.
    ///
    /// macOS: `~/Library/Application Support/tirith/daemons.json`.
    /// Windows: `%LOCALAPPDATA%\tirith\daemons.json`.
    /// Elsewhere: `$XDG_STATE_HOME/tirith/daemons.json`, defaulting to
    /// `~/.local/state/tirith/daemons.json`.
    pub fn default_path() -> Result<PathBuf, RegistryError> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let dir = if cfg!(target_os = "macos") {
            home.map(|h| h.join("Library/Application Support"))
        } else if cfg!(target_os = "windows") {
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_STATE_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .or_else(|| home.map(|h| h.join(".local/state")))
        };
        dir.map(|d| d.join("tirith").join(FILE))
            .ok_or(RegistryError::NoStateDir)
    }

    /// Loads the registry at `path`; a missing file is an empty registry.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, RegistryError> {
        let path = path.into();
        let entries = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|source| RegistryError::Parse {
                path: path.clone(),
                source,
            })?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(source) => {
                return Err(RegistryError::Io {
                    action: "read",
                    path,
                    source,
                });
            }
        };
        Ok(Self { path, entries })
    }

    /// Where this registry is stored.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The entries, in registration order.
    pub fn entries(&self) -> &[DaemonEntry] {
        &self.entries
    }

    /// Adds or replaces the entry for `entry.pid` (and drops any older
    /// entry for the same root, which can only be a dead daemon), then
    /// writes the file.
    pub fn register(&mut self, entry: DaemonEntry) -> Result<(), RegistryError> {
        self.entries
            .retain(|e| e.pid != entry.pid && e.root != entry.root);
        self.entries.push(entry);
        self.save()
    }

    /// Removes the entry for `pid`, if any, and writes the file.
    pub fn unregister(&mut self, pid: u32) -> Result<(), RegistryError> {
        let before = self.entries.len();
        self.entries.retain(|e| e.pid != pid);
        if self.entries.len() == before {
            return Ok(());
        }
        self.save()
    }

    /// Drops every entry for which `alive` returns false (the caller
    /// checks `/api/health` and that the root still exists) and writes
    /// the file when anything changed. Returns how many were dropped.
    pub fn prune(&mut self, alive: impl Fn(&DaemonEntry) -> bool) -> Result<usize, RegistryError> {
        let before = self.entries.len();
        self.entries.retain(|e| alive(e));
        let dropped = before - self.entries.len();
        if dropped > 0 {
            self.save()?;
        }
        Ok(dropped)
    }

    /// Writes the file atomically (temp file plus rename) so a reader
    /// never sees a half-written registry.
    fn save(&self) -> Result<(), RegistryError> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(io_err("create", dir))?;
        }
        let tmp = self
            .path
            .with_extension(format!("json.{}.tmp", std::process::id()));
        let bytes =
            serde_json::to_vec_pretty(&self.entries).map_err(|source| RegistryError::Parse {
                path: self.path.clone(),
                source,
            })?;
        fs::write(&tmp, bytes).map_err(io_err("write", &tmp))?;
        fs::rename(&tmp, &self.path).map_err(io_err("rename", &tmp))
    }
}

/// An `Io` error for `action` on `path`, for `map_err`.
fn io_err(action: &'static str, path: &Path) -> impl FnOnce(io::Error) -> RegistryError {
    let path = path.to_path_buf();
    move |source| RegistryError::Io {
        action,
        path,
        source,
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn entry(root: &str, pid: u32) -> DaemonEntry {
        DaemonEntry {
            root: PathBuf::from(root),
            url: format!("http://127.0.0.1:{pid}/mcp"),
            dashboard_url: format!("http://127.0.0.1:{pid}/"),
            pid,
            version: "1.0.0".into(),
            started_at: Utc.with_ymd_and_hms(2026, 9, 16, 3, 0, 0).unwrap(),
        }
    }

    #[test]
    fn a_missing_file_is_an_empty_registry_and_register_creates_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/tirith/daemons.json");
        let mut registry = Registry::load(&path).unwrap();
        assert!(registry.entries().is_empty());
        registry.register(entry("/repos/a", 10)).unwrap();
        registry.register(entry("/repos/b", 11)).unwrap();
        let reloaded = Registry::load(&path).unwrap();
        assert_eq!(reloaded.entries().len(), 2);
        assert_eq!(reloaded.entries()[0].folder_name(), "a");
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn registering_the_same_root_or_pid_again_replaces_the_old_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = Registry::load(dir.path().join("daemons.json")).unwrap();
        registry.register(entry("/repos/a", 10)).unwrap();
        registry.register(entry("/repos/a", 12)).unwrap();
        assert_eq!(registry.entries().len(), 1);
        assert_eq!(registry.entries()[0].pid, 12);
        let mut moved = entry("/repos/moved", 12);
        moved.version = "1.0.1".into();
        registry.register(moved).unwrap();
        assert_eq!(registry.entries().len(), 1);
        assert_eq!(registry.entries()[0].folder_name(), "moved");
    }

    #[test]
    fn unregister_and_prune_drop_entries_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemons.json");
        let mut registry = Registry::load(&path).unwrap();
        for (root, pid) in [("/repos/a", 1), ("/repos/b", 2), ("/repos/c", 3)] {
            registry.register(entry(root, pid)).unwrap();
        }
        registry.unregister(2).unwrap();
        registry.unregister(99).unwrap();
        assert_eq!(Registry::load(&path).unwrap().entries().len(), 2);
        let dropped = registry.prune(|e| e.pid != 1).unwrap();
        assert_eq!(dropped, 1);
        assert_eq!(registry.prune(|_| true).unwrap(), 0);
        let reloaded = Registry::load(&path).unwrap();
        assert_eq!(reloaded.entries().len(), 1);
        assert_eq!(reloaded.entries()[0].pid, 3);
    }

    #[test]
    fn a_corrupt_file_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemons.json");
        fs::write(&path, b"{ not json").unwrap();
        assert!(matches!(
            Registry::load(&path),
            Err(RegistryError::Parse { .. })
        ));
    }

    #[test]
    fn default_path_follows_the_platform() {
        let path = Registry::default_path().unwrap();
        assert!(path.ends_with("tirith/daemons.json"), "{}", path.display());
        if cfg!(target_os = "macos") {
            assert!(
                path.to_string_lossy()
                    .contains("Library/Application Support")
            );
        }
    }
}
