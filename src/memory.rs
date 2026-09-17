//! Memory notes: durable, path-scoped project knowledge.
//!
//! The other primitives record what is happening right now: who holds a
//! file, what changed, what was decided. Memory records what an agent
//! learned and wants the next agent to know, in prose, months later.
//!
//! Two things make this different from a general-purpose memory server:
//!
//! - **Notes are scoped to repository paths.** A note about
//!   `src/store.rs` is attached to that path, so it can be handed to
//!   whoever claims it instead of waiting to be searched for.
//! - **Notes are Markdown files with YAML frontmatter**, one file per
//!   note, committed to the repository. They are readable in a pull
//!   request, editable by hand, and use the line formats of Basic Memory,
//!   which is where this repository's own notes came from (ADR-0012), so
//!   notes written by other Markdown memory tools import without
//!   translation. See ADR-0011.
//!
//! This module is pure: it owns the note type, the file format, and the
//! in-memory index with its search. It performs no I/O and knows nothing
//! about MCP. [`MemoryNote::to_markdown`] and
//! [`MemoryNote::from_markdown`] are the seam the store writes through.

use std::collections::{HashMap, HashSet};
use std::fmt;

use chrono::{DateTime, SecondsFormat, SubsecRound, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::{AgentId, MemoryId, PathError, RepoPath};

/// The longest a note title may be.
///
/// Matched to [`MAX_PERMALINK_LEN`] so a title cannot be long enough to be
/// truncated away into a permalink it shares with a different title.
pub const MAX_TITLE_LEN: usize = 120;

/// The longest a permalink may be.
pub const MAX_PERMALINK_LEN: usize = 120;

/// The most `/`-separated segments a permalink may have.
pub const MAX_PERMALINK_DEPTH: usize = 8;

/// How many results a search returns when the caller does not say.
///
/// Searching runs under the daemon's single state lock, so an unbounded
/// search would make every other agent wait for it.
pub const DEFAULT_SEARCH_LIMIT: usize = 10;

/// The most results a single search may return.
pub const MAX_SEARCH_LIMIT: usize = 50;

/// The most relation hops [`MemoryBook::context`] may be asked to walk.
///
/// A dense relation graph reaches every note in a few hops, so callers
/// are not allowed to ask for an unbounded walk.
pub const MAX_CONTEXT_DEPTH: usize = 3;

/// How many notes a successful claim carries back.
pub const CLAIM_MEMORY_LIMIT: usize = 5;

/// How many characters of body a claim response or listing shows.
pub const CLAIM_MEMORY_EXCERPT: usize = 160;

/// The most related notes [`MemoryBook::context`] returns.
///
/// A dense relation graph reaches everything, and the caller asked for
/// one note, not the whole book.
pub const MAX_RELATED: usize = 20;

/// What a note is for.
///
/// The kind is advisory: it shapes how an agent reads the note, and it
/// filters searches. It does not change how the note is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// Something true about the project that the code does not say.
    Fact,
    /// Something learned the hard way, with the cost of learning it.
    Lesson,
    /// A trap the next agent will otherwise fall into.
    Gotcha,
    /// The state of unfinished work, for whoever picks it up.
    Handoff,
    /// The result of an investigation: what was compared, what won.
    Research,
    /// Anything else worth keeping.
    #[default]
    Note,
    /// A settled choice recorded through `decision_record`; stored under
    /// `.tirith/decisions/` in this same format (ADR-0022).
    Decision,
}

impl MemoryKind {
    /// The lowercase name used in frontmatter and tool arguments.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Lesson => "lesson",
            Self::Gotcha => "gotcha",
            Self::Handoff => "handoff",
            Self::Research => "research",
            Self::Note => "note",
            Self::Decision => "decision",
        }
    }

    /// Every kind, for help text and validation messages.
    pub fn all() -> [Self; 7] {
        [
            Self::Fact,
            Self::Lesson,
            Self::Gotcha,
            Self::Handoff,
            Self::Research,
            Self::Note,
            Self::Decision,
        ]
    }
}

impl fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for MemoryKind {
    type Err = MemoryError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let needle = raw.trim().to_lowercase();
        Self::all()
            .into_iter()
            .find(|k| k.as_str() == needle)
            .ok_or_else(|| MemoryError::UnknownKind(raw.to_owned()))
    }
}

/// A note's stable address, used as its filename and in `[[links]]`.
///
/// A permalink is a lowercase slug derived from the title when the note is
/// created, and it never changes afterwards, so links to it keep working
/// when the title is edited.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Permalink(String);

impl Permalink {
    /// Builds a permalink from arbitrary text.
    ///
    /// Letters and digits are kept and lowercased, every other run of
    /// characters becomes a single dash.
    ///
    /// ```
    /// use tirith::memory::Permalink;
    ///
    /// let link = Permalink::from_title("Persister writes are ordered!").unwrap();
    /// assert_eq!(link.as_str(), "persister-writes-are-ordered");
    /// ```
    pub fn from_title(title: &str) -> Result<Self, MemoryError> {
        let mut out = String::new();
        let mut last_dash = true;
        for ch in title.chars().flat_map(char::to_lowercase) {
            if ch.is_ascii_alphanumeric() {
                out.push(ch);
                last_dash = false;
            } else if !last_dash {
                out.push('-');
                last_dash = true;
            }
        }
        let trimmed: String = out
            .trim_matches('-')
            .chars()
            .take(MAX_PERMALINK_LEN)
            .collect();
        let trimmed = trimmed.trim_end_matches('-').to_owned();
        if trimmed.is_empty() {
            return Err(MemoryError::EmptyPermalink);
        }
        Ok(Self(trimmed))
    }

    /// A permalink for `title`, falling back to `note-<id>` when the title
    /// slugifies to nothing.
    ///
    /// A title written entirely in a non-Latin script has no ASCII
    /// alphanumerics to slugify, and refusing to store it would be a
    /// strange thing for a notes tool to do.
    ///
    /// ```
    /// use tirith::memory::Permalink;
    /// use tirith::types::MemoryId;
    ///
    /// let id = MemoryId::new();
    /// let link = Permalink::for_title("Заметка", id);
    /// assert!(link.as_str().starts_with("note-"));
    /// ```
    pub fn for_title(title: &str, id: MemoryId) -> Self {
        Self::from_title(title).unwrap_or_else(|_| Self(format!("note-{}", id.short())))
    }

    /// The same permalink with `id` appended, for when the slug is taken
    /// by a note with a different title.
    fn disambiguated(&self, id: MemoryId) -> Self {
        let suffix = format!("-{}", id.short());
        let room = MAX_PERMALINK_LEN.saturating_sub(suffix.len());
        let stem: String = self.0.chars().take(room).collect();
        Self(format!("{}{suffix}", stem.trim_end_matches('-')))
    }

    /// Accepts an existing permalink.
    ///
    /// A permalink is one or more slug segments separated by `/`, so notes
    /// can be filed in folders the way Basic Memory files them. Every
    /// segment must be lowercase letters, digits, and dashes, which makes
    /// `.` impossible and therefore makes escaping the memory directory
    /// impossible.
    ///
    /// ```
    /// use tirith::memory::Permalink;
    ///
    /// let link = Permalink::parse("tirith/design/pre-alpha-build").unwrap();
    /// assert_eq!(link.file_path(), "tirith/design/pre-alpha-build.md");
    /// assert!(Permalink::parse("../escape").is_err());
    /// ```
    pub fn parse(raw: &str) -> Result<Self, MemoryError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(MemoryError::EmptyPermalink);
        }
        if trimmed.len() > MAX_PERMALINK_LEN {
            return Err(MemoryError::PermalinkTooLong(trimmed.to_owned()));
        }
        let segments: Vec<&str> = trimmed.split('/').collect();
        if segments.len() > MAX_PERMALINK_DEPTH {
            return Err(MemoryError::InvalidPermalink(trimmed.to_owned()));
        }
        let valid = segments.iter().all(|segment| {
            !segment.is_empty()
                && !segment.starts_with('-')
                && !segment.ends_with('-')
                && segment
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        });
        if !valid {
            return Err(MemoryError::InvalidPermalink(trimmed.to_owned()));
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The slug itself.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The file this note is stored in, relative to the memory directory.
    ///
    /// Always forward-slashed and always relative. A permalink with
    /// folder segments yields nested directories, which the store must
    /// create before writing.
    ///
    /// ```
    /// use tirith::memory::Permalink;
    ///
    /// let link = Permalink::from_title("Lease renewal").unwrap();
    /// assert_eq!(link.file_path(), "lease-renewal.md");
    /// ```
    pub fn file_path(&self) -> String {
        format!("{}.md", self.0)
    }
}

impl fmt::Display for Permalink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One categorized fact inside a note body.
///
/// Written as `- [category] content #tag`, the same line format Basic
/// Memory uses, so notes move between the two without translation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    /// The bracketed category, for example `design` or `lesson`.
    pub category: String,
    /// The text after the category, tags included.
    pub content: String,
    /// Hashtags found in the content, without the `#`.
    pub tags: Vec<String>,
}

