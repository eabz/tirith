//! JSON persistence under `.tirith/` in the target repository.
//!
//! Runtime state (claims, tasks, sequence, daemon address) goes in
//! `.tirith/runtime/`, which is gitignored by a `.gitignore` this module
//! writes. Contracts, notices, and decisions are written as readable JSON
//! and JSON Lines so they can be committed and diffed. Every rewrite is
//! atomic: write a temp file, then rename. Logs are appended to. See
//! ADR-0003 and ADR-0010.
//!
//! Writes are incremental: [`JsonStore::apply`] takes a [`Delta`] and
//! touches only the files it names. The [`Persister`] is a background task
//! that drains deltas from [`State`] in sequence order, so a burst of
//! concurrent tool calls collapses into one write per file.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{Notify, watch};

use crate::contracts::Contract;
use crate::memory::{MemoryError, MemoryNote};
use crate::state::{Delta, Log, Snapshot, State};
use crate::types::MemoryId;

/// How long the [`Persister`] waits between checks for lease renewals
/// that nobody asked to flush.
pub const PERSIST_TICK: Duration = Duration::from_secs(1);

const DIR_NAME: &str = ".tirith";
const RUNTIME_DIR: &str = "runtime";
const CONTRACTS_DIR: &str = "contracts";
const MEMORY_DIR: &str = "memory";
const CLAIMS_FILE: &str = "runtime/claims.json";
const TASKS_FILE: &str = "runtime/tasks.json";
const META_FILE: &str = "runtime/meta.json";
const DAEMON_FILE: &str = "runtime/daemon.json";
const NOTICES_FILE: &str = "notices.jsonl";
/// Who acknowledged which notice: runtime only, so acking never rewrites
/// the committed notice log (ADR-0021).
const NOTICE_SEEN_FILE: &str = "runtime/notice_seen.jsonl";
/// Agent-to-agent messages: runtime only, never committed (ADR-0020).
const MESSAGES_FILE: &str = "runtime/messages.jsonl";
const DECISIONS_FILE: &str = "decisions.jsonl";
const GITIGNORE: &str = "# Written by tirith. Runtime state is local; everything else is meant to be committed.\nruntime/\n";

/// Why a store operation failed.
#[derive(Debug, Error)]
pub enum StoreError {
    /// A filesystem operation failed.
    #[error("{action} {path}: {source}")]
    Io {
        /// What was being attempted.
        action: &'static str,
        /// The file or directory involved.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: io::Error,
    },
    /// A file did not contain what was expected. Never a panic.
    #[error("parse {path}{}: {source}", .line.map(|l| format!(" line {l}")).unwrap_or_default())]
    Parse {
        /// The file involved.
        path: PathBuf,
        /// The line, for JSON Lines files.
        line: Option<usize>,
        /// The underlying error.
        #[source]
        source: serde_json::Error,
    },
    /// A memory note file could not be parsed. Never a panic.
    #[error("parse {path}: {source}")]
    Memory {
        /// The note file involved.
        path: PathBuf,
        /// What was wrong with it.
        #[source]
        source: MemoryError,
    },
    /// The blocking write task was cancelled or panicked.
    #[error("persistence task failed: {0}")]
    Join(String),
    /// The background persister reported a failed write, or has stopped.
    #[error("persist: {0}")]
    Persist(String),
}

/// A file or line that [`JsonStore::load`] could not read and skipped.
///
/// Loading never refuses to start over one bad line or file: the rest
/// of the store is loaded, the problem is logged, and it is reported
/// through `status`, `/api/health`, and the dashboard so a human can fix
/// the file. See ADR-0010 and the `Load errors` contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadError {
    /// The file, relative to `.tirith/`, with forward slashes.
    pub path: String,
    /// The 1-based line for JSON Lines files; `None` for whole files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// What went wrong.
    pub error: String,
}

impl LoadError {
    fn new(dir: &Path, path: &Path, line: Option<usize>, error: &dyn fmt::Display) -> Self {
        let rel = path
            .strip_prefix(dir)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let error = error.to_string();
        tracing::warn!(path = %rel, ?line, %error, "skipped while loading");
        Self {
            path: rel,
            line,
            error,
        }
    }
}

/// Where a running daemon can be found. Written on start, removed on
/// clean shutdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    /// The MCP endpoint, for example `http://127.0.0.1:7477/mcp`.
    pub url: String,
    /// The dashboard, for example `http://127.0.0.1:7477/`.
    pub dashboard_url: String,
    /// Process id of the daemon.
    pub pid: u32,
    /// When it started.
    pub started_at: DateTime<Utc>,
    /// Tirith version.
    pub version: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Meta {
    seq: u64,
}

/// Reads and writes the `.tirith/` directory.
///
/// Lines that failed to parse at load time are remembered per log file
/// so a later rewrite of that log puts them back at their original
/// position instead of silently dropping them.
#[derive(Debug, Clone)]
pub struct JsonStore {
    dir: PathBuf,
    damaged: Arc<Mutex<DamagedLines>>,
}

