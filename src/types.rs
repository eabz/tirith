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
uuid_id!(
    /// Identifies an agent-to-agent message.
    MessageId
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
    /// A `#` was followed by no usable symbol anchor (ADR-0029).
    #[error("symbol anchor after '#' is empty: {0:?}")]
    EmptyAnchor(String),
    /// An anchor segment was empty (`A::`, `a..b`) or contained whitespace.
    #[error("symbol anchor has an empty or spaced segment: {0:?}")]
    BadAnchorSegment(String),
    /// The path was written as a directory (trailing `/`), which has no
    /// symbols to anchor.
    #[error("a directory cannot carry a symbol anchor: {0:?}")]
    AnchorOnDirectory(String),
}

/// A normalized path relative to the repository root, optionally anchored
/// to a symbol inside the file.
///
/// Normalization removes `./`, duplicate and trailing slashes, and converts
/// backslashes. A path covers itself and everything beneath it, so
/// `src/auth` overlaps `src/auth/login.rs` but not `src/authz`. See
/// ADR-0005.
///
/// A path may carry a symbol anchor after the first `#`
/// (`src/state.rs#State::brief`, ADR-0029, experimental). Anchor segments
/// may be separated by `::`, `.` or `/`; they are stored with `::`. Generic
/// arguments (`Store<T>`) and a Go receiver (`(*Server).Handle`) are
/// dropped. Only the syntax is checked, never that the symbol exists.
///
/// Two comparisons exist. [`overlaps`](Self::overlaps) ignores anchors and
/// answers "same file or directory tree": briefs, notices, contracts,
/// decisions and memory match that way. [`claim_overlaps`](Self::claim_overlaps)
/// is the claim rule: anchors in one file overlap only when equal or
/// nested.
///
/// ```
/// use tirith::types::RepoPath;
///
/// let dir = RepoPath::new("./src/auth/").unwrap();
/// let file = RepoPath::new("src/auth/login.rs").unwrap();
/// assert_eq!(dir.as_str(), "src/auth");
/// assert!(dir.overlaps(&file));
/// assert!(!dir.overlaps(&RepoPath::new("src/authz/x.rs").unwrap()));
///
/// let brief = RepoPath::new("src/state.rs#State.brief").unwrap();
/// let claim = RepoPath::new("src/state.rs#State::claim").unwrap();
/// assert_eq!(brief.as_str(), "src/state.rs#State::brief");
/// assert!(!brief.claim_overlaps(&claim));
/// assert!(brief.overlaps(&claim));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepoPath(String);

/// Separator between anchor segments in the stored form.
const ANCHOR_SEP: &str = "::";

impl RepoPath {
    /// Normalizes `raw`, rejecting absolute paths, `..` segments, and
    /// malformed symbol anchors.
    pub fn new(raw: &str) -> Result<Self, PathError> {
        let trimmed = raw.trim();
        let (path_raw, anchor_raw) = match trimmed.split_once('#') {
            Some((path, anchor)) => (path, Some(anchor)),
            None => (trimmed, None),
        };
        let path = normalize_path(path_raw, raw)?;
        let Some(anchor_raw) = anchor_raw else {
            return Ok(Self(path));
        };
        let written = path_raw.trim_end();
        if written.ends_with('/') || written.ends_with('\\') {
            return Err(PathError::AnchorOnDirectory(raw.to_owned()));
        }
        let segments = parse_anchor(anchor_raw, raw)?;
        Ok(Self(format!("{path}#{}", segments.join(ANCHOR_SEP))))
    }

    /// The normalized path as a string slice, anchor included.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The path without its anchor.
    pub fn file(&self) -> &str {
        self.0
            .split_once('#')
            .map_or(self.0.as_str(), |(path, _)| path)
    }

    /// The symbol anchor in stored form (`State::brief`), if any.
    pub fn anchor(&self) -> Option<&str> {
        self.0.split_once('#').map(|(_, anchor)| anchor)
    }

    /// Whether the two paths cover a common file, ignoring anchors: equal,
    /// or one is an ancestor directory of the other.
    pub fn overlaps(&self, other: &Self) -> bool {
        paths_overlap(self.file(), other.file())
    }