/// A typed link from one note to another.
///
/// Written as `- relation_kind [[Target permalink or title]]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relation {
    /// The relation kind, for example `follows` or `supersedes`.
    pub kind: String,
    /// What it points at: a permalink or a title.
    pub target: String,
}

/// A single memory note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MemoryNote {
    /// Unique identifier, stable across edits.
    pub id: MemoryId,
    /// Stable address and filename stem.
    pub permalink: Permalink,
    /// Human title. May be edited; the permalink does not follow it.
    pub title: String,
    /// What the note is for.
    pub kind: MemoryKind,
    /// The Markdown body, everything after the frontmatter.
    pub body: String,
    /// Categorized facts parsed out of the body.
    pub observations: Vec<Observation>,
    /// Links to other notes parsed out of the body.
    pub relations: Vec<Relation>,
    /// Repository paths this note is about.
    pub paths: Vec<RepoPath>,
    /// Free tags for filtering.
    pub tags: Vec<String>,
    /// Who wrote it first.
    pub author: AgentId,
    /// Who wrote the current revision.
    pub updated_by: AgentId,
    /// When it was first written.
    pub created_at: DateTime<Utc>,
    /// When it was last written.
    pub updated_at: DateTime<Utc>,
}

impl MemoryNote {
    /// Whether `path` overlaps any path this note is scoped to.
    pub fn concerns(&self, path: &RepoPath) -> bool {
        self.paths.iter().any(|p| p.overlaps(path))
    }

    /// Whether the note carries `tag`, case-insensitively.
    pub fn has_tag(&self, tag: &str) -> bool {
        let needle = tag.trim().trim_start_matches('#').to_lowercase();
        self.tags.iter().any(|t| t.to_lowercase() == needle)
            || self
                .observations
                .iter()
                .any(|o| o.tags.iter().any(|t| t.to_lowercase() == needle))
    }

    /// This note without its body, for a listing.
    pub fn digest(&self) -> MemoryDigest {
        MemoryDigest {
            permalink: self.permalink.clone(),
            title: self.title.clone(),
            kind: self.kind,
            paths: self.paths.clone(),
            tags: self.tags.clone(),
            updated_at: self.updated_at,
            excerpt: self.excerpt(CLAIM_MEMORY_EXCERPT),
        }
    }

    /// A short plain-text excerpt of the body, for list results.
    pub fn excerpt(&self, max_chars: usize) -> String {
        let flat = self
            .body
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join(" ");
        if flat.chars().count() <= max_chars {
            return flat;
        }
        let cut: String = flat.chars().take(max_chars).collect();
        let cut = cut.trim_end().to_owned();
        format!("{cut}...")
    }

    /// Renders the note as the Markdown file that is written to disk.
    ///
    /// The inverse of [`MemoryNote::from_markdown`].
    pub fn to_markdown(&self) -> String {
        let mut out = String::from("---\n");
        push_scalar(&mut out, "id", &self.id.to_string());
        push_scalar(&mut out, "permalink", self.permalink.as_str());
        push_scalar(&mut out, "title", &self.title);
        push_scalar(&mut out, "kind", self.kind.as_str());
        push_list(&mut out, "tags", self.tags.iter().map(String::as_str));
        push_list(&mut out, "paths", self.paths.iter().map(RepoPath::as_str));
        push_scalar(&mut out, "author", self.author.as_str());
        push_scalar(&mut out, "updated_by", self.updated_by.as_str());
        push_scalar(&mut out, "created_at", &rfc3339(self.created_at));
        push_scalar(&mut out, "updated_at", &rfc3339(self.updated_at));
        out.push_str("---\n\n");
        out.push_str(self.body.trim_end());
        out.push('\n');
        out
    }

    /// Parses a note from the Markdown file written by
    /// [`MemoryNote::to_markdown`].
    ///
    /// Hand-edited files are accepted as long as the frontmatter carries
    /// a title; missing optional fields fall back to defaults so a human
    /// can drop a Markdown file into the directory and have it indexed. A
    /// missing `created_at` becomes `written_at`, which the caller takes
    /// from the file (its modification time) or its clock; this module
    /// never reads the system clock itself.
    pub fn from_markdown(text: &str, written_at: DateTime<Utc>) -> Result<Self, MemoryError> {
        let (front, body) = split_frontmatter(text)?;
        let fields = parse_frontmatter(&front)?;

        let title = fields
            .get("title")
            .map(String::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or(MemoryError::MissingField("title"))?
            .to_owned();

        let permalink = match fields.get("permalink") {
            Some(raw) => Permalink::parse(raw)?,
            None => Permalink::from_title(&title)?,
        };
        let id = match fields.get("id") {
            Some(raw) => MemoryId::parse(raw).map_err(|_| MemoryError::BadId(raw.clone()))?,
            None => MemoryId::new(),
        };
        let kind = match fields.get("kind") {
            Some(raw) => raw.parse()?,
            None => MemoryKind::default(),
        };
        let author = match fields.get("author") {
            Some(raw) => AgentId::new(raw).map_err(|_| MemoryError::BadAgent(raw.clone()))?,
            None => AgentId::new("unknown").map_err(|_| MemoryError::BadAgent("unknown".into()))?,
        };
        let updated_by = match fields.get("updated_by") {
            Some(raw) => AgentId::new(raw).map_err(|_| MemoryError::BadAgent(raw.clone()))?,
            None => author.clone(),
        };
        let created_at = match fields.get("created_at") {
            Some(raw) => parse_time(raw)?,
            None => written_at.trunc_subsecs(0),
        };
        let updated_at = match fields.get("updated_at") {
            Some(raw) => parse_time(raw)?,
            None => created_at,
        };

        let tags = normalize_tags(fields.list("tags"))?;
        let mut paths = Vec::new();
        for raw in fields.list("paths") {
            paths.push(RepoPath::new(&raw)?);
        }

        let body = body.trim().to_owned();
        Ok(Self {
            id,
            permalink,
            title,
            kind,
            observations: parse_observations(&body),
            relations: parse_relations(&body),
            body,
            paths,
            tags,
            author,
            updated_by,
            created_at,
            updated_at,
        })
    }
}

/// A note without its body, for listings.
///
/// Every place that returns more than one note returns these: search
/// results, the related notes of a read, and the notes a claim hands
/// back. Bodies are Markdown prose and are often kilobytes, so sending
/// twenty of them to answer "what is here?" spends the caller's context
/// on text it did not ask for. Only a read of one note returns a body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MemoryDigest {
    /// The note's stable address.
    pub permalink: Permalink,
    /// Its title.
    pub title: String,
    /// What it is for.
    pub kind: MemoryKind,
    /// The repository paths it is about.
    pub paths: Vec<RepoPath>,
    /// Its tags.
    pub tags: Vec<String>,
    /// When it was last written.
    pub updated_at: DateTime<Utc>,
    /// The first [`CLAIM_MEMORY_EXCERPT`] characters of the body, with
    /// headings and blank lines removed.
    pub excerpt: String,
}

/// What to write, for [`MemoryBook::write`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewMemory {
    /// See [`MemoryNote::title`].
    pub title: String,
    /// See [`MemoryNote::kind`].
    pub kind: MemoryKind,
    /// See [`MemoryNote::body`].
    pub body: String,
    /// See [`MemoryNote::paths`].
    pub paths: Vec<RepoPath>,
    /// See [`MemoryNote::tags`].
    pub tags: Vec<String>,
    /// The note to overwrite. When absent, the title decides: an existing
    /// note with the same permalink is updated, otherwise one is created.
    pub permalink: Option<Permalink>,
    /// Refuse the write unless the stored note was last updated at this
    /// instant.
    ///
    /// Two agents writing the same note otherwise silently lose one of
    /// the two bodies. Read the note, pass back its `updated_at`, and a
    /// write that would clobber someone else's work is refused with
    /// [`MemoryError::Conflict`] instead.
    pub if_updated_at: Option<DateTime<Utc>>,
}

impl NewMemory {
    /// A note with the required fields; everything else is set with the
    /// `with_*` methods, so a new optional field never breaks a caller.
    ///
    /// ```
    /// use tirith::memory::{MemoryKind, NewMemory};
    /// use tirith::types::RepoPath;
    ///
    /// let note = NewMemory::new("Leases renew on any call", "Any tool call renews every lease.")
    ///     .with_kind(MemoryKind::Fact)
    ///     .with_paths(vec![RepoPath::new("src/claims.rs").unwrap()])
    ///     .with_tags(vec!["leases".into()]);
    /// assert_eq!(note.kind, MemoryKind::Fact);
    /// assert_eq!(note.paths.len(), 1);
    /// ```
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            ..Self::default()
        }
    }

    /// Sets the kind; the default is [`MemoryKind::Note`].
    #[must_use]
    pub fn with_kind(mut self, kind: MemoryKind) -> Self {
        self.kind = kind;
        self
    }

    /// Sets the paths the note is about.
    #[must_use]
    pub fn with_paths(mut self, paths: Vec<RepoPath>) -> Self {
        self.paths = paths;
        self
    }

    /// Sets the tags.
    #[must_use]
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Targets an existing note instead of matching on the title.
    #[must_use]
    pub fn with_permalink(mut self, permalink: Permalink) -> Self {
        self.permalink = Some(permalink);
        self
    }

    /// Refuses the write unless the stored note was last updated at
    /// `at`; see [`NewMemory::if_updated_at`].
    #[must_use]
    pub fn with_if_updated_at(mut self, at: DateTime<Utc>) -> Self {
        self.if_updated_at = Some(at);
        self
    }
}