/// Per log file, the lines that failed to parse at load time as
/// `(1-based line, text)`.
type DamagedLines = HashMap<PathBuf, Vec<(usize, String)>>;

impl JsonStore {
    /// A store for the repository at `repo_root`.
    pub fn new(repo_root: impl AsRef<Path>) -> Self {
        Self {
            dir: repo_root.as_ref().join(DIR_NAME),
            damaged: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn damaged_lines(&self, path: &Path) -> Vec<(usize, String)> {
        self.damaged
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(path)
            .cloned()
            .unwrap_or_default()
    }

    /// The `.tirith/` directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Creates the directory layout and the `.gitignore` if missing.
    pub fn init(&self) -> Result<(), StoreError> {
        for sub in [RUNTIME_DIR, CONTRACTS_DIR, MEMORY_DIR] {
            let path = self.dir.join(sub);
            fs::create_dir_all(&path).map_err(|source| StoreError::Io {
                action: "create",
                path,
                source,
            })?;
        }
        let gitignore = self.dir.join(".gitignore");
        if !gitignore.exists() {
            write_atomic_now(&gitignore, GITIGNORE.as_bytes())?;
        }
        Ok(())
    }

    /// Loads everything. Missing files are empty. A file or line that
    /// cannot be parsed is skipped and reported in
    /// [`Snapshot::load_errors`]; only an I/O failure is an error.
    pub fn load(&self) -> Result<Snapshot, StoreError> {
        let mut errors = Vec::new();
        let meta: Meta = self.read_or_default(META_FILE, &mut errors)?;
        Ok(Snapshot {
            seq: meta.seq,
            claims: self.read_or_default(CLAIMS_FILE, &mut errors)?,
            tasks: self.read_or_default(TASKS_FILE, &mut errors)?,
            contracts: self.load_contracts(&mut errors)?,
            notices: self.read_log(NOTICES_FILE, &mut errors)?,
            notice_seen: self.read_log(NOTICE_SEEN_FILE, &mut errors)?,
            decisions: self.read_log(DECISIONS_FILE, &mut errors)?,
            memory: self.load_memory(&mut errors)?,
            messages: self.read_log(MESSAGES_FILE, &mut errors)?,
            load_errors: errors,
        })
    }

    /// Reads a whole-file JSON value, or the default when the file is
    /// missing or unparseable (the latter is reported).
    fn read_or_default<T: DeserializeOwned + Default>(
        &self,
        file: &str,
        errors: &mut Vec<LoadError>,
    ) -> Result<T, StoreError> {
        let path = self.dir.join(file);
        match read_json_or_default(&path) {
            Ok(value) => Ok(value),
            Err(StoreError::Parse { source, .. }) => {
                errors.push(LoadError::new(&self.dir, &path, None, &source));
                Ok(T::default())
            }
            Err(other) => Err(other),
        }
    }

    /// Reads a JSON Lines log, skipping and reporting bad lines, and
    /// remembers them so a rewrite keeps them in place.
    fn read_log<T: DeserializeOwned>(
        &self,
        file: &str,
        errors: &mut Vec<LoadError>,
    ) -> Result<Vec<T>, StoreError> {
        let path = self.dir.join(file);
        let (items, bad) = read_jsonl(&path)?;
        for (line, text, error) in bad {
            errors.push(LoadError::new(&self.dir, &path, Some(line), &error));
            self.damaged
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(path.clone())
                .or_default()
                .push((line, text));
        }
        Ok(items)
    }

    /// Reads every note under `.tirith/memory/`.
    ///
    /// The walk recurses because a permalink may carry folder segments.
    /// A missing directory is no notes. A file that does not parse, or
    /// whose permalink does not match its path, is skipped and reported
    /// in `errors`; it is never rewritten elsewhere.
    fn load_memory(&self, errors: &mut Vec<LoadError>) -> Result<Vec<MemoryNote>, StoreError> {
        let mut notes = Vec::new();
        let root = self.dir.join(MEMORY_DIR);
        let mut stack = vec![root.clone()];
        while let Some(current) = stack.pop() {
            let entries = match fs::read_dir(&current) {
                Ok(entries) => entries,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(source) => {
                    return Err(StoreError::Io {
                        action: "list",
                        path: current,
                        source,
                    });
                }
            };
            for entry in entries {
                let path = entry
                    .map_err(|source| StoreError::Io {
                        action: "list",
                        path: current.clone(),
                        source,
                    })?
                    .path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "md") {
                    let text = fs::read_to_string(&path).map_err(|source| StoreError::Io {
                        action: "read",
                        path: path.clone(),
                        source,
                    })?;
                    let note = match MemoryNote::from_markdown(&text) {
                        Ok(note) => note,
                        Err(error) => {
                            errors.push(LoadError::new(&self.dir, &path, error.line(), &error));
                            continue;
                        }
                    };
                    let expected = root.join(note.permalink.file_path());
                    if expected != path {
                        errors.push(LoadError::new(
                            &self.dir,
                            &path,
                            None,
                            &format!(
                                "permalink {} does not match the file path (expected {})",
                                note.permalink.as_str(),
                                expected
                                    .strip_prefix(&self.dir)
                                    .unwrap_or(&expected)
                                    .display()
                            ),
                        ));
                        continue;
                    }
                    notes.push(note);
                }
            }
        }
        Ok(notes)
    }

    fn load_contracts(&self, errors: &mut Vec<LoadError>) -> Result<Vec<Contract>, StoreError> {
        let dir = self.dir.join(CONTRACTS_DIR);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(StoreError::Io {
                    action: "list",
                    path: dir,
                    source,
                });
            }
        };
        let mut contracts = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|source| StoreError::Io {
                    action: "list",
                    path: dir.clone(),
                    source,
                })?
                .path();
            if path.extension().is_some_and(|e| e == "json") {
                match read_json::<Contract>(&path) {
                    Ok(contract) => contracts.push(contract),
                    Err(StoreError::Parse { source, .. }) => {
                        errors.push(LoadError::new(&self.dir, &path, None, &source));
                    }
                    Err(other) => return Err(other),
                }
            }
        }
        contracts.sort_by_key(|c| {
            c.history
                .first()
                .map_or(c.current.published_at, |v| v.published_at)
        });
        Ok(contracts)
    }

    /// Rewrites every file from `snapshot`.
    pub fn write_snapshot(&self, snapshot: &Snapshot) -> Result<(), StoreError> {
        self.apply(&Delta::full(snapshot))
    }

    /// Writes only what `delta` carries: whole-file rewrites for claims,
    /// tasks, and each changed contract; appends or rewrites for the logs.
    pub fn apply(&self, delta: &Delta) -> Result<(), StoreError> {
        self.init()?;
        let mut dirs = Dirs::new();
        if let Some(claims) = &delta.claims {
            write_json(&self.dir.join(CLAIMS_FILE), claims, &mut dirs)?;
        }
        if let Some(tasks) = &delta.tasks {
            write_json(&self.dir.join(TASKS_FILE), tasks, &mut dirs)?;
        }
        for contract in &delta.contracts {
            let file = format!("{}-{}.json", slug(&contract.name), contract.id.short());
            write_json_with(
                &self.dir.join(CONTRACTS_DIR).join(file),
                contract,
                &mut dirs,
                Durability::Lazy,
            )?;
        }
        for note in &delta.memory {
            let file = self.dir.join(MEMORY_DIR).join(note.permalink.file_path());
            // A permalink may carry folder segments, so the parent may not
            // exist yet. It can never escape MEMORY_DIR: see Permalink::parse.
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                    action: "create",
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            write_atomic_with(
                &file,
                note.to_markdown().as_bytes(),
                &mut dirs,
                Durability::Lazy,
            )?;
        }
        for id in &delta.memory_removed {
            self.remove_note_file(*id, &mut dirs)?;
        }
        self.write_log(NOTICES_FILE, &delta.notices, &mut dirs)?;
        self.write_log(NOTICE_SEEN_FILE, &delta.notice_seen, &mut dirs)?;
        self.write_log(DECISIONS_FILE, &delta.decisions, &mut dirs)?;
        self.write_log(MESSAGES_FILE, &delta.messages, &mut dirs)?;
        sync_dirs(&dirs)?;
        // The sequence number goes last, after everything above is durable,
        // so a crash mid-apply never records progress that did not happen.
        let mut meta_dir = Dirs::new();
        write_json(
            &self.dir.join(META_FILE),
            &Meta { seq: delta.seq },
            &mut meta_dir,
        )?;
        sync_dirs(&meta_dir)
    }

    /// Appends or rewrites a JSON Lines log. A rewrite puts back any lines
    /// that failed to parse at load time, at their original positions.
    fn write_log<T: Serialize>(
        &self,
        file: &str,
        log: &Log<T>,
        dirs: &mut Dirs,
    ) -> Result<(), StoreError> {
        let path = self.dir.join(file);
        match log {
            Log::Unchanged => Ok(()),
            Log::Appended(values) => append_jsonl(&path, values, dirs),
            Log::Rewritten(values) => write_jsonl(&path, values, &self.damaged_lines(&path), dirs),
        }
    }

    /// Deletes the file of a note that was removed from memory. The note
    /// is gone from the state, so its permalink is unknown here; the file
    /// is found by id under `memory/`. A missing file is fine.
    fn remove_note_file(&self, id: MemoryId, dirs: &mut Dirs) -> Result<(), StoreError> {
        let root = self.dir.join(MEMORY_DIR);
        let mut stack = vec![root];
        while let Some(current) = stack.pop() {
            let entries = match fs::read_dir(&current) {
                Ok(entries) => entries,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(source) => {
                    return Err(StoreError::Io {
                        action: "list",
                        path: current,
                        source,
                    });
                }
            };
            for entry in entries {
                let path = entry
                    .map_err(|source| StoreError::Io {
                        action: "list",
                        path: current.clone(),
                        source,
                    })?
                    .path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "md")
                    && fs::read_to_string(&path)
                        .ok()
                        .and_then(|text| MemoryNote::from_markdown(&text).ok())
                        .is_some_and(|note| note.id == id)
                {
                    fs::remove_file(&path).map_err(io_err("remove", &path))?;
                    dirs.insert(current);
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Records where the daemon is listening.
    pub fn write_daemon_info(&self, info: &DaemonInfo) -> Result<(), StoreError> {
        self.init()?;
        let mut dirs = Dirs::new();
        write_json(&self.dir.join(DAEMON_FILE), info, &mut dirs)?;
        sync_dirs(&dirs)
    }

    /// The daemon address recorded by the last `serve`, if any.
    pub fn read_daemon_info(&self) -> Result<Option<DaemonInfo>, StoreError> {
        let path = self.dir.join(DAEMON_FILE);
        if !path.exists() {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    /// Removes the daemon address on clean shutdown.
    pub fn clear_daemon_info(&self) -> Result<(), StoreError> {
        let path = self.dir.join(DAEMON_FILE);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StoreError::Io {
                action: "remove",
                path,
                source,
            }),
        }
    }
}

/// Where the background writer has got to.
#[derive(Debug, Clone, Default)]
struct Progress {
    /// Last sequence number attempted.
    seq: u64,
    /// Why that attempt failed, if it did.
    error: Option<String>,
    /// The first sequence number the current failure covers. Deltas
    /// below it reached disk, so their callers are not handed this error.
    failed_since: u64,
}

/// A background task that writes [`Delta`]s from [`State`] to a
/// [`JsonStore`], one at a time and in order.
///
/// Tool handlers call [`flush`](Self::flush) after a mutation; it wakes
/// the task and waits until everything dirty at that moment is on disk.
/// Many concurrent callers wait on the same write. Lease renewals never
/// wake the task; a periodic tick ([`PERSIST_TICK`]) picks them up. After
/// a failed write the next delta rewrites every file.
#[derive(Debug)]
pub struct Persister {
    store: Arc<JsonStore>,
    state: Arc<State>,
    wake: Notify,
    progress: watch::Sender<Progress>,
    attempted: watch::Sender<u64>,
    stopped: AtomicBool,
}

impl Persister {
    /// Starts the background task for `store` and `state`. The state's
    /// current sequence number counts as already written.
    pub fn spawn(store: Arc<JsonStore>, state: Arc<State>) -> Arc<Self> {
        let progress = watch::Sender::new(Progress {
            seq: state.seq(),
            error: None,
            failed_since: 0,
        });
        let attempted = watch::Sender::new(state.seq());
        let this = Arc::new(Self {
            store,
            state,
            wake: Notify::new(),
            progress,
            attempted,
            stopped: AtomicBool::new(false),
        });
        let worker = Arc::clone(&this);
        tokio::spawn(async move { worker.run().await });
        this
    }

    async fn run(&self) {
        while !self.stopped.load(Ordering::Acquire) {
            tokio::select! {
                () = self.wake.notified() => {}
                () = tokio::time::sleep(PERSIST_TICK) => {}
            }
            self.drain().await;
        }
    }

    /// Writes every pending delta. Stops at the first failure so a broken
    /// disk is retried on the next wake or tick, not in a tight loop.
    async fn drain(&self) {
        while let Some(delta) = self.state.take_dirty() {
            let seq = delta.seq;
            let store = Arc::clone(&self.store);
            let result = tokio::task::spawn_blocking(move || store.apply(&delta))
                .await
                .map_err(|e| StoreError::Join(e.to_string()))
                .and_then(|r| r);
            let error = result.err().map(|e| e.to_string());
            if let Some(error) = &error {
                tracing::error!(%error, seq, "failed to persist state");
                self.state.mark_all_dirty();
            }
            let failed = error.is_some();
            self.progress.send_replace(Progress {
                seq,
                error,
                failed_since: if failed { seq } else { 0 },
            });
            self.attempted.send_replace(seq);
            if failed {
                break;
            }
        }
    }

    /// A receiver that changes after every write attempt, carrying the
    /// sequence number attempted. The dashboard rebuilds its cached view
    /// on each change, so it never reads state per request.
    pub fn attempts(&self) -> watch::Receiver<u64> {
        self.attempted.subscribe()
    }

    /// Wakes the task without waiting.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Waits until every mutation made so far is on disk.
    pub async fn flush(&self) -> Result<(), StoreError> {
        self.wait_for(self.state.persist_target(false)).await
    }

    /// Like [`flush`](Self::flush) but also writes pending lease
    /// renewals. Called at shutdown.
    pub async fn sync(&self) -> Result<(), StoreError> {
        self.wait_for(self.state.persist_target(true)).await
    }

    async fn wait_for(&self, target: u64) -> Result<(), StoreError> {
        self.wake.notify_one();
        let mut rx = self.progress.subscribe();
        loop {
            let progress = rx.borrow_and_update().clone();
            if progress.seq >= target {
                return match progress.error {
                    Some(e) if target >= progress.failed_since => Err(StoreError::Persist(e)),
                    _ => Ok(()),
                };
            }
            if rx.changed().await.is_err() {
                return Err(StoreError::Persist("persister stopped".to_owned()));
            }
        }
    }

    /// Stops the background task after its current write.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.wake.notify_one();
    }

    /// The most recent write failure, if the last write failed.
    pub fn last_error(&self) -> Option<String> {
        self.progress.borrow().error.clone()
    }
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    for ch in name.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_end_matches('-');
    if trimmed.is_empty() {
        "contract".to_owned()
    } else {
        trimmed.chars().take(48).collect()
    }
}

/// Directories whose entries changed during one `apply`. Each is fsynced
/// once at the end, not once per file: on macOS every sync is a full
/// device flush, and a batch of two hundred notes must not pay for four
/// hundred of them.
type Dirs = BTreeSet<PathBuf>;

fn io_err(action: &'static str, path: &Path) -> impl FnOnce(io::Error) -> StoreError {
    let path = path.to_path_buf();
    move |source| StoreError::Io {
        action,
        path,
        source,
    }
}

/// Writes `bytes` to `path` through a temp file: write, fsync, rename. The
/// parent directory is recorded in `dirs` for [`sync_dirs`], which makes
/// the rename itself durable. A crash at any point leaves either the old
/// file or the complete new one.
fn write_atomic(path: &Path, bytes: &[u8], dirs: &mut Dirs) -> Result<(), StoreError> {
    write_atomic_with(path, bytes, dirs, Durability::Synced)
}

/// How hard a file write tries to survive a power loss.
#[derive(Debug, Clone, Copy)]
enum Durability {
    /// fsync the file before the rename: the file is complete or absent
    /// after a crash. For runtime state (`claims.json`, `tasks.json`,
    /// `meta.json`), which nothing else holds a copy of.
    Synced,
    /// Write and rename only; the directory sync at the end of the apply
    /// still makes the rename durable, but a power loss in between can
    /// leave the file empty or a log's last line torn. For every committed
    /// file (contracts, memory notes, the notices and decisions logs):
    /// git holds the content, and load reports a torn file instead of
    /// failing, so one full device flush per file (which is what
    /// `sync_all` is on macOS) is not worth paying N times per batch.
    /// Decision "Durability tiers: fsync runtime state, not committed
    /// files", 2026-09-16.
    Lazy,
}

fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    dirs: &mut Dirs,
    durability: Durability,
) -> Result<(), StoreError> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut file = fs::File::create(&tmp).map_err(io_err("create", &tmp))?;
    file.write_all(bytes).map_err(io_err("write", &tmp))?;
    if matches!(durability, Durability::Synced) {
        file.sync_all().map_err(io_err("sync", &tmp))?;
    }
    drop(file);
    fs::rename(&tmp, path).map_err(io_err("rename into", path))?;
    if let Some(parent) = path.parent() {
        dirs.insert(parent.to_path_buf());
    }
    Ok(())
}