    /// Whether claims on the two targets conflict (ADR-0029): their paths
    /// overlap, and either side has no anchor, or the paths differ (a file
    /// and a directory named alike, which a real tree cannot have), or one
    /// anchor equals or encloses the other by whole segments, compared
    /// ASCII case-insensitively.
    pub fn claim_overlaps(&self, other: &Self) -> bool {
        if !self.overlaps(other) {
            return false;
        }
        match (self.anchor(), other.anchor()) {
            (Some(a), Some(b)) if self.file() == other.file() => a
                .split(ANCHOR_SEP)
                .zip(b.split(ANCHOR_SEP))
                .all(|(x, y)| x.eq_ignore_ascii_case(y)),
            _ => true,
        }
    }

    /// Whether a claim on `self` already covers everything `other` names:
    /// `other` is the same path or lies beneath it, and when `self` is
    /// anchored, `other` is in the same file under an equal or enclosed
    /// anchor. An agent's own covered targets are renewed, not added.
    pub fn covers(&self, other: &Self) -> bool {
        match self.anchor() {
            None => self.file() == other.file() || is_ancestor(self.file(), other.file()),
            Some(held) => {
                self.file() == other.file()
                    && other.anchor().is_some_and(|target| {
                        let held: Vec<&str> = held.split(ANCHOR_SEP).collect();
                        let target: Vec<&str> = target.split(ANCHOR_SEP).collect();
                        held.len() <= target.len()
                            && held
                                .iter()
                                .zip(&target)
                                .all(|(x, y)| x.eq_ignore_ascii_case(y))
                    })
            }
        }
    }

    /// Whether `self` is a strict ancestor directory of `other`, comparing
    /// paths only.
    pub fn is_ancestor_of(&self, other: &Self) -> bool {
        is_ancestor(self.file(), other.file())
    }
}

/// ADR-0005 on bare path strings: equal, or one is an ancestor of the other.
fn paths_overlap(a: &str, b: &str) -> bool {
    a == b || is_ancestor(a, b) || is_ancestor(b, a)
}

/// Whether `dir` is a strict ancestor directory of `path`.
fn is_ancestor(dir: &str, path: &str) -> bool {
    path.len() > dir.len() && path.starts_with(dir) && path.as_bytes()[dir.len()] == b'/'
}

/// Forward slashes, no empty or `.` segments, no absolute paths, no `..`.
fn normalize_path(path: &str, raw: &str) -> Result<String, PathError> {
    let cleaned = path.trim().replace('\\', "/");
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
    Ok(segments.join("/"))
}

/// Splits an anchor on `::`, `.` or `/`, after dropping generic arguments
/// and a Go receiver's parentheses and `*`.
fn parse_anchor(anchor: &str, raw: &str) -> Result<Vec<String>, PathError> {
    let stripped = strip_generics(anchor.trim());
    let receiverless: String = if stripped.starts_with('(') {
        stripped
            .chars()
            .filter(|c| !matches!(c, '(' | ')' | '*'))
            .collect()
    } else {
        stripped
    };
    if receiverless.trim().is_empty() {
        return Err(PathError::EmptyAnchor(raw.to_owned()));
    }
    let unified = receiverless.replace(ANCHOR_SEP, "/").replace('.', "/");
    unified
        .split('/')
        .map(|segment| {
            let segment = segment.trim();
            if segment.is_empty() || segment.chars().any(char::is_whitespace) {
                Err(PathError::BadAnchorSegment(raw.to_owned()))
            } else {
                Ok(segment.to_owned())
            }
        })
        .collect()
}

