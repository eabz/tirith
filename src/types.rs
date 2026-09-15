//! Identifier and path newtypes shared by every primitive.
//!
//! Nothing in Tirith passes a bare `String` where an agent, a claim, or a
//! repository path is meant. Construction validates; the wire format stays
//! a plain string through `#[serde(transparent)]`.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Errors from constructing an identifier.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdError {
    /// The agent name was empty after trimming.
    #[error("agent name must not be empty")]
    EmptyAgent,
    /// The agent name exceeded the maximum length.
    #[error("agent name longer than {max} characters")]
    AgentTooLong {
        /// The limit that was exceeded.
        max: usize,
    },
    /// A UUID-based identifier did not parse.
    #[error("invalid {kind}: {value:?}")]
    Invalid {
        /// Which identifier type was expected.
        kind: &'static str,
        /// The rejected input.
        value: String,
    },
}

/// Maximum length of an agent name.
pub const MAX_AGENT_LEN: usize = 64;

/// The caller-supplied name of an agent. Stable within a session, unique
/// across agents. See ADR-0004.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(String);

impl AgentId {
    /// Validates and trims an agent name.
    pub fn new(raw: impl AsRef<str>) -> Result<Self, IdError> {
        let trimmed = raw.as_ref().trim();
        if trimmed.is_empty() {
            return Err(IdError::EmptyAgent);
        }
        if trimmed.chars().count() > MAX_AGENT_LEN {
            return Err(IdError::AgentTooLong { max: MAX_AGENT_LEN });
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a fresh random identifier.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Parses the string form produced by [`Display`](fmt::Display).
            pub fn parse(raw: &str) -> Result<Self, IdError> {
                Uuid::parse_str(raw.trim())
                    .map(Self)
                    .map_err(|_| IdError::Invalid {
                        kind: stringify!($name),
                        value: raw.to_owned(),
                    })
            }

            /// The first eight hex characters, for compact display.
            pub fn short(&self) -> String {
                self.0.simple().to_string()[..8].to_owned()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

uuid_id!(
    /// Identifies one claim (one agent, one or more paths).
    ClaimId
);
uuid_id!(
    /// Identifies a task on the board.
    TaskId
);
uuid_id!(
    /// Identifies a contract across all its versions.
    ContractId
);
uuid_id!(
    /// Identifies a change notice.
    NoticeId
);
uuid_id!(
    /// Identifies a recorded decision.
    DecisionId
);

/// Errors from normalizing a repository path.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathError {
    /// The path was empty or resolved to the repository root.
    #[error("path must not be empty")]
    Empty,
    /// The path was absolute; only repo-relative paths are accepted.
    #[error("path must be relative to the repository root: {0:?}")]
    Absolute(String),
    /// The path contained a `..` segment.
    #[error("path must not contain '..': {0:?}")]
    Traversal(String),
}

/// A normalized path relative to the repository root.
///
/// Normalization removes `./`, duplicate and trailing slashes, and converts
/// backslashes. A path covers itself and everything beneath it, so
/// `src/auth` overlaps `src/auth/login.rs` but not `src/authz`. See
/// ADR-0005.
///
/// ```
/// use tirith::types::RepoPath;
///
/// let dir = RepoPath::new("./src/auth/").unwrap();
/// let file = RepoPath::new("src/auth/login.rs").unwrap();
/// assert_eq!(dir.as_str(), "src/auth");
/// assert!(dir.overlaps(&file));
/// assert!(!dir.overlaps(&RepoPath::new("src/authz/x.rs").unwrap()));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepoPath(String);

impl RepoPath {
    /// Normalizes `raw`, rejecting absolute paths and `..` segments.
    pub fn new(raw: &str) -> Result<Self, PathError> {
        let cleaned = raw.trim().replace('\\', "/");
        if cleaned.is_empty() {
            return Err(PathError::Empty);
        }
        let looks_absolute = cleaned.starts_with('/')
            || cleaned
                .as_bytes()
                .get(1)
                .is_some_and(|b| *b == b':' && cleaned.as_bytes()[0].is_ascii_alphabetic());
        if looks_absolute {
            return Err(PathError::Absolute(raw.to_owned()));
        }
        let mut segments = Vec::new();
        for segment in cleaned.split('/') {
            match segment {
                "" | "." => {}
                ".." => return Err(PathError::Traversal(raw.to_owned())),
                other => segments.push(other),
            }
        }
        if segments.is_empty() {
            return Err(PathError::Empty);
        }
        Ok(Self(segments.join("/")))
    }

    /// The normalized path as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether the two paths cover a common file: equal, or one is an
    /// ancestor directory of the other.
    pub fn overlaps(&self, other: &Self) -> bool {
        self == other || self.is_ancestor_of(other) || other.is_ancestor_of(self)
    }

    /// Whether `self` is a strict ancestor directory of `other`.
    pub fn is_ancestor_of(&self, other: &Self) -> bool {
        other.0.len() > self.0.len()
            && other.0.starts_with(&self.0)
            && other.0.as_bytes()[self.0.len()] == b'/'
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Parses and normalizes a list of raw paths, keeping the first error.
pub fn parse_paths<I, S>(raw: I) -> Result<Vec<RepoPath>, PathError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = Vec::new();
    for item in raw {
        let path = RepoPath::new(item.as_ref())?;
        if !out.contains(&path) {
            out.push(path);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_names_are_trimmed_and_non_empty() {
        assert_eq!(AgentId::new("  alice ").unwrap().as_str(), "alice");
        assert_eq!(AgentId::new("   "), Err(IdError::EmptyAgent));
        assert!(AgentId::new("x".repeat(65)).is_err());
    }

    #[test]
    fn paths_are_normalized() {
        assert_eq!(RepoPath::new("./src//auth/").unwrap().as_str(), "src/auth");
        assert_eq!(
            RepoPath::new("src\\auth\\a.rs").unwrap().as_str(),
            "src/auth/a.rs"
        );
        assert_eq!(RepoPath::new(" . "), Err(PathError::Empty));
        assert!(matches!(
            RepoPath::new("/etc/passwd"),
            Err(PathError::Absolute(_))
        ));
        assert!(matches!(RepoPath::new("C:/x"), Err(PathError::Absolute(_))));
        assert!(matches!(
            RepoPath::new("src/../x"),
            Err(PathError::Traversal(_))
        ));
    }

    #[test]
    fn overlap_is_at_segment_boundaries() {
        let auth = RepoPath::new("src/auth").unwrap();
        let login = RepoPath::new("src/auth/login.rs").unwrap();
        let authz = RepoPath::new("src/authz/x.rs").unwrap();
        let src = RepoPath::new("src").unwrap();
        assert!(auth.overlaps(&login));
        assert!(login.overlaps(&auth));
        assert!(!auth.overlaps(&authz));
        assert!(src.overlaps(&login));
        assert!(auth.overlaps(&auth));
    }

    #[test]
    fn parse_paths_dedupes() {
        let paths = parse_paths(["a/b", "./a/b/", "c"]).unwrap();
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn ids_round_trip_through_strings() {
        let id = TaskId::new();
        assert_eq!(TaskId::parse(&id.to_string()).unwrap(), id);
        assert_eq!(id.short().len(), 8);
        assert!(TaskId::parse("nope").is_err());
    }
}