/// Filters for [`MemoryBook::search`].
///
/// [`MemorySearch::default`] is bounded at [`DEFAULT_SEARCH_LIMIT`]. An
/// unbounded search has to be asked for explicitly with `limit: None`,
/// because scoring runs under the daemon's single lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySearch {
    /// Free text; see [`MemoryBook::search`] for how it is scored.
    pub query: Option<String>,
    /// Only notes scoped to a path overlapping this one.
    pub path: Option<RepoPath>,
    /// Only notes of this kind.
    pub kind: Option<MemoryKind>,
    /// Only notes carrying this tag.
    pub tag: Option<String>,
    /// Only notes written at or after this time.
    pub since: Option<DateTime<Utc>>,
    /// How many to return. `None` means every match.
    pub limit: Option<usize>,
}

impl Default for MemorySearch {
    fn default() -> Self {
        Self {
            query: None,
            path: None,
            kind: None,
            tag: None,
            since: None,
            limit: Some(DEFAULT_SEARCH_LIMIT),
        }
    }
}

/// One search result and the score that ranked it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryHit<'a> {
    /// The matching note.
    pub note: &'a MemoryNote,
    /// Its relevance score; zero when the search had no query.
    pub score: u32,
}

/// The outcome of a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryWritten {
    /// The stored note.
    pub note: MemoryNote,
    /// True when the note did not exist before.
    pub created: bool,
}

/// Why a memory operation was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MemoryError {
    /// The title was empty.
    #[error("memory title must not be empty")]
    EmptyTitle,
    /// The title was longer than [`MAX_TITLE_LEN`].
    #[error("memory title must be at most {MAX_TITLE_LEN} characters")]
    TitleTooLong,
    /// The body was empty.
    #[error("memory body must not be empty")]
    EmptyBody,
    /// A title produced no usable slug.
    #[error("memory permalink must not be empty")]
    EmptyPermalink,
    /// The permalink was longer than [`MAX_PERMALINK_LEN`].
    #[error("permalink `{0}` is longer than {MAX_PERMALINK_LEN} characters")]
    PermalinkTooLong(String),
    /// The permalink was not a lowercase slug.
    #[error("permalink `{0}` must be lowercase letters, digits, and dashes")]
    InvalidPermalink(String),
    /// A tag held a character that cannot be written to frontmatter.
    #[error("tag `{0}` must not contain control characters, `:`, `[`, or `]`")]
    InvalidTag(String),
    /// A write targeted a permalink that does not exist.
    #[error("no memory note with permalink `{0}`")]
    NotFound(String),
    /// The note changed since the caller last read it.
    #[error("memory note `{permalink}` was updated at {updated_at}; re-read it before writing")]
    Conflict {
        /// The note that changed.
        permalink: String,
        /// When it actually changed, for the caller to pass back.
        updated_at: DateTime<Utc>,
    },
    /// The `kind` field was not one of the known kinds.
    #[error("unknown memory kind `{0}`")]
    UnknownKind(String),
    /// The frontmatter block was missing or unterminated.
    #[error("memory file has no `---` frontmatter block")]
    MissingFrontmatter,
    /// A frontmatter line was not understood.
    #[error("frontmatter line {line}: {message}")]
    Frontmatter {
        /// The one-based line number inside the frontmatter block.
        line: usize,
        /// What was wrong.
        message: String,
    },
    /// A required frontmatter field was absent.
    #[error("frontmatter is missing required field `{0}`")]
    MissingField(&'static str),
    /// The `id` field was not a UUID.
    #[error("frontmatter id `{0}` is not a valid identifier")]
    BadId(String),
    /// An author field was not a usable agent name.
    #[error("frontmatter agent `{0}` is not a valid agent name")]
    BadAgent(String),
    /// A timestamp field was not RFC 3339.
    #[error("frontmatter timestamp `{0}` is not RFC 3339")]
    BadTimestamp(String),
    /// A path field was not a usable repository path.
    #[error(transparent)]
    Path(#[from] PathError),
}

/// One note's searchable text, lowercased once instead of per query.
///
/// Kept beside the note in [`MemoryBook`] and rebuilt whenever that note
/// is written. Scoring a book of thousands of notes otherwise spends most
/// of its time allocating lowercase copies of bodies it has already seen.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Haystacks {
    title: String,
    permalink: String,
    tags: String,
    observations: String,
    body: String,
}

impl Haystacks {
    fn of(note: &MemoryNote) -> Self {
        Self {
            title: note.title.to_lowercase(),
            permalink: note.permalink.as_str().to_lowercase(),
            tags: note.tags.join(" ").to_lowercase(),
            observations: note
                .observations
                .iter()
                .map(|o| format!("{} {}", o.category, o.content))
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase(),
            body: note.body.to_lowercase(),
        }
    }
}

/// How much one field can contribute for a single term.
///
/// Without a cap, a long note that happens to repeat a common word beats
/// a short note that is actually about it: "the store" would rank every
/// note containing "the" forty times above the note titled "Store".
const MAX_HITS_PER_FIELD: u32 = 3;

/// Scores one note's text against already-tokenized query terms.
///
/// Weights are title, then permalink, then tags, then observations, then
/// body. A note matching every term outranks one matching only some.
///
/// Matching is on substrings, deliberately, so `lease` finds `leases` and
/// `renew` finds `renewal`. Repetition is capped rather than counted, so
/// length does not win on its own.
fn score_with(haystacks: &Haystacks, terms: &[String]) -> u32 {
    if terms.is_empty() {
        return 0;
    }
    let mut score = 0_u32;
    let mut matched = 0_usize;
    for term in terms {
        let hits = |text: &str| count_occurrences(text, term).min(MAX_HITS_PER_FIELD);
        let mut term_score = 0_u32;
        term_score += 6 * hits(&haystacks.title);
        term_score += 5 * hits(&haystacks.permalink);
        term_score += 4 * hits(&haystacks.tags);
        term_score += 2 * hits(&haystacks.observations);
        term_score += hits(&haystacks.body);
        if term_score > 0 {
            matched += 1;
        }
        score += term_score;
    }
    if matched == 0 {
        return 0;
    }
    if matched == terms.len() {
        score += 10;
    }
    score
}

impl MemoryError {
    /// The one-based frontmatter line this error is about, when it has one.
    ///
    /// The store reports it so a human can open the file at the right
    /// place instead of rereading the whole thing.
    pub fn line(&self) -> Option<usize> {
        match self {
            Self::Frontmatter { line, .. } => Some(*line),
            _ => None,
        }
    }
}

/// Every note, indexed by permalink and by id.
///
/// The book owns three things derived from `notes` and kept in step with
/// it: the lowercased search text, a permalink index, and an id index.
/// Looking a note up is a hash lookup rather than a scan, which matters
/// because every call runs under the daemon's single state lock.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MemoryBook {
    notes: Vec<MemoryNote>,
    haystacks: Vec<Haystacks>,
    by_permalink: HashMap<Permalink, usize>,
    by_id: HashMap<MemoryId, usize>,
    /// Lowercased relation target to the notes pointing at it.
    ///
    /// Relations are written one way in the file, but a reader wants both
    /// directions: what this note follows, and what follows this note.
    /// Without an index, answering the second means scanning every note's
    /// relations for every note on the frontier, which is quadratic and
    /// runs under the daemon's single lock.
    backlinks: HashMap<String, Vec<usize>>,
}

impl MemoryBook {
    /// Rebuilds a book from notes loaded off disk, newest last.
    ///
    /// When two notes claim the same permalink, which a hand-edited
    /// directory can produce, the one created later wins and the other is
    /// dropped, so the index and the notes never disagree.
    pub fn from_notes(mut notes: Vec<MemoryNote>) -> Self {
        notes.sort_by_key(|n| (n.created_at, n.id));
        let mut book = Self::default();
        for note in notes {
            if let Some(&existing) = book.by_permalink.get(&note.permalink) {
                book.haystacks[existing] = Haystacks::of(&note);
                book.by_id.remove(&book.notes[existing].id);
                book.by_id.insert(note.id, existing);
                book.notes[existing] = note;
                continue;
            }
            book.push(note);
        }
        book
    }

    /// Appends a note and indexes it.
    fn push(&mut self, note: MemoryNote) {
        let index = self.notes.len();
        self.by_permalink.insert(note.permalink.clone(), index);
        self.by_id.insert(note.id, index);
        self.haystacks.push(Haystacks::of(&note));
        for relation in &note.relations {
            self.backlinks
                .entry(relation.target.trim().to_lowercase())
                .or_default()
                .push(index);
        }
        self.notes.push(note);
    }

    /// Rebuilds the reverse-relation index from the notes.
    ///
    /// Called whenever a note's relations may have changed, which is any
    /// write. Writes are rare next to reads, and this is linear in the
    /// number of relations rather than quadratic in the number of notes.
    fn rebuild_backlinks(&mut self) {
        self.backlinks.clear();
        for (index, note) in self.notes.iter().enumerate() {
            for relation in &note.relations {
                self.backlinks
                    .entry(relation.target.trim().to_lowercase())
                    .or_default()
                    .push(index);
            }
        }
    }

