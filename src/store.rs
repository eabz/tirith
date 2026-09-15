//! JSON persistence under `.tirith/` in the target repository.
//!
//! Runtime state (claims, tasks, sequence, daemon address) goes in
//! `.tirith/runtime/`, which is gitignored by a `.gitignore` this module
//! writes. Contracts, notices, and decisions are written as readable JSON
//! and JSON Lines so they can be committed and diffed. Every write is
//! atomic: write a temp file, then rename. See ADR-0003.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::contracts::Contract;
use crate::decisions::Decision;
use crate::notices::Notice;
use crate::state::Snapshot;

const DIR_NAME: &str = ".tirith";
const RUNTIME_DIR: &str = "runtime";
const CONTRACTS_DIR: &str = "contracts";
const CLAIMS_FILE: &str = "runtime/claims.json";
const TASKS_FILE: &str = "runtime/tasks.json";
const META_FILE: &str = "runtime/meta.json";
const DAEMON_FILE: &str = "runtime/daemon.json";
const NOTICES_FILE: &str = "notices.jsonl";
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
    /// The blocking write task was cancelled or panicked.
    #[error("persistence task failed: {0}")]
    Join(String),
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
#[derive(Debug, Clone)]
pub struct JsonStore {
    dir: PathBuf,
}

impl JsonStore {
    /// A store for the repository at `repo_root`.
    pub fn new(repo_root: impl AsRef<Path>) -> Self {
        Self {
            dir: repo_root.as_ref().join(DIR_NAME),
        }
    }

    /// The `.tirith/` directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Creates the directory layout and the `.gitignore` if missing.
    pub fn init(&self) -> Result<(), StoreError> {
        for sub in [RUNTIME_DIR, CONTRACTS_DIR] {
            let path = self.dir.join(sub);
            fs::create_dir_all(&path).map_err(|source| StoreError::Io {
                action: "create",
                path,
                source,
            })?;
        }
        let gitignore = self.dir.join(".gitignore");
        if !gitignore.exists() {
            write_atomic(&gitignore, GITIGNORE.as_bytes())?;
        }
        Ok(())
    }

    /// Loads everything. Missing files are empty; corrupt files are errors.
    pub fn load(&self) -> Result<Snapshot, StoreError> {
        let meta: Meta = read_json_or_default(&self.dir.join(META_FILE))?;
        Ok(Snapshot {
            seq: meta.seq,
            claims: read_json_or_default(&self.dir.join(CLAIMS_FILE))?,
            tasks: read_json_or_default(&self.dir.join(TASKS_FILE))?,
            contracts: self.load_contracts()?,
            notices: read_jsonl(&self.dir.join(NOTICES_FILE))?,
            decisions: read_jsonl(&self.dir.join(DECISIONS_FILE))?,
        })
    }

