//! The per-user daemon registry: every daemon on this machine records
//! itself here on start and removes itself on clean shutdown, so a menu
//! bar tray (or a human with `cat`) can find all of them without knowing
//! which repositories exist. One JSON file, one entry per daemon.
//!
//! The file lives outside any repository, in the user's state directory:
//! `~/Library/Application Support/tirith/daemons.json` on macOS,
//! `$XDG_STATE_HOME/tirith/daemons.json` (default `~/.local/state`)
//! elsewhere. `TIRITH_STATE_DIR` replaces that directory on every
//! platform, which keeps test daemons out of the user's menu bar. Entries
//! for daemons that died without cleaning up are dropped by
//! [`Registry::prune`], which the tray runs on every poll. A running daemon
//! also puts its entry back every minute if something removed it (a tray
//! that took a busy daemon for a dead one, a deleted file), through
//! `Registration`.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// The registry file name inside the state directory.
const FILE: &str = "daemons.json";

/// The environment variable that replaces the per-user state directory:
/// the registry (and the tray's lock beside it) live directly inside it.
const STATE_DIR_ENV: &str = "TIRITH_STATE_DIR";

/// How often a running daemon checks that its entry is still in the
/// registry and puts it back if not.
pub(crate) const REREGISTER_EVERY: Duration = Duration::from_secs(60);

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
    #[error(
        "no state directory: none of TIRITH_STATE_DIR, XDG_STATE_HOME, LOCALAPPDATA, or HOME is set"
    )]
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
    /// `$TIRITH_STATE_DIR/daemons.json` when that variable is set and not
    /// empty, on every platform. Otherwise:
    /// macOS: `~/Library/Application Support/tirith/daemons.json`.
    /// Windows: `%LOCALAPPDATA%\tirith\daemons.json`.
    /// Elsewhere: `$XDG_STATE_HOME/tirith/daemons.json`, defaulting to
    /// `~/.local/state/tirith/daemons.json`.
    pub fn default_path() -> Result<PathBuf, RegistryError> {
        Self::path_from_env(std::env::var_os)
    }

    /// [`Registry::default_path`] with the environment read through `var`,
    /// so the rules are testable without changing the process environment.
    fn path_from_env(
        var: impl Fn(&'static str) -> Option<OsString>,
    ) -> Result<PathBuf, RegistryError> {
        if let Some(dir) = var(STATE_DIR_ENV).filter(|d| !d.is_empty()) {
            let dir = PathBuf::from(dir);
            // Relative to the working directory, like any path argument.
            return std::path::absolute(&dir)
                .map(|d| d.join(FILE))
                .map_err(io_err("resolve", &dir));
        }
        let home = var("HOME").map(PathBuf::from);
        let dir = if cfg!(target_os = "macos") {
            home.map(|h| h.join("Library/Application Support"))
        } else if cfg!(target_os = "windows") {
            var("LOCALAPPDATA").map(PathBuf::from)
        } else {
            var("XDG_STATE_HOME")
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
    /// writes the file. Like every change, it applies to the file as it is
    /// now, not as it was loaded, so concurrent daemons keep each other's
    /// entries.
    pub fn register(&mut self, entry: DaemonEntry) -> Result<(), RegistryError> {
        self.update(|entries| {
            entries.retain(|e| e.pid != entry.pid && e.root != entry.root);
            entries.push(entry);
            true
        })
    }

    /// Removes the entry for `pid`, if any, and writes the file.
    pub fn unregister(&mut self, pid: u32) -> Result<(), RegistryError> {
        self.update(|entries| {
            let before = entries.len();
            entries.retain(|e| e.pid != pid);
            entries.len() != before
        })
    }

    /// Drops every entry for which `alive` returns false (the tray drops
    /// daemons whose process or repository is gone) and writes the file
    /// when anything changed. Returns how many were dropped.
    /// `alive` also sees entries registered after this registry was
    /// loaded, so it should keep what it has not checked.
    pub fn prune(&mut self, alive: impl Fn(&DaemonEntry) -> bool) -> Result<usize, RegistryError> {
        let mut dropped = 0;
        self.update(|entries| {
            let before = entries.len();
            entries.retain(|e| alive(e));
            dropped = before - entries.len();
            dropped > 0
        })?;
        Ok(dropped)
    }

    /// Puts `entry` back unless it is already there, and writes the file
    /// only when it did; returns whether it did. A running daemon calls this
    /// periodically, so an entry that a tray pruned by mistake returns. The
    /// newest daemon for a root wins: an entry for the same root from a
    /// daemon that started later is kept, and one from a daemon that started
    /// earlier (or a dead daemon whose pid this one reuses) is replaced.
    fn register_if_missing(&mut self, entry: &DaemonEntry) -> Result<bool, RegistryError> {
        let mut added = false;
        self.update(|entries| {
            let newer_holds_root = entries.iter().any(|e| {
                e.root == entry.root && e.pid != entry.pid && e.started_at >= entry.started_at
            });
            if newer_holds_root || entries.contains(entry) {
                return false;
            }
            entries.retain(|e| e.pid != entry.pid && e.root != entry.root);
            entries.push(entry.clone());
            added = true;
            true
        })?;
        Ok(added)
    }

    /// Re-reads the file and applies `change` under an exclusive lock on
    /// `daemons.json.lock`, then writes the file if `change` returns true.
    /// Without the lock, two daemons starting together each wrote the list
    /// they had read, and one of them vanished from the tray (ADR-0032).
    fn update(
        &mut self,
        change: impl FnOnce(&mut Vec<DaemonEntry>) -> bool,
    ) -> Result<(), RegistryError> {
        let lock_path = self.path.with_extension("json.lock");
        if let Some(dir) = lock_path.parent() {
            fs::create_dir_all(dir).map_err(io_err("create", dir))?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(io_err("open", &lock_path))?;
        // Held for one read and one write; the kernel drops it if this
        // process dies in between.
        lock.lock().map_err(io_err("lock", &lock_path))?;
        self.entries = Self::load(self.path.clone())?.entries;
        let changed = change(&mut self.entries);
        let saved = if changed { self.save() } else { Ok(()) };
        drop(lock);
        saved
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

/// A daemon's entry in the registry for as long as the daemon runs:
/// written by [`Registration::start`], put back every `every` if it went
/// missing, and removed by [`Registration::leave`]. Registry errors are
/// logged, never returned: a registry that cannot be written must not stop
/// a daemon. Dropping it without `leave` stops the re-registration and
/// leaves the entry for the tray to prune.
#[derive(Debug)]
pub(crate) struct Registration {
    path: PathBuf,
    pid: u32,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Registration {
    /// Registers `entry` in the registry at `path` before returning, then
    /// checks every `every` that it is still there.
    pub(crate) async fn start(path: PathBuf, entry: DaemonEntry, every: Duration) -> Self {
        let pid = entry.pid;
        let first = entry.clone();
        let registered = blocking(&path, move |registry| registry.register(first)).await;
        if let Err(error) = registered {
            tracing::warn!(%error, "could not register in the daemon registry");
        }
        let (stop, mut stopped) = oneshot::channel::<()>();
        // `interval_at` panics on a zero period.
        let every = every.max(Duration::from_millis(1));
        let task = tokio::spawn({
            let path = path.clone();
            async move {
                let mut ticks =
                    tokio::time::interval_at(tokio::time::Instant::now() + every, every);
                ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tokio::select! {
                        _ = &mut stopped => break,
                        _ = ticks.tick() => {}
                    }
                    let entry = entry.clone();
                    match blocking(&path, move |registry| registry.register_if_missing(&entry))
                        .await
                    {
                        Ok(true) => tracing::info!("put this daemon back in the daemon registry"),
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!(%error, "could not re-register in the daemon registry");
                        }
                    }
                }
            }
        });
        Self {
            path,
            pid,
            stop,
            task,
        }
    }

    /// Stops re-registering, waits for a check in flight (which could
    /// otherwise put the entry back after it is removed), and removes the
    /// entry.
    pub(crate) async fn leave(self) {
        let Self {
            path,
            pid,
            stop,
            task,
        } = self;
        drop(stop);
        if let Err(error) = task.await {
            tracing::warn!(%error, "the daemon registry task failed");
        }
        if let Err(error) = blocking(&path, move |registry| registry.unregister(pid)).await {
            tracing::warn!(%error, "could not leave the daemon registry");
        }
    }
}

/// Loads the registry at `path` and applies `change` on the blocking pool,
/// since both read and write files.
async fn blocking<T: Send + 'static>(
    path: &Path,
    change: impl FnOnce(&mut Registry) -> Result<T, RegistryError> + Send + 'static,
) -> Result<T, RegistryError> {
    let owned = path.to_path_buf();
    tokio::task::spawn_blocking(move || Registry::load(owned).and_then(|mut r| change(&mut r)))
        .await
        .unwrap_or_else(|join| Err(io_err("update", path)(io::Error::other(join))))
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
        let leftovers: Vec<PathBuf> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
    }

    #[test]
    fn daemons_registering_from_stale_snapshots_keep_each_others_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tirith/daemons.json");
        // Three processes load the empty registry at the same moment.
        let mut first = Registry::load(&path).unwrap();
        let mut second = Registry::load(&path).unwrap();
        let mut third = Registry::load(&path).unwrap();
        first.register(entry("/repos/a", 10)).unwrap();
        second.register(entry("/repos/b", 11)).unwrap();
        let on_disk = || Registry::load(&path).unwrap().entries().to_vec();
        assert_eq!(
            on_disk(),
            vec![entry("/repos/a", 10), entry("/repos/b", 11)]
        );
        // Unregister and prune judge the file as it is now, not as loaded.
        third.unregister(10).unwrap();
        assert_eq!(on_disk(), vec![entry("/repos/b", 11)]);
        assert_eq!(first.prune(|e| e.pid != 11).unwrap(), 1);
        assert!(on_disk().is_empty());
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
    fn putting_an_entry_back_writes_only_when_missing_and_the_newest_daemon_keeps_a_root() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemons.json");
        let on_disk = || Registry::load(&path).unwrap().entries().to_vec();
        let mut registry = Registry::load(&path).unwrap();
        let older = entry("/repos/a", 10);
        assert!(registry.register_if_missing(&older).unwrap(), "missing");
        assert!(!registry.register_if_missing(&older).unwrap(), "present");
        assert_eq!(on_disk(), vec![older.clone()]);

        // A newer daemon took the root over: the older one leaves it be.
        let mut newer = entry("/repos/a", 11);
        newer.started_at += chrono::Duration::minutes(1);
        registry.register(newer.clone()).unwrap();
        assert!(!registry.register_if_missing(&older).unwrap());
        assert_eq!(on_disk(), vec![newer.clone()]);

        // The newer one replaces an older entry for its root...
        registry.register(older.clone()).unwrap();
        assert!(registry.register_if_missing(&newer).unwrap());
        assert_eq!(on_disk(), vec![newer.clone()]);
        // ...and a dead daemon's entry under the pid it now has.
        registry.register(entry("/repos/gone", 11)).unwrap();
        assert!(registry.register_if_missing(&newer).unwrap());
        assert_eq!(on_disk(), vec![newer]);
    }

    /// Polls `done` until it holds, for up to five seconds.
    async fn eventually(done: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "timed out");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn a_running_daemon_puts_its_pruned_entry_back_until_it_leaves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/tirith/daemons.json");
        let on_disk = || Registry::load(&path).unwrap().entries().to_vec();
        let ours = entry("/repos/a", 10);
        let every = Duration::from_millis(20);
        let registration = Registration::start(path.clone(), ours.clone(), every).await;
        assert_eq!(on_disk(), vec![ours.clone()], "registered on start");

        // A tray took the busy daemon for a dead one.
        Registry::load(&path).unwrap().unregister(ours.pid).unwrap();
        assert!(on_disk().is_empty());
        eventually(|| on_disk() == vec![ours.clone()]).await;

        registration.leave().await;
        assert!(on_disk().is_empty(), "left");
        tokio::time::sleep(every * 5).await;
        assert!(on_disk().is_empty(), "never put back after leaving");
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

    /// An environment holding only `vars`.
    fn env(vars: &[(&str, OsString)]) -> impl Fn(&'static str) -> Option<OsString> {
        let vars: Vec<(String, OsString)> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect();
        move |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }

    #[test]
    fn default_path_follows_the_platform() {
        assert!(Registry::default_path().unwrap().ends_with(FILE));
        let home = std::env::temp_dir().join("home");
        let state = std::env::temp_dir().join("xdg-state");
        let path = Registry::path_from_env(env(&[
            ("HOME", home.clone().into()),
            ("XDG_STATE_HOME", state.clone().into()),
            ("LOCALAPPDATA", state.clone().into()),
        ]))
        .unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(
                path,
                home.join("Library/Application Support/tirith/daemons.json")
            );
        } else {
            assert_eq!(path, state.join("tirith/daemons.json"));
        }
        assert!(matches!(
            Registry::path_from_env(env(&[])),
            Err(RegistryError::NoStateDir)
        ));
    }

    #[test]
    fn tirith_state_dir_replaces_the_per_user_directory() {
        let home = std::env::temp_dir().join("home");
        let state = std::env::temp_dir().join("tirith-test-state");
        let with = |value: OsString| {
            Registry::path_from_env(env(&[
                ("HOME", home.clone().into()),
                ("XDG_STATE_HOME", home.clone().into()),
                ("LOCALAPPDATA", home.clone().into()),
                (STATE_DIR_ENV, value),
            ]))
            .unwrap()
        };
        assert_eq!(with(state.clone().into()), state.join(FILE));
        // Relative to the working directory, and never the user's own.
        assert_eq!(
            with("state".into()),
            std::env::current_dir().unwrap().join("state").join(FILE)
        );
        // Set but empty counts as unset.
        assert!(with(OsString::new()).starts_with(&home));
    }
}