/// [`write_atomic`] followed immediately by the directory sync, for
/// one-off writes outside `apply`.
fn write_atomic_now(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let mut dirs = Dirs::new();
    write_atomic(path, bytes, &mut dirs)?;
    sync_dirs(&dirs)
}

/// Fsyncs each directory once so renames and unlinks in it are durable.
/// A no-op on platforms where directories cannot be opened.
fn sync_dirs(dirs: &Dirs) -> Result<(), StoreError> {
    #[cfg(unix)]
    for dir in dirs {
        fs::File::open(dir)
            .and_then(|handle| handle.sync_all())
            .map_err(io_err("sync", dir))?;
    }
    #[cfg(not(unix))]
    let _ = dirs;
    Ok(())
}

fn write_json<T: Serialize>(path: &Path, value: &T, dirs: &mut Dirs) -> Result<(), StoreError> {
    write_json_with(path, value, dirs, Durability::Synced)
}

fn write_json_with<T: Serialize>(
    path: &Path,
    value: &T,
    dirs: &mut Dirs,
    durability: Durability,
) -> Result<(), StoreError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|source| StoreError::Parse {
        path: path.to_path_buf(),
        line: None,
        source,
    })?;
    bytes.push(b'\n');
    write_atomic_with(path, &bytes, dirs, durability)
}