    /// Rebuilds every index from the notes.
    ///
    /// Only removal needs this: taking a note out shifts every later
    /// position, so the maps cannot be patched in place.
    fn reindex(&mut self) {
        self.by_permalink.clear();
        self.by_id.clear();
        self.haystacks.clear();
        for (index, note) in self.notes.iter().enumerate() {
            self.by_permalink.insert(note.permalink.clone(), index);
            self.by_id.insert(note.id, index);
            self.haystacks.push(Haystacks::of(note));
        }
        self.rebuild_backlinks();
    }

    /// Removes a note by permalink, id, or exact title.
    ///
    /// Returns the note that was removed so the caller can delete its
    /// file. A note holding a secret or a plainly wrong fact has to be
    /// retractable; without this the only way out is editing the
    /// directory by hand, and the daemon writes it back on the next full
    /// rewrite.
    pub fn remove(&mut self, name_or_id: &str) -> Option<MemoryNote> {
        let permalink = self.get(name_or_id)?.permalink.clone();
        let index = *self.by_permalink.get(&permalink)?;
        let removed = self.notes.remove(index);
        self.reindex();
        Some(removed)
    }

    /// Every note, oldest first.
    pub fn notes(&self) -> &[MemoryNote] {
        &self.notes
    }

    /// How many notes are stored.
    pub fn len(&self) -> usize {
        self.notes.len()
    }

    /// Whether the book holds no notes.
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Writes a note, creating it or replacing an existing one.
    ///
    /// The permalink decides which: an explicit `permalink` targets that
    /// note and fails if it is gone, otherwise the title's slug is used
    /// and a matching note is updated in place.
    ///
    /// `now` is truncated to whole seconds, which is the precision the
    /// frontmatter stores. That keeps the note held in memory byte-identical
    /// to the note on disk, so the store can compare or reload either one.
    pub fn write(
        &mut self,
        by: AgentId,
        new: NewMemory,
        now: DateTime<Utc>,
    ) -> Result<MemoryWritten, MemoryError> {
        let now = now.trunc_subsecs(0);
        let title = new.title.trim().to_owned();
        if title.is_empty() {
            return Err(MemoryError::EmptyTitle);
        }
        if title.chars().count() > MAX_TITLE_LEN {
            return Err(MemoryError::TitleTooLong);
        }
        let body = new.body.trim().to_owned();
        if body.is_empty() {
            return Err(MemoryError::EmptyBody);
        }

        let id = MemoryId::new();
        let (permalink, existing) = if let Some(link) = new.permalink {
            let found = self.position(&link);
            if found.is_none() {
                return Err(MemoryError::NotFound(link.to_string()));
            }
            (link, found)
        } else {
            let link = Permalink::for_title(&title, id);
            match self.position(&link) {
                // Same slug, same title: the caller means this note.
                Some(index) if self.notes[index].title == title => (link, Some(index)),
                // Same slug, different title: two titles that slugify alike.
                // Overwriting one with the other would lose a note, so the
                // newcomer gets its own address.
                Some(_) => (link.disambiguated(id), None),
                None => (link, None),
            }
        };

        let tags = normalize_tags(new.tags)?;
        let observations = parse_observations(&body);
        let relations = parse_relations(&body);

        if let Some(index) = existing {
            let note = &mut self.notes[index];
            if let Some(expected) = new.if_updated_at
                && expected.trunc_subsecs(0) != note.updated_at
            {
                return Err(MemoryError::Conflict {
                    permalink: note.permalink.to_string(),
                    updated_at: note.updated_at,
                });
            }
            note.title = title;
            note.kind = new.kind;
            note.body = body;
            note.observations = observations;
            note.relations = relations;
            note.paths = new.paths;
            note.tags = tags;
            note.updated_by = by;
            note.updated_at = now;
            let updated = note.clone();
            self.haystacks[index] = Haystacks::of(&updated);
            self.rebuild_backlinks();
            return Ok(MemoryWritten {
                note: updated,
                created: false,
            });
        }

        let note = MemoryNote {
            id,
            permalink,
            title,
            kind: new.kind,
            body,
            observations,
            relations,
            paths: new.paths,
            tags,
            author: by.clone(),
            updated_by: by,
            created_at: now,
            updated_at: now,
        };
        self.push(note.clone());
        Ok(MemoryWritten {
            note,
            created: true,
        })
    }

    /// Finds a note by permalink, id, or exact title.
    ///
    /// Permalink and id are hash lookups. Only the title fallback scans,
    /// and only when the first two miss.
    pub fn get(&self, name_or_id: &str) -> Option<&MemoryNote> {
        let needle = name_or_id.trim();
        if let Ok(link) = Permalink::parse(needle)
            && let Some(&index) = self.by_permalink.get(&link)
        {
            return self.notes.get(index);
        }
        if let Ok(id) = MemoryId::parse(needle)
            && let Some(&index) = self.by_id.get(&id)
        {
            return self.notes.get(index);
        }
        let lowered = needle.to_lowercase();
        self.notes
            .iter()
            .find(|n| n.title.to_lowercase() == lowered)
    }

    /// Notes matching every filter, best first.
    ///
    /// With a query, notes are scored on title, permalink, tags,
    /// observations, and body, in that order of weight; a note matching
    /// every term outranks one matching some, and non-matching notes are
    /// dropped. Matching is on substrings, so `lease` finds `leases`.
    /// Without a query, results are the filtered notes, newest first.
    pub fn search(&self, filter: &MemorySearch) -> Vec<MemoryHit<'_>> {
        let query = filter
            .query
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty());
        let terms = query.map(tokenize).unwrap_or_default();
        let mut hits: Vec<MemoryHit<'_>> = self
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| filter.path.as_ref().is_none_or(|p| n.concerns(p)))
            .filter(|(_, n)| filter.kind.is_none_or(|k| n.kind == k))
            .filter(|(_, n)| filter.tag.as_deref().is_none_or(|t| n.has_tag(t)))
            .filter(|(_, n)| filter.since.is_none_or(|s| n.updated_at >= s))
            .filter_map(|(index, note)| {
                if terms.is_empty() {
                    return Some(MemoryHit { note, score: 0 });
                }
                let score = score_with(&self.haystacks[index], &terms);
                (score > 0).then_some(MemoryHit { note, score })
            })
            .collect();

        if query.is_some() {
            hits.sort_by_key(|h| std::cmp::Reverse((h.score, h.note.updated_at)));
        } else {
            hits.sort_by_key(|h| std::cmp::Reverse(h.note.updated_at));
        }
        if let Some(limit) = filter.limit {
            hits.truncate(limit);
        }
        hits
    }

    /// Notes scoped to a path, newest first.
    ///
    /// This is what an agent is handed when it claims that path.
    pub fn for_path(&self, path: &RepoPath, limit: Option<usize>) -> Vec<&MemoryNote> {
        let mut found: Vec<&MemoryNote> = self.notes.iter().filter(|n| n.concerns(path)).collect();
        found.sort_by_key(|n| std::cmp::Reverse(n.updated_at));
        if let Some(limit) = limit {
            found.truncate(limit);
        }
        found
    }

    /// A note plus everything reachable from it by relations, up to
    /// `depth` hops.
    ///
    /// Links resolve by permalink first and by title second, so a
    /// hand-written `[[Some Title]]` finds its note. The starting note is
    /// always first; unresolvable links are ignored.
    pub fn context(&self, start: &str, depth: usize) -> Vec<&MemoryNote> {
        let Some(root) = self.get(start) else {
            return Vec::new();
        };
        let mut seen: HashSet<&Permalink> = HashSet::new();
        seen.insert(&root.permalink);
        let mut out = vec![root];
        let mut frontier = vec![root];
        for _ in 0..depth {
            let mut next = Vec::new();
            for note in &frontier {
                for link in note.relations.iter().map(|r| r.target.as_str()) {
                    if let Some(found) = self.get(link)
                        && seen.insert(&found.permalink)
                    {
                        out.push(found);
                        next.push(found);
                    }
                }
                // Backlinks matter as much as forward links, and come
                // from the index rather than a scan over every note.
                let keys = [
                    note.permalink.as_str().to_lowercase(),
                    note.title.trim().to_lowercase(),
                ];
                for index in keys.iter().filter_map(|k| self.backlinks.get(k)).flatten() {
                    if let Some(other) = self.notes.get(*index)
                        && seen.insert(&other.permalink)
                    {
                        out.push(other);
                        next.push(other);
                    }
                }
            }
            if next.is_empty() || out.len() > MAX_RELATED {
                break;
            }
            frontier = next;
        }
        // The starting note is always first and is not itself "related".
        out.truncate(MAX_RELATED + 1);
        out
    }

    fn position(&self, link: &Permalink) -> Option<usize> {
        self.by_permalink.get(link).copied()
    }
}