/// Removes every `<...>` group, nested or not; an unbalanced `>` is kept.
fn strip_generics(anchor: &str) -> String {
    let mut depth = 0usize;
    let mut out = String::with_capacity(anchor.len());
    for c in anchor.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
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

    // --- ADR-0029: symbol anchors (ported from the claims-symbols prototype) ---

    /// Claim overlap, asserted symmetric.
    fn ov(a: &str, b: &str) -> bool {
        let (a, b) = (RepoPath::new(a).unwrap(), RepoPath::new(b).unwrap());
        let forward = a.claim_overlaps(&b);
        assert_eq!(
            forward,
            b.claim_overlaps(&a),
            "must be symmetric: {a} / {b}"
        );
        forward
    }

    fn norm(raw: &str) -> String {
        RepoPath::new(raw).unwrap().to_string()
    }

    #[test]
    fn plain_paths_keep_their_normalization_and_have_no_anchor() {
        assert_eq!(norm("./src//auth/"), "src/auth");
        assert_eq!(norm("src\\state.rs"), "src/state.rs");
        let plain = RepoPath::new("src/state.rs").unwrap();
        assert_eq!(plain.anchor(), None);
        assert_eq!(plain.file(), "src/state.rs");
    }

    #[test]
    fn anchor_separators_normalize_to_double_colon() {
        assert_eq!(
            norm("src/state.rs#State::brief"),
            "src/state.rs#State::brief"
        );
        assert_eq!(
            norm("app/config.py#Config.from_env"),
            "app/config.py#Config::from_env"
        );
        assert_eq!(
            norm("src/state.rs#State/brief"),
            "src/state.rs#State::brief"
        );
        assert_eq!(
            norm(" ./src/state.rs # State :: brief "),
            "src/state.rs#State::brief"
        );
    }

    #[test]
    fn go_receivers_and_generics_are_stripped() {
        assert_eq!(
            norm("pkg/server.go#(*Server).Handle"),
            "pkg/server.go#Server::Handle"
        );
        assert_eq!(
            norm("pkg/server.go#(Server).Handle"),
            "pkg/server.go#Server::Handle"
        );
        assert_eq!(
            norm("src/store.rs#Store<T>::load"),
            "src/store.rs#Store::load"
        );
        assert_eq!(norm("src/a.ts#Cache<Map<K, V>>.get"), "src/a.ts#Cache::get");
    }

    #[test]
    fn only_the_first_hash_separates_the_anchor() {
        let target = RepoPath::new("src/counter.ts#Counter.#count").unwrap();
        assert_eq!(target.file(), "src/counter.ts");
        assert_eq!(target.anchor(), Some("Counter::#count"));
    }

    #[test]
    fn invalid_anchors_are_rejected() {
        assert_eq!(RepoPath::new("#State"), Err(PathError::Empty));
        assert!(matches!(
            RepoPath::new("/etc/passwd#x"),
            Err(PathError::Absolute(_))
        ));
        assert!(matches!(
            RepoPath::new("C:/x.rs#A"),
            Err(PathError::Absolute(_))
        ));
        assert!(matches!(
            RepoPath::new("../x.rs#A"),
            Err(PathError::Traversal(_))
        ));
        for empty in ["src/state.rs#", "src/state.rs#  ", "src/state.rs#<T>"] {
            assert!(
                matches!(RepoPath::new(empty), Err(PathError::EmptyAnchor(_))),
                "{empty}"
            );
        }
        for bad in [
            "src/state.rs#State::",
            "src/state.rs#::brief",
            "src/a.py#Config..x",
            "src/a.rs#impl Foo",
        ] {
            assert!(
                matches!(RepoPath::new(bad), Err(PathError::BadAnchorSegment(_))),
                "{bad}"
            );
        }
        assert!(matches!(
            RepoPath::new("src/#State"),
            Err(PathError::AnchorOnDirectory(_))
        ));
    }

    #[test]
    fn plain_paths_follow_adr_0005() {
        assert!(ov("src/state.rs", "src/state.rs"));
        assert!(ov("src/", "src/state.rs"));
        assert!(ov("src/auth", "src/auth/login.rs"));
        assert!(!ov("src/auth/", "src/authz/login.rs"));
        assert!(!ov("src/state.rs", "src/server.rs"));
    }

    #[test]
    fn a_file_claim_overlaps_every_anchor_in_the_file() {
        assert!(ov("src/state.rs", "src/state.rs#State::brief"));
        assert!(ov("app/router.py", "app/router.py#build_pipeline"));
    }

    #[test]
    fn a_directory_claim_overlaps_anchors_below_it() {
        assert!(ov("src/", "src/state.rs#State::brief"));
        assert!(ov(
            "app/middlewares/",
            "app/middlewares/__init__.py#__all__"
        ));
    }

    #[test]
    fn anchors_never_reach_other_files() {
        assert!(!ov("src/server.rs", "src/state.rs#State::brief"));
        assert!(!ov("src/state.rs#State", "src/state_test.rs#State"));
        assert!(!ov("src/auth/", "src/authz.rs#Login"));
        assert!(!ov("src/state.rs#State", "src/server.rs#State"));
    }

    #[test]
    fn equal_anchors_overlap() {
        assert!(ov(
            "app/router.py#build_pipeline",
            "app/router.py#build_pipeline"
        ));
        assert!(ov("src/state.rs#State::brief", "src/state.rs#State.brief"));
    }

    #[test]
    fn enclosing_anchors_overlap() {
        assert!(ov("app/config.py#Config", "app/config.py#Config::from_env"));
        assert!(ov("src/state.rs#State", "src/state.rs#State::brief::inner"));
    }

    #[test]
    fn sibling_anchors_do_not_overlap() {
        assert!(!ov(
            "app/config.py#Config::from_env",
            "app/config.py#Config::rate_limit_burst"
        ));
        assert!(!ov(
            "src/state.rs#State::brief",
            "src/state.rs#State::claim"
        ));
        assert!(!ov("src/state.rs#State", "src/state.rs#Claims"));
    }

    #[test]
    fn new_members_are_siblings_of_each_other() {
        assert!(!ov(
            "app/config.py#Config::access_log",
            "app/config.py#Config::gzip_min_size"
        ));
        assert!(ov(
            "app/config.py#Config",
            "app/config.py#Config::access_log"
        ));
    }

    #[test]
    fn segment_prefixes_are_whole_segments() {
        assert!(!ov("src/state.rs#State", "src/state.rs#StateView"));
        assert!(!ov(
            "src/state.rs#State::brief",
            "src/state.rs#State::brief_page"
        ));
    }

    #[test]
    fn anchors_compare_case_insensitively() {
        assert!(ov("pkg/server.go#Server", "pkg/server.go#server"));
        assert!(ov(
            "pkg/server.go#(*Server).Handle",
            "pkg/server.go#server.handle"
        ));
    }

    #[test]
    fn the_same_name_in_different_containers_does_not_overlap() {
        assert!(!ov(
            "src/types.rs#RepoPath::fmt",
            "src/types.rs#AgentId::fmt"
        ));
        assert!(ov(
            "src/types.rs#RepoPath::fmt",
            "src/types.rs#RepoPath::fmt"
        ));
    }

    #[test]
    fn anchored_paths_that_nest_are_treated_as_overlapping() {
        // Not a real tree (a file cannot also be a directory); stay safe.
        assert!(ov("src/auth#Login", "src/auth/login.rs#Login"));
    }

    #[test]
    fn e2e_feature_tasks_share_only_build_pipeline_and_from_env() {
        let rate_limit = [
            "app/config.py#Config::rate_limit_per_second",
            "app/config.py#Config::rate_limit_burst",
            "app/config.py#Config::from_env",
            "app/router.py#build_pipeline",
        ];
        let access_log = [
            "app/config.py#Config::access_log",
            "app/router.py#build_pipeline",
        ];
        let cors = [
            "app/config.py#Config::cors_allowed_origins",
            "app/config.py#Config::from_env",
            "app/router.py#build_pipeline",
        ];
        let clash = |a: &[&str], b: &[&str]| -> Vec<String> {
            let mut hits: Vec<String> = a
                .iter()
                .filter(|x| b.iter().any(|y| ov(x, y)))
                .map(|x| (*x).to_owned())
                .collect();
            hits.dedup();
            hits
        };
        assert_eq!(
            clash(&rate_limit, &access_log),
            ["app/router.py#build_pipeline"]
        );
        assert_eq!(
            clash(&rate_limit, &cors),
            [
                "app/config.py#Config::from_env",
                "app/router.py#build_pipeline"
            ]
        );
    }

    #[test]
    fn file_level_overlap_ignores_anchors() {
        let brief = RepoPath::new("app/router.py#build_pipeline").unwrap();
        let other = RepoPath::new("app/router.py#Router::add").unwrap();
        let dir = RepoPath::new("app").unwrap();
        assert!(brief.overlaps(&other));
        assert!(dir.overlaps(&brief));
        assert!(!brief.is_ancestor_of(&other));
        assert!(dir.is_ancestor_of(&brief));
    }

    #[test]
    fn covers_decides_renewal_and_absorption() {
        let p = |raw: &str| RepoPath::new(raw).unwrap();
        assert!(p("src").covers(&p("src/state.rs#State")));
        assert!(p("src/state.rs").covers(&p("src/state.rs#State")));
        assert!(p("src/state.rs#State").covers(&p("src/state.rs#state::brief")));
        assert!(p("src/state.rs#State").covers(&p("src/state.rs#State")));
        assert!(!p("src/state.rs#State::brief").covers(&p("src/state.rs#State")));
        assert!(!p("src/state.rs#State").covers(&p("src/state.rs")));
        assert!(!p("src/state.rs#State").covers(&p("src/state.rs#StateView")));
        assert!(!p("src/auth#X").covers(&p("src/auth/x.rs#X")));
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
