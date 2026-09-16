//! Contracts: interface shapes published before implementation.
//!
//! A contract is named, versioned, and carries a free-form JSON `shape`
//! plus the paths expected to consume it. Publishing under an existing
//! name creates a new version; the caller (see [`crate::state::State`])
//! turns that into a change notice for the consumers.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::types::{AgentId, ContractId, Page, PrefixError, RepoPath, resolve_prefix};

/// What kind of interface a contract describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractKind {
    /// An HTTP endpoint: method, path, request, response, errors.
    Http,
    /// A function or method signature.
    Function,
    /// A data type, struct, or schema.
    Type,
    /// An emitted event or message.
    Event,
    /// A command-line interface.
    Cli,
    /// Anything else.
    Other,
}

impl ContractKind {
    /// The wire name of the kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Function => "function",
            Self::Type => "type",
            Self::Event => "event",
            Self::Cli => "cli",
            Self::Other => "other",
        }
    }
}

impl fmt::Display for ContractKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ContractKind {
    type Err = ContractError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "http" => Ok(Self::Http),
            "function" | "fn" => Ok(Self::Function),
            "type" => Ok(Self::Type),
            "event" => Ok(Self::Event),
            "cli" => Ok(Self::Cli),
            "other" => Ok(Self::Other),
            other => Err(ContractError::UnknownKind(other.to_owned())),
        }
    }
}

/// One published version of a contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ContractVersion {
    /// Starts at 1 and increases by one per publish.
    pub version: u32,
    /// The interface shape. Free JSON in this release.
    pub shape: Value,
    /// Free-text notes about this version.
    pub notes: String,
    /// Who published it.
    pub published_by: AgentId,
    /// When it was published.
    pub published_at: DateTime<Utc>,
}

/// A named, versioned interface contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Contract {
    /// Stable across versions.
    pub id: ContractId,
    /// Unique name, for example `POST /api/sessions` or `State::claim`.
    pub name: String,
    /// What kind of interface this is.
    pub kind: ContractKind,
    /// Paths expected to depend on this contract.
    pub consumers: Vec<RepoPath>,
    /// The latest version.
    pub current: ContractVersion,
    /// Older versions, oldest first.
    pub history: Vec<ContractVersion>,
}

impl Contract {
    /// Whether `path` overlaps any consumer path.
    pub fn consumed_by(&self, path: &RepoPath) -> bool {
        self.consumers.iter().any(|c| c.overlaps(path))
    }
}

/// Input for publishing a contract. Build it with [`NewContract::new`]
/// and the `with_*` setters outside this module, so a new optional field
/// never breaks a caller (decision 19c6595c).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewContract {
    /// See [`Contract::name`].
    pub name: String,
    /// See [`Contract::kind`].
    pub kind: ContractKind,
    /// See [`ContractVersion::shape`].
    pub shape: Value,
    /// See [`Contract::consumers`]. `None` keeps the existing list when
    /// republishing; `Some(vec![])` clears it.
    pub consumers: Option<Vec<RepoPath>>,
    /// See [`ContractVersion::notes`].
    pub notes: String,
    /// The version the publisher believes is current. When set and the
    /// name exists at another version, publishing is refused, so two agents
    /// cannot silently overwrite each other's v2.
    pub expected_version: Option<u32>,
}

impl NewContract {
    /// A contract `name` of `kind` with `shape`, no consumers, no notes,
    /// and no version expectation.
    ///
    /// ```
    /// use serde_json::json;
    /// use tirith::contracts::{ContractKind, NewContract};
    /// use tirith::types::RepoPath;
    ///
    /// let contract = NewContract::new("POST /api/sessions", ContractKind::Http, json!({}))
    ///     .with_consumers(vec![RepoPath::new("src/client").unwrap()])
    ///     .with_expected_version(0);
    /// assert_eq!(contract.expected_version, Some(0));
    /// ```
    pub fn new(name: impl Into<String>, kind: ContractKind, shape: Value) -> Self {
        Self {
            name: name.into(),
            kind,
            shape,
            consumers: None,
            notes: String::new(),
            expected_version: None,
        }
    }

    /// Sets the consumer paths; an empty list clears them on a republish.
    #[must_use]
    pub fn with_consumers(mut self, consumers: Vec<RepoPath>) -> Self {
        self.consumers = Some(consumers);
        self
    }

    /// Sets the free-text notes for this version.
    #[must_use]
    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = notes.into();
        self
    }

    /// Refuses the publish unless the contract is at `version` (0 when it
    /// must not exist yet).
    #[must_use]
    pub fn with_expected_version(mut self, version: u32) -> Self {
        self.expected_version = Some(version);
        self
    }
}