/// Lowercases, strips a leading `#`, deduplicates, and refuses anything
/// that could not be read back out of frontmatter.
fn normalize_tags(tags: Vec<String>) -> Result<Vec<String>, MemoryError> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in tags {
        let tag = raw.trim().trim_start_matches('#').to_lowercase();
        if tag.is_empty() {
            continue;
        }
        // A newline would end the frontmatter line; the others are YAML
        // structure. Quoting would hide the first but not fix the value.
        if tag
            .chars()
            .any(|c| c.is_control() || matches!(c, ':' | '[' | ']'))
        {
            return Err(MemoryError::InvalidTag(raw));
        }
        if seen.insert(tag.clone()) {
            out.push(tag);
        }
    }
    Ok(out)
}

/// Query terms shorter than this are dropped when longer ones exist.
const MIN_USEFUL_TERM: usize = 3;

/// Words that appear in nearly every note and rank nothing.
///
/// Without this, searching "the store" scores a long note repeating "the"
/// above the note actually titled "Store", because the common word both
/// contributes and earns the all-terms bonus.
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "any", "are", "as", "at", "be", "but", "by", "can", "do", "for", "from",
    "has", "have", "how", "if", "in", "into", "is", "it", "its", "no", "not", "of", "on", "or",
    "so", "than", "that", "the", "their", "them", "then", "there", "these", "they", "this", "to",
    "was", "we", "what", "when", "which", "will", "with", "you",
];

/// Splits text into lowercase terms.
///
/// Terms under [`MIN_USEFUL_TERM`] characters, and terms in
/// [`STOP_WORDS`], are dropped: they match everything and rank nothing.
/// They are kept when the whole query is made of them, so searching for
/// `ci` or `the` still works.
fn tokenize(text: &str) -> Vec<String> {
    let all: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect();
    let useful: Vec<String> = all
        .iter()
        .filter(|t| t.chars().count() >= MIN_USEFUL_TERM)
        .filter(|t| !STOP_WORDS.contains(&t.as_str()))
        .cloned()
        .collect();
    if useful.is_empty() { all } else { useful }
}

fn count_occurrences(haystack: &str, needle: &str) -> u32 {
    if needle.is_empty() {
        return 0;
    }
    u32::try_from(haystack.matches(needle).count()).unwrap_or(u32::MAX)
}

fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_time(raw: &str) -> Result<DateTime<Utc>, MemoryError> {
    DateTime::parse_from_rfc3339(raw.trim())
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| MemoryError::BadTimestamp(raw.to_owned()))
}

/// Parses `- [category] content #tag` lines out of a body.
fn parse_observations(body: &str) -> Vec<Observation> {
    let mut out = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("- ") else {
            continue;
        };
        let rest = rest.trim();
        if !rest.starts_with('[') || rest.starts_with("[[") {
            continue;
        }
        let Some(end) = rest.find(']') else {
            continue;
        };
        let category = rest[1..end].trim().to_owned();
        if category.is_empty() {
            continue;
        }
        let content = rest[end + 1..].trim().to_owned();
        if content.is_empty() {
            continue;
        }
        let tags = content
            .split_whitespace()
            .filter_map(|w| w.strip_prefix('#'))
            .map(|t| t.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-'))
            .filter(|t| !t.is_empty())
            .map(str::to_lowercase)
            .collect();
        out.push(Observation {
            category,
            content,
            tags,
        });
    }
    out
}

/// Parses `- relation_kind [[Target]]` lines out of a body.
fn parse_relations(body: &str) -> Vec<Relation> {
    let mut out = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("- ") else {
            continue;
        };
        let Some(open) = rest.find("[[") else {
            continue;
        };
        let Some(close) = rest[open..].find("]]") else {
            continue;
        };
        let target = rest[open + 2..open + close].trim().to_owned();
        if target.is_empty() {
            continue;
        }
        let kind = rest[..open].trim();
        let kind = if kind.is_empty() {
            "relates_to".to_owned()
        } else {
            kind.to_owned()
        };
        out.push(Relation { kind, target });
    }
    out
}

/// Splits `---` frontmatter from the body.
fn split_frontmatter(text: &str) -> Result<(String, String), MemoryError> {
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    let rest = trimmed
        .strip_prefix("---")
        .ok_or(MemoryError::MissingFrontmatter)?;
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let mut front = String::new();
    let mut lines = rest.lines();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        front.push_str(line);
        front.push('\n');
    }
    if !closed {
        return Err(MemoryError::MissingFrontmatter);
    }
    let body = lines.collect::<Vec<_>>().join("\n");
    Ok((front, body))
}

/// The frontmatter fields of one note.
///
/// Values are either a scalar or a list; the parser accepts the small
/// YAML subset this crate writes, plus inline `[a, b]` lists, which is
/// what Basic Memory produces.
#[derive(Debug, Default)]
struct Frontmatter {
    scalars: HashMap<String, String>,
    lists: HashMap<String, Vec<String>>,
}

impl Frontmatter {
    fn get(&self, key: &str) -> Option<&String> {
        self.scalars.get(key)
    }

    fn list(&self, key: &str) -> Vec<String> {
        if let Some(items) = self.lists.get(key) {
            return items.clone();
        }
        // A scalar in a list position is a one-element list.
        match self.scalars.get(key) {
            Some(value) if !value.trim().is_empty() => vec![value.clone()],
            _ => Vec::new(),
        }
    }
}

fn parse_frontmatter(front: &str) -> Result<Frontmatter, MemoryError> {
    let mut out = Frontmatter::default();
    let mut current_list: Option<String> = None;
    for (index, raw) in front.lines().enumerate() {
        let line = raw.trim_end();
        let number = index + 1;
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if let Some(item) = line.trim_start().strip_prefix("- ") {
            let key = current_list
                .clone()
                .ok_or_else(|| MemoryError::Frontmatter {
                    line: number,
                    message: "list item before any key".to_owned(),
                })?;
            out.lists.entry(key).or_default().push(unquote(item.trim()));
            continue;
        }
        let Some(colon) = line.find(':') else {
            return Err(MemoryError::Frontmatter {
                line: number,
                message: format!("expected `key: value`, found `{}`", line.trim()),
            });
        };
        let key = line[..colon].trim().to_owned();
        if key.is_empty() {
            return Err(MemoryError::Frontmatter {
                line: number,
                message: "empty key".to_owned(),
            });
        }
        let value = line[colon + 1..].trim();
        if value.is_empty() {
            current_list = Some(key.clone());
            out.lists.entry(key).or_default();
            continue;
        }
        current_list = None;
        if let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            let items = inner
                .split(',')
                .map(|i| unquote(i.trim()))
                .filter(|i| !i.is_empty())
                .collect();
            out.lists.insert(key, items);
            continue;
        }
        out.scalars.insert(key, unquote(value));
    }
    Ok(out)
}

/// Reverses [`escape`] for a quoted frontmatter value.
///
/// A double-quoted value has its escapes undone; a single-quoted one is
/// taken literally, as YAML does. Anything unquoted is returned as is.
fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => out.push('\\'),
                }
            } else {
                out.push(ch);
            }
        }
        return out;
    }
    if trimmed.len() >= 2 && trimmed.starts_with('\'') && trimmed.ends_with('\'') {
        return trimmed[1..trimmed.len() - 1].to_owned();
    }
    trimmed.to_owned()
}

/// Whether a frontmatter value has to be quoted to survive a round trip.
///
/// Everything here either starts a YAML construct or ends the line.
fn needs_quotes(value: &str) -> bool {
    value.is_empty()
        || value.starts_with(' ')
        || value.ends_with(' ')
        || value.starts_with([
            '[', '{', '#', '&', '*', '-', '|', '>', '!', '?', '%', '@', '`', ',',
        ])
        || value.contains(": ")
        || value.ends_with(':')
        || value.contains('"')
        || value.chars().any(char::is_control)
}

fn push_scalar(out: &mut String, key: &str, value: &str) {
    let needs_quotes = needs_quotes(value);
    out.push_str(key);
    if needs_quotes {
        out.push_str(": \"");
        out.push_str(&escape(value));
        out.push_str("\"\n");
    } else {
        out.push_str(": ");
        out.push_str(value);
        out.push('\n');
    }
}

fn push_list<'a>(out: &mut String, key: &str, values: impl Iterator<Item = &'a str>) {
    let items: Vec<&str> = values.collect();
    out.push_str(key);
    if items.is_empty() {
        out.push_str(": []\n");
        return;
    }
    out.push_str(":\n");
    for item in items {
        out.push_str("- ");
        if needs_quotes(item) {
            out.push('"');
            out.push_str(&escape(item));
            out.push('"');
        } else {
            out.push_str(item);
        }
        out.push('\n');
    }
}