/// Rewrites a JSON Lines file. `keep` holds lines that failed to parse at
/// load time as `(1-based line, text)`; each is put back at its line so
/// nothing a human wrote is lost.
fn write_jsonl<T: Serialize>(
    path: &Path,
    values: &[T],
    keep: &[(usize, String)],
    dirs: &mut Dirs,
) -> Result<(), StoreError> {
    // The logs are committed files, so they take the lazy path like
    // contracts and notes: see `Durability`.
    let mut lines: Vec<Vec<u8>> = Vec::with_capacity(values.len() + keep.len());
    for value in values {
        let mut line = Vec::new();
        serde_json::to_writer(&mut line, value).map_err(|source| StoreError::Parse {
            path: path.to_path_buf(),
            line: None,
            source,
        })?;
        lines.push(line);
    }
    for (number, text) in keep {
        let index = number.saturating_sub(1).min(lines.len());
        lines.insert(index, text.as_bytes().to_vec());
    }
    let mut bytes = Vec::new();
    for line in lines {
        bytes.extend_from_slice(&line);
        bytes.push(b'\n');
    }
    write_atomic_with(path, &bytes, dirs, Durability::Lazy)
}

/// Appends `values` as lines. If the file was hand-edited and lost its
/// trailing newline, one is added first so lines never run together.
fn append_jsonl<T: Serialize>(
    path: &Path,
    values: &[T],
    dirs: &mut Dirs,
) -> Result<(), StoreError> {
    let io_err = |action: &'static str| io_err(action, path);
    let mut file = fs::OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(path)
        .map_err(io_err("open"))?;
    let len = file.metadata().map_err(io_err("stat"))?.len();
    let mut bytes = Vec::new();
    if len > 0 {
        file.seek(SeekFrom::End(-1)).map_err(io_err("seek"))?;
        let mut last = [0u8; 1];
        file.read_exact(&mut last).map_err(io_err("read"))?;
        if last[0] != b'\n' {
            bytes.push(b'\n');
        }
    }
    for value in values {
        serde_json::to_writer(&mut bytes, value).map_err(|source| StoreError::Parse {
            path: path.to_path_buf(),
            line: None,
            source,
        })?;
        bytes.push(b'\n');
    }
    file.write_all(&bytes).map_err(io_err("append"))?;
    // No per-call fsync: the log is a committed file (see `Durability`),
    // and a torn last line is reported at load rather than fatal.
    if let Some(parent) = path.parent() {
        dirs.insert(parent.to_path_buf());
    }
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, StoreError> {
    let bytes = fs::read(path).map_err(|source| StoreError::Io {
        action: "read",
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| StoreError::Parse {
        path: path.to_path_buf(),
        line: None,
        source,
    })
}

fn read_json_or_default<T: DeserializeOwned + Default>(path: &Path) -> Result<T, StoreError> {
    if path.exists() {
        read_json(path)
    } else {
        Ok(T::default())
    }
}

/// Reads a JSON Lines file into the values that parsed and the lines that
/// did not, as `(1-based line, text, error)`.
#[allow(clippy::type_complexity)] // one private return type; a struct would only rename it
fn read_jsonl<T: DeserializeOwned>(
    path: &Path,
) -> Result<(Vec<T>, Vec<(usize, String, serde_json::Error)>), StoreError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), Vec::new())),
        Err(source) => {
            return Err(StoreError::Io {
                action: "read",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let mut out = Vec::new();
    let mut bad = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(value) => out.push(value),
            Err(error) => bad.push((index + 1, line.to_owned(), error)),
        }
    }
    Ok((out, bad))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;
    use crate::claims::ClaimBook;
    use crate::contracts::{ContractKind, ContractRegistry, NewContract};
    use crate::types::{AgentId, RepoPath};

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 15, 18, 0, 0).unwrap()
    }

    fn sample() -> Snapshot {
        let agent = AgentId::new("alice").unwrap();
        let mut claims = ClaimBook::default();
        claims
            .claim(
                agent.clone(),
                vec![RepoPath::new("src/auth").unwrap()],
                "r".into(),
                600,
                t0(),
            )
            .unwrap();
        let mut contracts = ContractRegistry::default();
        contracts
            .publish(
                agent,
                NewContract {
                    name: "POST /api/sessions".into(),
                    kind: ContractKind::Http,
                    shape: json!({"request": {}}),
                    consumers: Some(vec![]),
                    notes: String::new(),
                    expected_version: None,
                },
                t0(),
            )
            .unwrap();
        let mut memory = crate::memory::MemoryBook::default();
        memory
            .write(
                AgentId::new("alice").unwrap(),
                crate::memory::NewMemory {
                    title: "Store layout".to_owned(),
                    body: "One file per note, committed.".to_owned(),
                    paths: vec![RepoPath::new("src/store.rs").unwrap()],
                    ..crate::memory::NewMemory::default()
                },
                t0(),
            )
            .unwrap();
        Snapshot {
            messages: Vec::new(),
            notice_seen: Vec::new(),
            seq: 7,
            claims: claims.claims().to_vec(),
            tasks: vec![],
            contracts: contracts.contracts().to_vec(),
            notices: vec![],
            decisions: vec![],
            memory: memory.notes().to_vec(),
            load_errors: Vec::new(),
        }
    }

    #[test]
    fn round_trip_and_layout() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::new(dir.path());
        assert_eq!(
            store.load().unwrap(),
            Snapshot::default(),
            "empty store loads empty"
        );
        let snapshot = sample();
        store.write_snapshot(&snapshot).unwrap();
        assert_eq!(store.load().unwrap(), snapshot);
        let root = dir.path().join(".tirith");
        assert!(root.join(".gitignore").exists());
        assert!(root.join("runtime/claims.json").exists());
        let contracts: Vec<_> = fs::read_dir(root.join("contracts")).unwrap().collect();
        assert_eq!(contracts.len(), 1);
        assert!(
            contracts[0]
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("post-api-sessions-")
        );
        assert!(
            !fs::read_dir(root.join("runtime"))
                .unwrap()
                .any(|e| { e.unwrap().file_name().to_string_lossy().ends_with(".tmp") })
        );
    }

    #[test]
    fn bad_lines_and_files_are_reported_not_fatal_and_survive_a_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::new(dir.path());
        store.write_snapshot(&sample()).unwrap();
        let notices = store.dir().join("notices.jsonl");
        let good = serde_json::to_string(
            &crate::notices::NoticeBoard::default()
                .publish(
                    AgentId::new("a").unwrap(),
                    crate::notices::NewNotice {
                        kind: crate::notices::NoticeKind::Rename,
                        summary: "fine".into(),
                        from: None,
                        to: None,
                        affected_paths: vec![],
                        contract_id: None,
                    },
                    t0(),
                )
                .unwrap(),
        )
        .unwrap();
        fs::write(&notices, format!("<<<<<<< HEAD\n{good}\n{{}}\n")).unwrap();
        fs::write(store.dir().join("contracts/broken-00000000.json"), "{ nope").unwrap();
        fs::write(store.dir().join("runtime/tasks.json"), "[[[").unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.notices.len(), 1, "the good line loads");
        assert_eq!(loaded.contracts.len(), 1, "the good contract loads");
        assert!(
            loaded.tasks.is_empty(),
            "a corrupt runtime file reads as empty"
        );
        let mut reported: Vec<(String, Option<usize>)> = loaded
            .load_errors
            .iter()
            .map(|e| (e.path.clone(), e.line))
            .collect();
        reported.sort();
        assert_eq!(
            reported,
            vec![
                ("contracts/broken-00000000.json".to_owned(), None),
                ("notices.jsonl".to_owned(), Some(1)),
                ("notices.jsonl".to_owned(), Some(3)),
                ("runtime/tasks.json".to_owned(), None),
            ]
        );

        // A rewrite of the damaged log keeps the bad lines where they were.
        store
            .apply(&Delta {
                messages: Log::Unchanged,
                notice_seen: Log::Unchanged,
                seq: 9,
                notices: Log::Rewritten(loaded.notices.clone()),
                ..Delta::default()
            })
            .unwrap();
        let lines: Vec<String> = fs::read_to_string(&notices)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        assert_eq!(lines[0], "<<<<<<< HEAD");
        assert_eq!(lines[1], good);
        assert_eq!(lines[2], "{}");
        assert_eq!(store.load().unwrap().load_errors.len(), 4, "still reported");
    }

    #[test]
    fn meta_is_written_last_so_a_failed_apply_records_no_progress() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::new(dir.path());
        let snapshot = sample();
        store.write_snapshot(&snapshot).unwrap();
        assert_eq!(store.load().unwrap().seq, 7);
        // Make the contracts directory unwritable by replacing it with a file.
        fs::remove_dir_all(store.dir().join("contracts")).unwrap();
        fs::write(store.dir().join("contracts"), "in the way").unwrap();
        let err = store
            .apply(&Delta {
                messages: Log::Unchanged,
                notice_seen: Log::Unchanged,
                seq: 8,
                contracts: snapshot.contracts.clone(),
                ..Delta::default()
            })
            .unwrap_err();
        assert!(matches!(err, StoreError::Io { .. }), "{err}");
        fs::remove_file(store.dir().join("contracts")).unwrap();
        assert_eq!(store.load().unwrap().seq, 7, "seq did not advance");
    }

    #[test]
    fn daemon_info_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::new(dir.path());
        assert!(store.read_daemon_info().unwrap().is_none());
        let info = DaemonInfo {
            url: "http://127.0.0.1:1/mcp".into(),
            dashboard_url: "http://127.0.0.1:1/".into(),
            pid: 1,
            started_at: t0(),
            version: "0".into(),
        };
        store.write_daemon_info(&info).unwrap();
        assert_eq!(store.read_daemon_info().unwrap(), Some(info));
        store.clear_daemon_info().unwrap();
        store.clear_daemon_info().unwrap();
        assert!(store.read_daemon_info().unwrap().is_none());
    }

    #[test]
    fn deltas_append_rewrite_and_leave_other_files_alone() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::new(dir.path());
        let snapshot = sample();
        store.write_snapshot(&snapshot).unwrap();
        let notices_path = store.dir().join("notices.jsonl");
        assert_eq!(fs::read_to_string(&notices_path).unwrap(), "");

        let agent = AgentId::new("alice").unwrap();
        let mut board = crate::notices::NoticeBoard::default();
        for summary in ["one", "two", "three"] {
            board
                .publish(
                    agent.clone(),
                    crate::notices::NewNotice {
                        kind: crate::notices::NoticeKind::Rename,
                        summary: summary.into(),
                        from: None,
                        to: None,
                        affected_paths: vec![],
                        contract_id: None,
                    },
                    t0(),
                )
                .unwrap();
        }
        let notices = board.notices();
        let claims_before = fs::metadata(store.dir().join("runtime/claims.json"))
            .unwrap()
            .modified()
            .unwrap();
        store
            .apply(&Delta {
                messages: Log::Unchanged,
                notice_seen: Log::Unchanged,
                seq: 8,
                notices: Log::Appended(notices[..2].to_vec()),
                ..Delta::default()
            })
            .unwrap();
        // A hand edit that drops the trailing newline must not merge lines.
        let text = fs::read_to_string(&notices_path).unwrap();
        fs::write(&notices_path, text.trim_end()).unwrap();
        store
            .apply(&Delta {
                messages: Log::Unchanged,
                notice_seen: Log::Unchanged,
                seq: 9,
                notices: Log::Appended(notices[2..].to_vec()),
                ..Delta::default()
            })
            .unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.seq, 9);
        assert_eq!(loaded.notices, notices);
        assert_eq!(loaded.claims, snapshot.claims, "claims untouched");
        assert_eq!(
            fs::metadata(store.dir().join("runtime/claims.json"))
                .unwrap()
                .modified()
                .unwrap(),
            claims_before
        );

        store
            .apply(&Delta {
                messages: Log::Unchanged,
                notice_seen: Log::Unchanged,
                seq: 10,
                notices: Log::Rewritten(notices[..1].to_vec()),
                ..Delta::default()
            })
            .unwrap();
        assert_eq!(store.load().unwrap().notices, notices[..1]);
    }

    #[tokio::test]
    async fn persister_coalesces_and_flushes() {
        use crate::clock::ManualClock;
        use crate::types::RepoPath;

        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(JsonStore::new(dir.path()));
        let clock = Arc::new(ManualClock::new(t0()));
        let state = Arc::new(State::new(clock, Snapshot::default()));
        let persister = Persister::spawn(Arc::clone(&store), Arc::clone(&state));
        persister.flush().await.unwrap();
        assert_eq!(store.load().unwrap().seq, 0, "nothing to write");

        for i in 0..20 {
            state
                .claim(
                    AgentId::new(format!("a{i}")).unwrap(),
                    vec![RepoPath::new(&format!("p{i}")).unwrap()],
                    "r".into(),
                    None,
                )
                .unwrap();
        }
        persister.flush().await.unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.claims.len(), 20);
        assert!(loaded.seq <= 2, "one burst, one or two writes, not 20");
        assert!(persister.last_error().is_none());

        // A renewal is picked up by sync() even though flush() ignores it.
        state.claims(Some(&AgentId::new("a0").unwrap()), None);
        let before = store.load().unwrap().seq;
        persister.flush().await.unwrap();
        assert_eq!(store.load().unwrap().seq, before);
        persister.sync().await.unwrap();
        assert_eq!(store.load().unwrap().seq, before + 1);
        persister.stop();
    }

    #[tokio::test]
    async fn persister_over_loaded_state_does_not_wait_for_old_seq() {
        use crate::clock::ManualClock;

        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(JsonStore::new(dir.path()));
        let mut snapshot = sample();
        snapshot.seq = 42;
        store.write_snapshot(&snapshot).unwrap();
        let clock = Arc::new(ManualClock::new(t0()));
        let state = Arc::new(State::new(clock, store.load().unwrap()));
        let persister = Persister::spawn(Arc::clone(&store), Arc::clone(&state));
        tokio::time::timeout(Duration::from_secs(5), persister.sync())
            .await
            .expect("sync must not wait for a write that never comes")
            .unwrap();
        assert_eq!(store.load().unwrap().seq, 42);
        persister.stop();
    }

    #[tokio::test]
    async fn persister_reports_failures_and_recovers() {
        use crate::clock::ManualClock;
        use crate::types::RepoPath;

        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(JsonStore::new(dir.path()));
        let clock = Arc::new(ManualClock::new(t0()));
        let state = Arc::new(State::new(clock, Snapshot::default()));
        let persister = Persister::spawn(Arc::clone(&store), Arc::clone(&state));
        // Make the runtime directory a file so writes into it fail.
        fs::create_dir_all(store.dir()).unwrap();
        fs::write(store.dir().join("runtime"), "not a dir").unwrap();
        state
            .claim(
                AgentId::new("alice").unwrap(),
                vec![RepoPath::new("a").unwrap()],
                "r".into(),
                None,
            )
            .unwrap();
        let err = persister.flush().await.unwrap_err();
        assert!(matches!(err, StoreError::Persist(_)), "{err}");
        assert!(persister.last_error().is_some());
        assert!(
            persister.wait_for(0).await.is_ok(),
            "a caller whose delta preceded the failure is not handed it"
        );

        fs::remove_file(store.dir().join("runtime")).unwrap();
        persister.flush().await.unwrap();
        assert!(persister.last_error().is_none());
        assert_eq!(store.load().unwrap().claims.len(), 1);
        persister.stop();
    }

    #[test]
    fn slugs_are_filesystem_safe() {
        assert_eq!(slug("POST /api/sessions"), "post-api-sessions");
        assert_eq!(slug("State::claim()"), "state-claim");
        assert_eq!(slug("///"), "contract");
    }
}