/// The result of publishing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    /// The contract after publishing.
    pub contract: Contract,
    /// The version that was current before, if the name already existed.
    pub previous_version: Option<u32>,
    /// Every path that consumed the contract before or after this publish;
    /// the change notice goes to all of them.
    pub notify: Vec<RepoPath>,
}

/// Why a contract operation was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ContractError {
    /// The name was empty.
    #[error("contract name must not be empty")]
    EmptyName,
    /// No contract has this name or id.
    #[error("no contract named or identified by {0:?}")]
    NotFound(String),
    /// The kind string was not recognized.
    #[error("unknown contract kind {0:?}; expected http, function, type, event, cli, or other")]
    UnknownKind(String),
    /// `expected_version` did not match what is current.
    #[error("contract {name:?} is at v{current}, not the expected v{expected}")]
    VersionConflict {
        /// The contract name.
        name: String,
        /// The current version (0 when the name does not exist yet).
        current: u32,
        /// What the publisher expected.
        expected: u32,
        /// Who published the current version, if any.
        published_by: Option<AgentId>,
    },
}

/// All contracts.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractRegistry {
    contracts: Vec<Contract>,
}

impl ContractRegistry {
    /// Rebuilds a registry from persisted contracts.
    pub fn from_contracts(contracts: Vec<Contract>) -> Self {
        Self { contracts }
    }

    /// All contracts in first-publish order.
    pub fn contracts(&self) -> &[Contract] {
        &self.contracts
    }

    /// Publishes a new contract or a new version of an existing one.
    pub fn publish(
        &mut self,
        by: AgentId,
        new: NewContract,
        now: DateTime<Utc>,
    ) -> Result<Published, ContractError> {
        let name = new.name.trim().to_owned();
        if name.is_empty() {
            return Err(ContractError::EmptyName);
        }
        if let Some(existing) = self.contracts.iter_mut().find(|c| c.name == name) {
            if let Some(expected) = new.expected_version
                && expected != existing.current.version
            {
                return Err(ContractError::VersionConflict {
                    name,
                    current: existing.current.version,
                    expected,
                    published_by: Some(existing.current.published_by.clone()),
                });
            }
            let next_version = existing.current.version + 1;
            let previous = std::mem::replace(
                &mut existing.current,
                ContractVersion {
                    version: next_version,
                    shape: new.shape,
                    notes: new.notes,
                    published_by: by,
                    published_at: now,
                },
            );
            let previous_version = previous.version;
            existing.history.push(previous);
            existing.kind = new.kind;
            let mut notify = existing.consumers.clone();
            if let Some(consumers) = new.consumers {
                for path in &consumers {
                    if !notify.contains(path) {
                        notify.push(path.clone());
                    }
                }
                existing.consumers = consumers;
            }
            return Ok(Published {
                contract: existing.clone(),
                previous_version: Some(previous_version),
                notify,
            });
        }
        if let Some(expected) = new.expected_version.filter(|v| *v != 0) {
            return Err(ContractError::VersionConflict {
                name,
                current: 0,
                expected,
                published_by: None,
            });
        }
        let consumers = new.consumers.unwrap_or_default();
        let contract = Contract {
            id: ContractId::new(),
            name,
            kind: new.kind,
            consumers: consumers.clone(),
            current: ContractVersion {
                version: 1,
                shape: new.shape,
                notes: new.notes,
                published_by: by,
                published_at: now,
            },
            history: Vec::new(),
        };
        self.contracts.push(contract.clone());
        Ok(Published {
            contract,
            previous_version: None,
            notify: consumers,
        })
    }

    /// Finds a contract by exact name, by id, or by a unique id prefix.
    pub fn get(&self, name_or_id: &str) -> Option<&Contract> {
        let key = name_or_id.trim();
        self.contracts.iter().find(|c| c.name == key).or_else(|| {
            self.resolve_id(key)
                .ok()
                .and_then(|id| self.contracts.iter().find(|c| c.id == id))
        })
    }

    /// Resolves a full id or a unique prefix to a contract id.
    pub fn resolve_id(&self, raw: &str) -> Result<ContractId, PrefixError> {
        resolve_prefix("contract", self.contracts.iter().map(|c| c.id), raw)
    }

    /// Contracts matching the optional consumer-path and kind filters.
    pub fn list(&self, path: Option<&RepoPath>, kind: Option<ContractKind>) -> Vec<&Contract> {
        self.contracts
            .iter()
            .filter(|c| path.is_none_or(|p| c.consumed_by(p)))
            .filter(|c| kind.is_none_or(|k| c.kind == k))
            .collect()
    }

