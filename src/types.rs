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

            /// The all-zero identifier, which sorts before every real one.
            /// Used as the id half of a paging cursor built from a bare
            /// timestamp, so "before T" excludes every row at T.
            pub fn nil() -> Self {
                Self(Uuid::nil())
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
uuid_id!(
    /// Identifies a memory note across its revisions.
    MemoryId
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

/// Rows a list tool returns when the caller gives no `limit`.
pub const DEFAULT_LIST_LIMIT: usize = 20;
/// The most rows a list tool returns in one call, whatever the caller asks.
pub const MAX_LIST_LIMIT: usize = 200;

/// Clamps a caller-supplied limit to `1..=MAX_LIST_LIMIT`, defaulting to
/// [`DEFAULT_LIST_LIMIT`].
///
/// ```
/// use tirith::types::{clamp_limit, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT};
///
/// assert_eq!(clamp_limit(None), DEFAULT_LIST_LIMIT);
/// assert_eq!(clamp_limit(Some(0)), 1);
/// assert_eq!(clamp_limit(Some(1_000_000)), MAX_LIST_LIMIT);
/// ```
pub fn clamp_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT)
}

/// A bounded, newest-first slice of a listing.
///
/// Every list tool returns one of these so an agent never receives more
/// than [`MAX_LIST_LIMIT`] rows and can page older rows with a cursor.
/// Rows are keyed by `(timestamp, id)`, never by timestamp alone, so two
/// rows written in the same instant are never skipped between pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    /// The rows, newest first.
    pub items: Vec<T>,
    /// How many rows matched before the limit was applied.
    pub total: usize,
}

impl<T> Page<T> {
    /// Orders `rows` newest first by `key`, drops rows whose key is at or
    /// after `before`, and keeps the first `limit`.
    ///
    /// ```
    /// use chrono::{TimeZone, Utc};
    /// use tirith::types::Page;
    ///
    /// let t = |s: u32| Utc.with_ymd_and_hms(2026, 9, 16, 0, 0, s).unwrap();
    /// // Two rows share a timestamp; the id breaks the tie.
    /// let rows = [("a", t(1), 1), ("b", t(2), 2), ("c", t(2), 3)];
    /// let key = |r: &(&str, chrono::DateTime<Utc>, i32)| (r.1, r.2);
    /// let page = Page::newest_first(rows.iter().copied(), key, None, 1);
    /// assert_eq!(page.items[0].0, "c");
    /// assert_eq!(page.total, 3);
    /// let cursor = page.next_before(key).unwrap();
    /// let next = Page::newest_first(rows.iter().copied(), key, Some(&cursor), 1);
    /// assert_eq!(next.items[0].0, "b", "same timestamp, older id");
    /// let cursor = next.next_before(key);
    /// let last = Page::newest_first(rows.iter().copied(), key, cursor.as_ref(), 1);
    /// assert_eq!(last.items[0].0, "a");
    /// assert!(!last.truncated());
    /// ```
    pub fn newest_first<K: Ord>(
        rows: impl IntoIterator<Item = T>,
        key: impl Fn(&T) -> K,
        before: Option<&K>,
        limit: usize,
    ) -> Self {
        let mut items: Vec<T> = rows
            .into_iter()
            .filter(|row| before.is_none_or(|b| key(row) < *b))
            .collect();
        items.sort_by_cached_key(|row| std::cmp::Reverse(key(row)));
        let total = items.len();
        items.truncate(limit);
        Self { items, total }
    }

    /// Whether rows were dropped to honor the limit.
    pub fn truncated(&self) -> bool {
        self.items.len() < self.total
    }

    /// The cursor for the next page: the key of the oldest row returned,
    /// or `None` when nothing was truncated.
    pub fn next_before<K>(&self, key: impl Fn(&T) -> K) -> Option<K> {
        if self.truncated() {
            self.items.last().map(key)
        } else {
            None
        }
    }

    /// Applies `f` to every row, keeping `total`.
    pub fn map<U>(self, f: impl FnMut(T) -> U) -> Page<U> {
        Page {
            items: self.items.into_iter().map(f).collect(),
            total: self.total,
        }
    }
}

/// Why an id given by a caller could not be resolved to one item.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PrefixError {
    /// Nothing starts with the given characters.
    #[error("no {kind} id starts with {prefix:?}")]
    NotFound {
        /// The identifier kind, for the message.
        kind: &'static str,
        /// What the caller gave.
        prefix: String,
    },
    /// More than one id starts with the given characters.
    #[error("{kind} id prefix {prefix:?} matches {count} ids; give more characters")]
    Ambiguous {
        /// The identifier kind, for the message.
        kind: &'static str,
        /// What the caller gave.
        prefix: String,
        /// How many ids matched.
        count: usize,
    },
}

/// Resolves `raw` to exactly one of `ids`. A full id matches itself;
/// otherwise `raw` must be a prefix (hyphens ignored, case-insensitive)
/// of exactly one id, so the eight-character `short()` form works when
/// it is unique.
///
/// ```
/// use tirith::types::{NoticeId, resolve_prefix};
///
/// let id = NoticeId::new();
/// let ids = [id, NoticeId::new()];
/// assert_eq!(resolve_prefix("notice", ids, &id.short()), Ok(id));
/// assert_eq!(resolve_prefix("notice", ids, &id.to_string()), Ok(id));
/// assert!(resolve_prefix("notice", ids, "").is_err());
/// ```
pub fn resolve_prefix<I: fmt::Display + Copy>(
    kind: &'static str,
    ids: impl IntoIterator<Item = I>,
    raw: &str,
) -> Result<I, PrefixError> {
    let prefix: String = raw
        .trim()
        .chars()
        .filter(|c| *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let not_found = || PrefixError::NotFound {
        kind,
        prefix: raw.trim().to_owned(),
    };
    if prefix.is_empty() {
        return Err(not_found());
    }
    let mut matches = ids.into_iter().filter(|id| {
        id.to_string()
            .chars()
            .filter(|c| *c != '-')
            .map(|c| c.to_ascii_lowercase())
            .collect::<String>()
            .starts_with(&prefix)
    });
    let first = matches.next().ok_or_else(not_found)?;
    let extra = matches.count();
    if extra > 0 {
        return Err(PrefixError::Ambiguous {
            kind,
            prefix: raw.trim().to_owned(),
            count: extra + 1,
        });
    }
    Ok(first)
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
    fn prefix_resolution_is_unique_or_refused() {
        let a = NoticeId::parse("11111111-2222-3333-4444-555555555555").unwrap();
        let b = NoticeId::parse("11111111-2222-3333-4444-666666666666").unwrap();
        let c = NoticeId::parse("aaaaaaaa-2222-3333-4444-666666666666").unwrap();
        let ids = [a, b, c];
        assert_eq!(resolve_prefix("notice", ids, "AAAA"), Ok(c));
        assert_eq!(
            resolve_prefix("notice", ids, "11111111-2222-3333-4444-5"),
            Ok(a)
        );
        assert!(matches!(
            resolve_prefix("notice", ids, "11111111"),
            Err(PrefixError::Ambiguous { count: 2, .. })
        ));
        assert!(matches!(
            resolve_prefix("notice", ids, "zzzz"),
            Err(PrefixError::NotFound { .. })
        ));
    }

    #[test]
    fn ids_round_trip_through_strings() {
        let id = TaskId::new();
        assert_eq!(TaskId::parse(&id.to_string()).unwrap(), id);
        assert_eq!(id.short().len(), 8);
        assert!(TaskId::parse("nope").is_err());
    }
}