    fn load_contracts(&self) -> Result<Vec<Contract>, StoreError> {
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
                contracts.push(read_json::<Contract>(&path)?);
            }
        }
        contracts.sort_by_key(|c| {
            c.history
                .first()
                .map_or(c.current.published_at, |v| v.published_at)
        });
        Ok(contracts)
    }

    /// Writes every part of `snapshot` atomically, file by file.
    pub fn write_snapshot(&self, snapshot: &Snapshot) -> Result<(), StoreError> {
        self.init()?;
        write_json(&self.dir.join(META_FILE), &Meta { seq: snapshot.seq })?;
        write_json(&self.dir.join(CLAIMS_FILE), &snapshot.claims)?;
        write_json(&self.dir.join(TASKS_FILE), &snapshot.tasks)?;
        for contract in &snapshot.contracts {
            let file = format!("{}-{}.json", slug(&contract.name), contract.id.short());
            write_json(&self.dir.join(CONTRACTS_DIR).join(file), contract)?;
        }
        write_jsonl::<Notice>(&self.dir.join(NOTICES_FILE), &snapshot.notices)?;
        write_jsonl::<Decision>(&self.dir.join(DECISIONS_FILE), &snapshot.decisions)?;
        Ok(())
    }

    /// Records where the daemon is listening.
    pub fn write_daemon_info(&self, info: &DaemonInfo) -> Result<(), StoreError> {
        self.init()?;
        write_json(&self.dir.join(DAEMON_FILE), info)
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

/// Serializes writes so snapshots land on disk in sequence order and a
/// slow write never overwrites a newer one.
#[derive(Debug)]
pub struct Persister {
    store: Arc<JsonStore>,
    last_seq: tokio::sync::Mutex<u64>,
    last_error: Mutex<Option<String>>,
}

impl Persister {
    /// A persister for `store`.
    pub fn new(store: Arc<JsonStore>) -> Self {
        Self {
            store,
            last_seq: tokio::sync::Mutex::new(0),
            last_error: Mutex::new(None),
        }
    }

    /// Writes `snapshot` unless a newer one was already written.
    pub async fn persist(&self, snapshot: Snapshot) -> Result<(), StoreError> {
        let mut last = self.last_seq.lock().await;
        if snapshot.seq <= *last {
            return Ok(());
        }
        let seq = snapshot.seq;
        let store = Arc::clone(&self.store);
        let result = tokio::task::spawn_blocking(move || store.write_snapshot(&snapshot))
            .await
            .map_err(|e| StoreError::Join(e.to_string()))
            .and_then(|r| r);
        let mut last_error = self
            .last_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match &result {
            Ok(()) => {
                *last = seq;
                *last_error = None;
            }
            Err(e) => *last_error = Some(e.to_string()),
        }
        result
    }

    /// The most recent write failure, if the last write failed.
    pub fn last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, bytes).map_err(|source| StoreError::Io {
        action: "write",
        path: tmp.clone(),
        source,
    })?;
    fs::rename(&tmp, path).map_err(|source| StoreError::Io {
        action: "rename into",
        path: path.to_path_buf(),
        source,
    })
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), StoreError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|source| StoreError::Parse {
        path: path.to_path_buf(),
        line: None,
        source,
    })?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

fn write_jsonl<T: Serialize>(path: &Path, values: &[T]) -> Result<(), StoreError> {
    let mut bytes = Vec::new();
    for value in values {
        serde_json::to_writer(&mut bytes, value).map_err(|source| StoreError::Parse {
            path: path.to_path_buf(),
            line: None,
            source,
        })?;
        bytes.push(b'\n');
    }
    write_atomic(path, &bytes)
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

fn read_jsonl<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, StoreError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(StoreError::Io {
                action: "read",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let mut out = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        out.push(
            serde_json::from_str(line).map_err(|source| StoreError::Parse {
                path: path.to_path_buf(),
                line: Some(index + 1),
                source,
            })?,
        );
    }
    Ok(out)
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
                    consumers: vec![],
                    notes: String::new(),
                },
                t0(),
            )
            .unwrap();
        Snapshot {
            seq: 7,
            claims: claims.claims().to_vec(),
            tasks: vec![],
            contracts: contracts.contracts().to_vec(),
            notices: vec![],
            decisions: vec![],
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
    fn corrupt_files_are_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::new(dir.path());
        store.init().unwrap();
        fs::write(store.dir().join("notices.jsonl"), "{}\nnot json\n").unwrap();
        let err = store.load().unwrap_err();
        assert!(
            matches!(err, StoreError::Parse { line: Some(1), .. }),
            "{err}"
        );
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

    #[tokio::test]
    async fn persister_skips_stale_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(JsonStore::new(dir.path()));
        let persister = Persister::new(store.clone());
        let mut newer = sample();
        newer.seq = 9;
        persister.persist(newer.clone()).await.unwrap();
        let older = Snapshot {
            seq: 8,
            ..Snapshot::default()
        };
        persister.persist(older).await.unwrap();
        assert_eq!(store.load().unwrap(), newer);
        assert!(persister.last_error().is_none());
    }

    #[test]
    fn slugs_are_filesystem_safe() {
        assert_eq!(slug("POST /api/sessions"), "post-api-sessions");
        assert_eq!(slug("State::claim()"), "state-claim");
        assert_eq!(slug("///"), "contract");
    }
}