    /// Most recently published `limit` contracts matching the filters,
    /// whose current version was published before `before`. See [`Page`].
    pub fn list_page(
        &self,
        path: Option<&RepoPath>,
        kind: Option<ContractKind>,
        before: Option<&(DateTime<Utc>, ContractId)>,
        limit: usize,
    ) -> Page<&Contract> {
        Page::newest_first(
            self.list(path, kind),
            |c| (c.current.published_at, c.id),
            before,
            limit,
        )
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    fn agent(name: &str) -> AgentId {
        AgentId::new(name).unwrap()
    }

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 15, 18, 0, 0).unwrap()
    }

    fn new(name: &str, shape: Value) -> NewContract {
        NewContract::new(name, ContractKind::Http, shape)
            .with_consumers(vec![RepoPath::new("src/client").unwrap()])
    }

    fn path(p: &str) -> RepoPath {
        RepoPath::new(p).unwrap()
    }

    #[test]
    fn republishing_without_consumers_keeps_them_and_notifies_the_union() {
        let mut reg = ContractRegistry::default();
        reg.publish(agent("a"), new("Foo", json!({})), t0())
            .unwrap();
        let mut omitted = new("Foo", json!({"v": 2}));
        omitted.consumers = None;
        let kept = reg.publish(agent("a"), omitted, t0()).unwrap();
        assert_eq!(kept.contract.consumers, vec![path("src/client")]);
        assert_eq!(kept.notify, vec![path("src/client")]);
        let mut moved = new("Foo", json!({"v": 3}));
        moved.consumers = Some(vec![path("src/cli")]);
        let changed = reg.publish(agent("a"), moved, t0()).unwrap();
        assert_eq!(changed.contract.consumers, vec![path("src/cli")]);
        assert_eq!(changed.notify, vec![path("src/client"), path("src/cli")]);
        let mut cleared = new("Foo", json!({"v": 4}));
        cleared.consumers = Some(Vec::new());
        let none = reg.publish(agent("a"), cleared, t0()).unwrap();
        assert!(none.contract.consumers.is_empty());
        assert_eq!(none.notify, vec![path("src/cli")]);
    }

    #[test]
    fn a_stale_expected_version_is_refused_and_a_matching_one_publishes() {
        let mut reg = ContractRegistry::default();
        let mut first = new("Foo", json!({}));
        first.expected_version = Some(0);
        reg.publish(agent("a"), first, t0()).unwrap();
        let mut stale = new("Foo", json!({"v": 2}));
        stale.expected_version = Some(0);
        assert_eq!(
            reg.publish(agent("b"), stale, t0()).unwrap_err(),
            ContractError::VersionConflict {
                name: "Foo".into(),
                current: 1,
                expected: 0,
                published_by: Some(agent("a")),
            }
        );
        let mut fresh = new("Foo", json!({"v": 2}));
        fresh.expected_version = Some(1);
        assert_eq!(
            reg.publish(agent("b"), fresh, t0())
                .unwrap()
                .contract
                .current
                .version,
            2
        );
        let mut absent = new("Bar", json!({}));
        absent.expected_version = Some(3);
        assert_eq!(
            reg.publish(agent("b"), absent, t0()).unwrap_err(),
            ContractError::VersionConflict {
                name: "Bar".into(),
                current: 0,
                expected: 3,
                published_by: None,
            }
        );
    }

    #[test]
    fn republishing_bumps_version_and_keeps_history() {
        let mut reg = ContractRegistry::default();
        let first = reg
            .publish(agent("a"), new("POST /x", json!({"v": 1})), t0())
            .unwrap();
        assert_eq!(first.previous_version, None);
        assert_eq!(first.contract.current.version, 1);
        let second = reg
            .publish(agent("b"), new("POST /x", json!({"v": 2})), t0())
            .unwrap();
        assert_eq!(second.previous_version, Some(1));
        assert_eq!(second.contract.current.version, 2);
        assert_eq!(second.contract.history.len(), 1);
        assert_eq!(second.contract.id, first.contract.id);
        assert_eq!(reg.contracts().len(), 1);
    }

    #[test]
    fn lookup_by_name_id_or_id_prefix_and_filter_by_consumer() {
        let mut reg = ContractRegistry::default();
        let published = reg
            .publish(agent("a"), new("Foo", json!({})), t0())
            .unwrap();
        assert!(reg.get("Foo").is_some());
        assert!(reg.get(&published.contract.id.to_string()).is_some());
        assert!(reg.get(&published.contract.id.short()).is_some());
        assert!(reg.get("Bar").is_none());
        let inside = RepoPath::new("src/client/sessions.rs").unwrap();
        let outside = RepoPath::new("src/server").unwrap();
        assert_eq!(reg.list(Some(&inside), None).len(), 1);
        assert!(reg.list(Some(&outside), None).is_empty());
        assert!(reg.list(None, Some(ContractKind::Cli)).is_empty());
    }

    #[test]
    fn empty_name_is_refused() {
        let mut reg = ContractRegistry::default();
        assert_eq!(
            reg.publish(agent("a"), new("  ", json!({})), t0()).err(),
            Some(ContractError::EmptyName)
        );
    }
}
