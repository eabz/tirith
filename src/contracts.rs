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

use crate::types::{AgentId, ContractId, RepoPath};

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

/// Input for publishing a contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewContract {
    /// See [`Contract::name`].
    pub name: String,
    /// See [`Contract::kind`].
    pub kind: ContractKind,
    /// See [`ContractVersion::shape`].
    pub shape: Value,
    /// See [`Contract::consumers`].
    pub consumers: Vec<RepoPath>,
    /// See [`ContractVersion::notes`].
    pub notes: String,
}

/// The result of publishing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    /// The contract after publishing.
    pub contract: Contract,
    /// The version that was current before, if the name already existed.
    pub previous_version: Option<u32>,
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
            existing.consumers = new.consumers;
            return Ok(Published {
                contract: existing.clone(),
                previous_version: Some(previous_version),
            });
        }
        let contract = Contract {
            id: ContractId::new(),
            name,
            kind: new.kind,
            consumers: new.consumers,
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
        })
    }

    /// Finds a contract by exact name or by id string.
    pub fn get(&self, name_or_id: &str) -> Option<&Contract> {
        let key = name_or_id.trim();
        self.contracts.iter().find(|c| c.name == key).or_else(|| {
            ContractId::parse(key)
                .ok()
                .and_then(|id| self.contracts.iter().find(|c| c.id == id))
        })
    }

    /// Contracts matching the optional consumer-path and kind filters.
    pub fn list(&self, path: Option<&RepoPath>, kind: Option<ContractKind>) -> Vec<&Contract> {
        self.contracts
            .iter()
            .filter(|c| path.is_none_or(|p| c.consumed_by(p)))
            .filter(|c| kind.is_none_or(|k| c.kind == k))
            .collect()
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
        NewContract {
            name: name.into(),
            kind: ContractKind::Http,
            shape,
            consumers: vec![RepoPath::new("src/client").unwrap()],
            notes: String::new(),
        }
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
    fn lookup_by_name_or_id_and_filter_by_consumer() {
        let mut reg = ContractRegistry::default();
        let published = reg
            .publish(agent("a"), new("Foo", json!({})), t0())
            .unwrap();
        assert!(reg.get("Foo").is_some());
        assert!(reg.get(&published.contract.id.to_string()).is_some());
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