/// Escapes a value for a double-quoted frontmatter scalar.
fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\n', '\r', '\t'], " ")
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn agent(name: &str) -> AgentId {
        AgentId::new(name).unwrap()
    }

    fn path(raw: &str) -> RepoPath {
        RepoPath::new(raw).unwrap()
    }

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 16, 1, minute, 0).unwrap()
    }

    fn note(title: &str, body: &str) -> NewMemory {
        NewMemory::new(title, body)
    }

    /// Writes a plain note by `tester` at minute 0 and returns it.
    fn write(book: &mut MemoryBook, title: &str, body: &str) -> MemoryNote {
        book.write(agent("tester"), note(title, body), at(0))
            .unwrap()
            .note
    }

    fn book_with(entries: &[(&str, &str)]) -> MemoryBook {
        let mut book = MemoryBook::default();
        for (index, (title, body)) in entries.iter().enumerate() {
            book.write(
                agent("tester"),
                note(title, body),
                at(u32::try_from(index).unwrap()),
            )
            .unwrap();
        }
        book
    }

    #[test]
    fn permalinks_slugify_titles() {
        assert_eq!(
            Permalink::from_title("Persister writes are ordered!")
                .unwrap()
                .as_str(),
            "persister-writes-are-ordered"
        );
        assert_eq!(
            Permalink::from_title("  Lease   renewal / TTL  ")
                .unwrap()
                .as_str(),
            "lease-renewal-ttl"
        );
        // A slash in a title is punctuation, not a folder.
        assert_eq!(
            Permalink::from_title("A note / with a slash")
                .unwrap()
                .as_str(),
            "a-note-with-a-slash"
        );
        assert_eq!(
            Permalink::from_title("Lease renewal").unwrap().file_path(),
            "lease-renewal.md"
        );
        assert_eq!(
            Permalink::from_title("***").unwrap_err(),
            MemoryError::EmptyPermalink
        );
    }

    #[test]
    fn permalinks_may_carry_folder_segments() {
        let link = Permalink::parse("tirith/design/pre-alpha-build").unwrap();
        assert_eq!(link.file_path(), "tirith/design/pre-alpha-build.md");
        assert!(Permalink::parse("valid-slug-1").is_ok());
    }

    /// `.` is outside the allowed character set, so `..` cannot be
    /// spelled and a permalink can never escape the memory directory.
    #[test]
    fn permalinks_cannot_escape_the_directory() {
        for bad in [
            "../escape",
            "..",
            "/absolute",
            "trailing/",
            "double//slash",
            "has space",
            "has space/x",
            "Upper",
            "Upper/case",
            "trailing-",
            "-leading",
            "a/b/c/d/e/f/g/h/i",
        ] {
            assert!(Permalink::parse(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn markdown_round_trips() {
        let mut book = MemoryBook::default();
        let written = book
            .write(
                agent("storage-claude"),
                note(
                    "Persister: writes are sequence-ordered",
                    "Stale snapshots are skipped.\n\n- [design] Writes go through spawn_blocking #storage\n- follows [[pre-alpha-build]]",
                )
                .with_kind(MemoryKind::Lesson)
                .with_paths(vec![path("src/store.rs"), path("src/state.rs")])
                .with_tags(vec!["Storage".to_owned(), "#rmcp".to_owned()]),
                at(5),
            )
            .unwrap();

        let text = written.note.to_markdown();
        let parsed = MemoryNote::from_markdown(&text, at(0)).unwrap();
        assert_eq!(parsed, written.note);
        // The title carries a colon, which must survive quoting.
        assert_eq!(parsed.title, "Persister: writes are sequence-ordered");
        assert_eq!(parsed.kind, MemoryKind::Lesson);
        assert_eq!(parsed.tags, vec!["storage", "rmcp"]);
        assert_eq!(
            parsed.paths,
            vec![path("src/store.rs"), path("src/state.rs")]
        );
        assert_eq!(parsed.observations.len(), 1);
        assert_eq!(parsed.relations.len(), 1);
    }

    #[test]
    fn empty_lists_round_trip() {
        let mut book = MemoryBook::default();
        let bare = write(&mut book, "Bare note", "Body.");
        let parsed = MemoryNote::from_markdown(&bare.to_markdown(), at(0)).unwrap();
        assert!(parsed.tags.is_empty());
        assert!(parsed.paths.is_empty());
        assert_eq!(parsed, bare);
    }

    /// The store relies on this: whatever `write` returns is exactly what
    /// `from_markdown` gives back after `to_markdown`, sub-second
    /// timestamps included.
    #[test]
    fn a_note_written_now_round_trips_exactly() {
        let mut book = MemoryBook::default();
        let written = book
            .write(
                agent("a"),
                note("Written at an awkward instant", "Body."),
                // A timestamp with nanoseconds, as the system clock produces.
                at(0) + chrono::Duration::nanoseconds(123_456_789),
            )
            .unwrap()
            .note;
        assert_eq!(written.created_at.timestamp_subsec_nanos(), 0);
        let parsed = MemoryNote::from_markdown(&written.to_markdown(), at(0)).unwrap();
        assert_eq!(parsed, written);
    }

    /// Anything that survives `write` must survive the file round trip.
    #[test]
    fn awkward_titles_and_paths_round_trip_through_frontmatter() {
        let mut book = MemoryBook::default();
        for title in [
            "- leading dash",
            "? leading question",
            "% leading percent",
            "@ leading at",
            "| leading pipe",
            "> leading angle",
            "key: value pairs",
            "trailing colon:",
            "quotes \"inside\" it",
            "# leading hash",
            ", leading comma",
        ] {
            let written = book
                .write(
                    agent("a"),
                    note(title, "Body.")
                        .with_tags(vec!["plain".to_owned()])
                        .with_paths(vec![path("src/store.rs")]),
                    at(0),
                )
                .unwrap()
                .note;
            let parsed = MemoryNote::from_markdown(&written.to_markdown(), at(0))
                .unwrap_or_else(|e| panic!("{title:?} did not round trip: {e}"));
            assert_eq!(parsed, written, "{title:?} changed across a round trip");
        }
    }

    #[test]
    fn basic_memory_frontmatter_is_accepted() {
        // Inline lists and unknown extra keys, as Basic Memory writes them.
        let text = "---\ntitle: Project kickoff 2026-09-15\ntype: note\ntags: [tirith, kickoff]\npermalink: project-kickoff\n---\n\nTirith is an MCP coordination server.\n\n- [decision] Rust edition 2024 #architecture\n- documented_in [[Tirith docs]]\n";
        let parsed = MemoryNote::from_markdown(text, at(0)).unwrap();
        assert_eq!(parsed.title, "Project kickoff 2026-09-15");
        assert_eq!(parsed.permalink.as_str(), "project-kickoff");
        assert_eq!(parsed.tags, vec!["tirith", "kickoff"]);
        assert_eq!(parsed.kind, MemoryKind::Note);
        assert_eq!(parsed.observations[0].category, "decision");
        assert_eq!(parsed.observations[0].tags, vec!["architecture"]);
        assert_eq!(parsed.relations[0].kind, "documented_in");
        assert_eq!(parsed.relations[0].target, "Tirith docs");
    }

    #[test]
    fn a_hand_written_file_needs_only_a_title() {
        let parsed =
            MemoryNote::from_markdown("---\ntitle: Quick thought\n---\n\nSomething.\n", at(0))
                .unwrap();
        assert_eq!(parsed.permalink.as_str(), "quick-thought");
        assert_eq!(parsed.body, "Something.");
        assert_eq!(parsed.author.as_str(), "unknown");
        assert_eq!(parsed.updated_at, parsed.created_at);
    }

    #[test]
    fn malformed_files_are_errors_never_panics() {
        assert_eq!(
            MemoryNote::from_markdown("no frontmatter here", at(0)).unwrap_err(),
            MemoryError::MissingFrontmatter
        );
        assert_eq!(
            MemoryNote::from_markdown("---\ntitle: Unterminated\n", at(0)).unwrap_err(),
            MemoryError::MissingFrontmatter
        );
        assert_eq!(
            MemoryNote::from_markdown("---\nkind: lesson\n---\nbody", at(0)).unwrap_err(),
            MemoryError::MissingField("title")
        );
        assert_eq!(
            MemoryNote::from_markdown("---\ntitle: T\nkind: wat\n---\nbody", at(0)).unwrap_err(),
            MemoryError::UnknownKind("wat".to_owned())
        );
        assert_eq!(
            MemoryNote::from_markdown("---\ntitle: T\ncreated_at: yesterday\n---\nbody", at(0))
                .unwrap_err(),
            MemoryError::BadTimestamp("yesterday".to_owned())
        );
        // Frontmatter errors carry their one-based line so the store can
        // point a human at it.
        let err =
            MemoryNote::from_markdown("---\ntitle: T\nthis line has no colon\n---\nbody", at(0))
                .unwrap_err();
        assert!(
            matches!(err, MemoryError::Frontmatter { line: 2, .. }),
            "got {err:?}"
        );
        assert_eq!(err.line(), Some(2));
        assert_eq!(MemoryError::EmptyTitle.line(), None);
        let err =
            MemoryNote::from_markdown("---\n- orphan\ntitle: T\n---\nbody", at(0)).unwrap_err();
        assert!(
            matches!(err, MemoryError::Frontmatter { line: 1, .. }),
            "a list item before any key: {err:?}"
        );
    }

    #[test]
    fn writing_the_same_title_updates_in_place() {
        let mut book = MemoryBook::default();
        let first = book
            .write(agent("a"), note("Lease renewal", "First."), at(0))
            .unwrap();
        assert!(first.created);

        let second = book
            .write(
                agent("b"),
                note("Lease renewal", "Second.").with_kind(MemoryKind::Gotcha),
                at(1),
            )
            .unwrap();
        assert!(!second.created);
        assert_eq!(book.len(), 1);
        // Identity and creation time survive the edit; authorship splits.
        assert_eq!(second.note.id, first.note.id);
        assert_eq!(second.note.permalink, first.note.permalink);
        assert_eq!(second.note.created_at, at(0));
        assert_eq!(second.note.updated_at, at(1));
        assert_eq!(second.note.author.as_str(), "a");
        assert_eq!(second.note.updated_by.as_str(), "b");
        assert_eq!(second.note.body, "Second.");
        assert_eq!(second.note.kind, MemoryKind::Gotcha);
    }

    #[test]
    fn editing_by_permalink_keeps_the_address_when_the_title_changes() {
        let mut book = MemoryBook::default();
        let link = write(&mut book, "Old title", "Body.").permalink;
        let updated = book
            .write(
                agent("a"),
                note("A completely different title", "Body.").with_permalink(link.clone()),
                at(1),
            )
            .unwrap();
        assert!(!updated.created);
        assert_eq!(updated.note.permalink, link);
        assert_eq!(updated.note.title, "A completely different title");
        assert_eq!(book.len(), 1);
    }

    #[test]
    fn writes_are_validated() {
        let mut book = MemoryBook::default();
        assert_eq!(
            book.write(agent("a"), note("  ", "body"), at(0))
                .unwrap_err(),
            MemoryError::EmptyTitle
        );
        assert_eq!(
            book.write(
                agent("a"),
                note(&"x".repeat(MAX_TITLE_LEN + 1), "body"),
                at(0)
            )
            .unwrap_err(),
            MemoryError::TitleTooLong
        );
        assert_eq!(
            book.write(agent("a"), note("Title", "  \n "), at(0))
                .unwrap_err(),
            MemoryError::EmptyBody
        );
        assert_eq!(
            book.write(
                agent("a"),
                note("T", "b").with_permalink(Permalink::parse("does-not-exist").unwrap()),
                at(0),
            )
            .unwrap_err(),
            MemoryError::NotFound("does-not-exist".to_owned())
        );
        assert!(book.is_empty());
    }

    #[test]
    fn tags_that_cannot_be_written_back_are_refused() {
        let mut book = MemoryBook::default();
        for bad in ["has\nnewline", "has:colon", "has[bracket", "has]bracket"] {
            let result = book.write(
                agent("a"),
                note("Tagged", "Body.").with_tags(vec![bad.to_owned()]),
                at(0),
            );
            assert!(
                matches!(result, Err(MemoryError::InvalidTag(_))),
                "{bad} should be refused, got {result:?}"
            );
        }
        assert!(book.is_empty());
    }

    /// Two agents editing one note: the write that carries a stale
    /// `updated_at` is refused with the real one, so nothing is clobbered.
    #[test]
    fn a_stale_if_updated_at_is_refused_and_the_current_one_writes() {
        let mut book = MemoryBook::default();
        let mine = write(&mut book, "Shared note", "Mine.");
        book.write(agent("other"), note("Shared note", "Theirs."), at(1))
            .unwrap();

        let stale = book
            .write(
                agent("tester"),
                note("Shared note", "Merged?").with_if_updated_at(mine.updated_at),
                at(2),
            )
            .unwrap_err();
        assert_eq!(
            stale,
            MemoryError::Conflict {
                permalink: "shared-note".to_owned(),
                updated_at: at(1),
            }
        );
        assert_eq!(book.get("shared-note").unwrap().body, "Theirs.");

        let fresh = book
            .write(
                agent("tester"),
                note("Shared note", "Merged.").with_if_updated_at(at(1)),
                at(2),
            )
            .unwrap();
        assert!(!fresh.created);
        assert_eq!(fresh.note.body, "Merged.");
        assert_eq!(book.len(), 1);
    }

    /// A title in a script with no ASCII letters must still be storable.
    #[test]
    fn a_title_that_slugifies_to_nothing_still_gets_a_permalink() {
        let mut book = MemoryBook::default();
        let stored = write(&mut book, "Заметка о хранилище", "Body.");
        assert!(
            stored.permalink.as_str().starts_with("note-"),
            "{}",
            stored.permalink
        );
        // And it round-trips, so the loader can read it back.
        let parsed = MemoryNote::from_markdown(&stored.to_markdown(), at(0)).unwrap();
        assert_eq!(parsed, stored);
        assert_eq!(parsed.title, "Заметка о хранилище");
    }

    /// Two different titles that slugify the same must not overwrite.
    #[test]
    fn titles_that_slugify_alike_get_separate_notes() {
        let mut book = MemoryBook::default();
        let first = book
            .write(agent("a"), note("Storage design", "First note."), at(0))
            .unwrap();
        let second = book
            .write(agent("a"), note("Storage  design!", "Second note."), at(0))
            .unwrap();
        assert!(first.created);
        assert!(second.created, "the second title must not update the first");
        assert_eq!(book.len(), 2);
        assert_ne!(first.note.permalink, second.note.permalink);
        assert_eq!(
            book.get(first.note.permalink.as_str()).unwrap().body,
            "First note."
        );
        assert_eq!(
            book.get(second.note.permalink.as_str()).unwrap().body,
            "Second note."
        );
    }

    #[test]
    fn search_ranks_titles_above_bodies() {
        let book = book_with(&[
            ("Unrelated note", "It mentions leases once: lease."),
            ("Lease expiry", "How leases expire."),
        ]);
        let hits = book.search(&MemorySearch {
            query: Some("lease".to_owned()),
            ..MemorySearch::default()
        });
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].note.title, "Lease expiry");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn search_drops_misses_and_honors_limit() {
        let book = book_with(&[("Alpha", "one"), ("Beta", "two"), ("Gamma", "three")]);
        let hits = book.search(&MemorySearch {
            query: Some("nothing matches this".to_owned()),
            ..MemorySearch::default()
        });
        assert!(hits.is_empty());

        let all = book.search(&MemorySearch {
            limit: Some(2),
            ..MemorySearch::default()
        });
        assert_eq!(all.len(), 2);
        // No query means newest first.
        assert_eq!(all[0].note.title, "Gamma");
    }

    #[test]
    fn search_filters_stack() {
        let mut book = MemoryBook::default();
        book.write(
            agent("a"),
            note("Store gotcha", "Body.")
                .with_kind(MemoryKind::Gotcha)
                .with_paths(vec![path("src/store.rs")])
                .with_tags(vec!["storage".to_owned()]),
            at(0),
        )
        .unwrap();
        book.write(
            agent("a"),
            note("Server fact", "Body.")
                .with_kind(MemoryKind::Fact)
                .with_paths(vec![path("src/server.rs")])
                .with_tags(vec!["mcp".to_owned()]),
            at(1),
        )
        .unwrap();

        let by_kind = book.search(&MemorySearch {
            kind: Some(MemoryKind::Gotcha),
            ..MemorySearch::default()
        });
        assert_eq!(by_kind.len(), 1);
        assert_eq!(by_kind[0].note.title, "Store gotcha");

        let by_tag = book.search(&MemorySearch {
            tag: Some("#mcp".to_owned()),
            ..MemorySearch::default()
        });
        assert_eq!(by_tag.len(), 1);
        assert_eq!(by_tag[0].note.title, "Server fact");

        let by_path = book.search(&MemorySearch {
            path: Some(path("src/store.rs")),
            ..MemorySearch::default()
        });
        assert_eq!(by_path.len(), 1);

        let since = book.search(&MemorySearch {
            since: Some(at(1)),
            ..MemorySearch::default()
        });
        assert_eq!(since.len(), 1);
        assert_eq!(since[0].note.title, "Server fact");
    }

    #[test]
    fn a_repeated_common_word_does_not_beat_a_matching_title() {
        let mut book = MemoryBook::default();
        let padding = "the store is the thing that the store does. ".repeat(40);
        write(&mut book, "Unrelated rambling", &padding);
        write(&mut book, "Store", "Short and on topic.");

        let hits = book.search(&MemorySearch {
            query: Some("the store".to_owned()),
            ..MemorySearch::default()
        });
        assert_eq!(hits[0].note.title, "Store", "long note won on repetition");
    }

    #[test]
    fn short_terms_are_dropped_unless_the_query_is_all_short() {
        // "of" matches everything, so it must not drag in every note.
        let mut book = MemoryBook::default();
        write(&mut book, "Persistence of state", "A note.");
        write(&mut book, "Something else", "Nothing of interest here.");

        let hits = book.search(&MemorySearch {
            query: Some("of persistence".to_owned()),
            ..MemorySearch::default()
        });
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].note.title, "Persistence of state");

        // But an all-short query still works.
        let short = book.search(&MemorySearch {
            query: Some("of".to_owned()),
            ..MemorySearch::default()
        });
        assert_eq!(short.len(), 2);
    }

    #[test]
    fn substring_matching_still_finds_word_variants() {
        let mut book = MemoryBook::default();
        write(&mut book, "Claim leases renew on any call", "Body.");
        for query in ["lease", "renew", "leases renewal"] {
            let hits = book.search(&MemorySearch {
                query: Some(query.to_owned()),
                ..MemorySearch::default()
            });
            assert_eq!(hits.len(), 1, "{query} found nothing");
        }
    }

    #[test]
    fn a_default_search_is_bounded() {
        let mut book = MemoryBook::default();
        for index in 0..(DEFAULT_SEARCH_LIMIT + 5) {
            write(&mut book, &format!("Note {index}"), "Body.");
        }
        assert_eq!(
            book.search(&MemorySearch::default()).len(),
            DEFAULT_SEARCH_LIMIT
        );
        // Unbounded has to be asked for.
        assert_eq!(
            book.search(&MemorySearch {
                limit: None,
                ..MemorySearch::default()
            })
            .len(),
            DEFAULT_SEARCH_LIMIT + 5
        );
    }

    #[test]
    fn for_path_uses_directory_overlap() {
        let mut book = MemoryBook::default();
        book.write(
            agent("a"),
            note("About the store", "Body.").with_paths(vec![path("src/store.rs")]),
            at(0),
        )
        .unwrap();

        // A claim on the directory picks up notes about files beneath it.
        assert_eq!(book.for_path(&path("src"), None).len(), 1);
        assert_eq!(book.for_path(&path("src/store.rs"), None).len(), 1);
        // But not a sibling that merely shares a prefix.
        assert!(book.for_path(&path("srcs"), None).is_empty());
        assert!(book.for_path(&path("docs"), None).is_empty());
    }

    #[test]
    fn context_walks_relations_in_both_directions() {
        let book = book_with(&[
            ("Root note", "The root."),
            ("Child note", "Follows.\n\n- follows [[root-note]]"),
            ("Island", "Unconnected."),
        ]);

        // From the root, the backlink from the child is found.
        let from_root = book.context("root-note", 1);
        assert_eq!(from_root.len(), 2);
        assert_eq!(from_root[0].title, "Root note");
        assert!(from_root.iter().any(|n| n.title == "Child note"));
        assert!(from_root.iter().all(|n| n.title != "Island"));

        // From the child, the forward link is followed.
        let from_child = book.context("child-note", 1);
        assert!(from_child.iter().any(|n| n.title == "Root note"));

        assert!(book.context("no-such-note", 2).is_empty());
    }

    #[test]
    fn related_notes_are_capped() {
        let mut book = MemoryBook::default();
        write(&mut book, "Hub", "Everything points here.");
        for index in 0..(MAX_RELATED + 10) {
            write(&mut book, &format!("Spoke {index}"), "- follows [[hub]]");
        }
        let context = book.context("hub", 2);
        assert_eq!(context[0].title, "Hub");
        assert!(
            context.len() <= MAX_RELATED + 1,
            "context returned {} notes",
            context.len()
        );
    }

    #[test]
    fn notes_are_found_by_permalink_id_or_title() {
        let mut book = MemoryBook::default();
        let stored = write(&mut book, "Findable note", "Body.");
        assert!(book.get("findable-note").is_some());
        assert!(book.get(&stored.id.to_string()).is_some());
        assert!(book.get("Findable Note").is_some());
        assert!(book.get("absent").is_none());
    }

    #[test]
    fn tags_are_lowercased_deduped_and_stripped() {
        let mut book = MemoryBook::default();
        let tagged = book
            .write(
                agent("a"),
                note("Tagged", "Body.").with_tags(vec![
                    "#Storage".to_owned(),
                    "storage".to_owned(),
                    "  ".to_owned(),
                    "RMCP".to_owned(),
                ]),
                at(0),
            )
            .unwrap()
            .note;
        assert_eq!(tagged.tags, vec!["storage", "rmcp"]);
        assert!(tagged.has_tag("STORAGE"));
        assert!(tagged.has_tag("#rmcp"));
        assert!(!tagged.has_tag("absent"));
    }

    #[test]
    fn observation_tags_count_as_tags() {
        let mut book = MemoryBook::default();
        let observed = write(
            &mut book,
            "Observed",
            "- [lesson] Something happened #claims",
        );
        assert!(observed.tags.is_empty());
        assert!(observed.has_tag("claims"));
    }

    #[test]
    fn excerpts_skip_headings_and_truncate() {
        let mut book = MemoryBook::default();
        let long = write(
            &mut book,
            "Long",
            "# Heading\n\nThe quick brown fox jumps over the lazy dog.",
        );
        assert_eq!(
            long.excerpt(200),
            "The quick brown fox jumps over the lazy dog."
        );
        assert_eq!(long.excerpt(9), "The quick...");
    }

    #[test]
    fn a_digest_carries_no_body() {
        let mut book = MemoryBook::default();
        let long = write(
            &mut book,
            "Long note",
            "# Heading\n\nThis body is far longer than the excerpt allowance, which is exactly why a listing must not carry it around for every row it returns to a caller that only wanted to know what exists.",
        );
        let digest = long.digest();
        assert_eq!(digest.title, "Long note");
        assert_eq!(digest.permalink, long.permalink);
        assert_eq!(digest.updated_at, long.updated_at);
        assert!(digest.excerpt.chars().count() <= CLAIM_MEMORY_EXCERPT + 3);
        assert!(!digest.excerpt.contains('#'), "heading leaked into excerpt");

        // And it serializes without a body field at all.
        let json = serde_json::to_value(&digest).unwrap();
        assert!(json.get("body").is_none());
        assert!(json.get("observations").is_none());
        assert!(json.get("excerpt").is_some());
    }

    #[test]
    fn kinds_round_trip_through_strings() {
        for kind in MemoryKind::all() {
            assert_eq!(kind.as_str().parse::<MemoryKind>().unwrap(), kind);
        }
        assert!("nonsense".parse::<MemoryKind>().is_err());
    }

    #[test]
    fn relations_without_a_kind_default_to_relates_to() {
        let mut book = MemoryBook::default();
        let linker = write(&mut book, "Linker", "- [[other-note]]");
        assert_eq!(linker.relations.len(), 1);
        assert_eq!(linker.relations[0].kind, "relates_to");
        assert_eq!(linker.relations[0].target, "other-note");
    }

    #[test]
    fn a_note_can_be_retracted() {
        let mut book = MemoryBook::default();
        write(&mut book, "Keep this", "Fine.");
        let secret = write(&mut book, "Leaked secret", "Do not keep this.");
        write(&mut book, "Also keep", "Fine.");

        let removed = book.remove("leaked-secret").unwrap();
        assert_eq!(removed.id, secret.id);
        assert_eq!(book.len(), 2);
        assert!(book.get("leaked-secret").is_none());
        // The survivors are still reachable, which is the part a naive
        // Vec::remove breaks by leaving the position maps stale.
        assert_eq!(book.get("keep-this").unwrap().title, "Keep this");
        assert_eq!(book.get("also-keep").unwrap().title, "Also keep");
        assert!(book.remove("leaked-secret").is_none());
    }

    #[test]
    fn removal_keeps_search_and_writes_consistent() {
        let mut book = MemoryBook::default();
        write(&mut book, "First storage note", "About storage.");
        write(&mut book, "Second storage note", "Also about storage.");
        book.remove("first-storage-note").unwrap();

        let hits = book.search(&MemorySearch {
            query: Some("storage".to_owned()),
            ..MemorySearch::default()
        });
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].note.title, "Second storage note");

        // Writing after a removal must land on the right note.
        let again = book
            .write(
                agent("tester"),
                note("Second storage note", "Edited."),
                at(1),
            )
            .unwrap();
        assert!(!again.created);
        assert_eq!(book.len(), 1);
        assert_eq!(book.get("second-storage-note").unwrap().body, "Edited.");
    }

    #[test]
    fn removal_drops_the_notes_backlinks() {
        let mut book = MemoryBook::default();
        write(&mut book, "Root", "The root.");
        write(&mut book, "Follower", "- follows [[root]]");

        assert_eq!(book.context("root", 1).len(), 2);
        book.remove("follower").unwrap();
        // The backlink must go with the note, not linger as a stale index
        // entry pointing at a shifted position.
        assert_eq!(book.context("root", 1).len(), 1);
    }

    #[test]
    fn backlinks_follow_a_retitled_note() {
        let mut book = MemoryBook::default();
        write(&mut book, "Root", "The root.");
        write(&mut book, "Follower", "- follows [[root]]");
        assert_eq!(book.context("root", 1).len(), 2);

        // Rewriting the follower without the relation must drop the link.
        book.write(
            agent("tester"),
            note("Follower", "No longer follows anything."),
            at(1),
        )
        .unwrap();
        assert_eq!(book.context("root", 1).len(), 1);
    }

    #[test]
    fn title_hits_weigh_more_than_permalink_hits() {
        let empty = || Haystacks {
            title: String::new(),
            permalink: String::new(),
            tags: String::new(),
            observations: String::new(),
            body: String::new(),
        };
        let terms = vec!["lease".to_owned()];
        let in_title = Haystacks {
            title: "lease".to_owned(),
            ..empty()
        };
        let in_permalink = Haystacks {
            permalink: "lease".to_owned(),
            ..empty()
        };
        // One title hit is worth 6, plus 10 for matching every term.
        assert_eq!(score_with(&in_title, &terms), 16);
        assert!(score_with(&in_title, &terms) > score_with(&in_permalink, &terms));
    }
}
