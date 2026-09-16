//! The memory primitive across the filesystem boundary.
//!
//! The unit tests in `src/memory.rs` cover the domain in isolation. These
//! cover what the store will actually do with it: write one Markdown file
//! per note into a directory, read the directory back, and end up with the
//! same notes. They also check that every note the repository ships in
//! `.tirith/memory/` parses, which is the standing proof of the import
//! compatibility ADR-0011 claims.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;

use tirith::memory::{MemoryBook, MemoryKind, MemoryNote, MemorySearch, NewMemory};
use tirith::types::{AgentId, RepoPath};

fn agent(name: &str) -> AgentId {
    AgentId::new(name).unwrap()
}

fn path(raw: &str) -> RepoPath {
    RepoPath::new(raw).unwrap()
}

/// Writes every note to its own file, the way `JsonStore::apply` will.
fn write_all(dir: &Path, book: &MemoryBook) {
    fs::create_dir_all(dir).unwrap();
    for note in book.notes() {
        let file = dir.join(note.permalink.file_path());
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(file, note.to_markdown()).unwrap();
    }
}

/// Scans a directory back into a book, the way `JsonStore::load` will.
///
/// The walk is recursive because a permalink may carry folder segments.
fn read_all(dir: &Path) -> MemoryBook {
    let mut notes = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current).unwrap() {
            let file = entry.unwrap().path();
            if file.is_dir() {
                stack.push(file);
            } else if file.extension().is_some_and(|e| e == "md") {
                let text = fs::read_to_string(&file).unwrap();
                notes.push(
                    MemoryNote::from_markdown(&text)
                        .unwrap_or_else(|e| panic!("{}: {e}", file.display())),
                );
            }
        }
    }
    MemoryBook::from_notes(notes)
}

fn seeded() -> MemoryBook {
    let mut book = MemoryBook::default();
    let now = chrono::Utc::now();
    book.write(
        agent("storage-claude"),
        NewMemory::new(
            "Persister writes are sequence-ordered".to_owned(),
            "A snapshot older than the last written one is skipped.\n\n\
                   - [design] Writes go through spawn_blocking #storage\n\
                   - follows [[claim-leases-renew-on-any-call]]"
                .to_owned(),
        )
        .with_kind(MemoryKind::Lesson)
        .with_paths(vec![path("src/store.rs")])
        .with_tags(vec!["storage".to_owned()]),
        now,
    )
    .unwrap();
    book.write(
        agent("claude-scaffold"),
        NewMemory::new(
            "Claim leases renew on any call".to_owned(),
            "Any tool call by the owning agent renews all of its leases.".to_owned(),
        )
        .with_kind(MemoryKind::Fact)
        .with_paths(vec![path("src/claims.rs"), path("src/state.rs")])
        .with_tags(vec!["claims".to_owned()]),
        now,
    )
    .unwrap();
    book
}

#[test]
fn notes_survive_a_round_trip_through_the_filesystem() {
    let dir = tempfile::tempdir().unwrap();
    let written = seeded();
    write_all(dir.path(), &written);

    // One file per note, named by permalink.
    assert!(
        dir.path()
            .join("persister-writes-are-sequence-ordered.md")
            .exists()
    );
    assert!(
        dir.path()
            .join("claim-leases-renew-on-any-call.md")
            .exists()
    );

    let loaded = read_all(dir.path());
    assert_eq!(loaded.len(), 2);
    for original in written.notes() {
        let found = loaded.get(original.permalink.as_str()).unwrap();
        assert_eq!(found, original);
    }
}

#[test]
fn editing_a_note_rewrites_exactly_one_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut book = seeded();
    write_all(dir.path(), &book);

    let untouched = dir.path().join("claim-leases-renew-on-any-call.md");
    let before = fs::read_to_string(&untouched).unwrap();

    let edited = book
        .write(
            agent("someone-else"),
            NewMemory::new(
                "Persister writes are sequence-ordered".to_owned(),
                "Rewritten body.".to_owned(),
            )
            .with_kind(MemoryKind::Lesson),
            chrono::Utc::now(),
        )
        .unwrap();
    assert!(!edited.created);

    // The store writes only the changed note.
    fs::write(
        dir.path().join(edited.note.permalink.file_path()),
        edited.note.to_markdown(),
    )
    .unwrap();

    assert_eq!(fs::read_to_string(&untouched).unwrap(), before);
    let loaded = read_all(dir.path());
    assert_eq!(loaded.len(), 2);
    assert_eq!(
        loaded
            .get("persister-writes-are-sequence-ordered")
            .unwrap()
            .body,
        "Rewritten body."
    );
}

#[test]
fn a_note_written_by_hand_is_picked_up() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path()).unwrap();
    fs::write(
        dir.path().join("hand-written.md"),
        "---\ntitle: Hand written\npaths:\n- src/server.rs\ntags: [mcp]\n---\n\n\
         Someone opened an editor and typed this.\n\n- [gotcha] server.rs must not branch on domain rules #layers\n",
    )
    .unwrap();

    let book = read_all(dir.path());
    assert_eq!(book.len(), 1);
    let note = book.get("hand-written").unwrap();
    assert_eq!(note.title, "Hand written");
    assert_eq!(note.paths, vec![path("src/server.rs")]);
    assert!(note.has_tag("mcp"));
    assert!(note.has_tag("layers"));

    // And it is found by a claim on the directory above it.
    assert_eq!(book.for_path(&path("src"), None).len(), 1);
}

#[test]
fn a_corrupt_file_is_an_error_not_a_panic() {
    let bad = "---\ntitle: Broken\nthis line has no colon\n---\nbody";
    let err = MemoryNote::from_markdown(bad).unwrap_err();
    assert!(err.to_string().contains("line 2"), "{err}");
}

#[test]
fn search_finds_notes_by_path_and_text() {
    let book = seeded();

    // What an agent claiming src/store.rs should be handed.
    let scoped = book.for_path(&path("src/store.rs"), None);
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].title, "Persister writes are sequence-ordered");

    let hits = book.search(&MemorySearch {
        query: Some("lease renewal".to_owned()),
        ..MemorySearch::default()
    });
    assert_eq!(hits[0].note.title, "Claim leases renew on any call");

    // No query is recent activity.
    let recent = book.search(&MemorySearch {
        limit: Some(1),
        ..MemorySearch::default()
    });
    assert_eq!(recent.len(), 1);
}

#[test]
fn relations_link_notes_into_context() {
    let book = seeded();
    let context = book.context("persister-writes-are-sequence-ordered", 1);
    assert_eq!(context.len(), 2);
    assert_eq!(context[0].title, "Persister writes are sequence-ordered");
    assert_eq!(context[1].title, "Claim leases renew on any call");
}

/// Every note the repository actually ships must parse.
///
/// The notes in `.tirith/memory/` were originally written by Basic Memory
/// and migrated in unchanged, so this doubles as the standing proof of the
/// import compatibility ADR-0011 claims. Skipped rather than failed when
/// the directory is absent, since a fresh clone has no notes yet.
#[test]
fn the_repositorys_own_notes_parse() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".tirith")
        .join("memory");
    if !dir.exists() {
        return;
    }
    let mut checked = 0_usize;
    let mut stack = vec![dir.clone()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current).unwrap() {
            let file = entry.unwrap().path();
            if file.is_dir() {
                stack.push(file);
            } else if file.extension().is_some_and(|e| e == "md") {
                let text = fs::read_to_string(&file).unwrap();
                let note = MemoryNote::from_markdown(&text)
                    .unwrap_or_else(|e| panic!("{} did not parse: {e}", file.display()));
                assert!(!note.title.is_empty());
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "{} exists but holds no notes", dir.display());
}

#[test]
fn notes_can_live_in_folders() {
    let dir = tempfile::tempdir().unwrap();
    let mut book = MemoryBook::default();
    // Seed a note, then re-file it under a folder permalink.
    let flat = book
        .write(
            agent("a"),
            NewMemory::new("Storage design".to_owned(), "Body.".to_owned()),
            chrono::Utc::now(),
        )
        .unwrap()
        .note;
    assert_eq!(flat.permalink.file_path(), "storage-design.md");

    let mut nested =
        MemoryBook::from_notes(vec![MemoryNote::from_markdown(
        "---\ntitle: Pre-alpha build\npermalink: tirith/design/pre-alpha-build\n---\n\nBody.\n",
    )
    .unwrap()]);
    let note = nested.get("tirith/design/pre-alpha-build").unwrap();
    assert_eq!(note.permalink.folders(), vec!["tirith", "design"]);
    assert_eq!(
        note.permalink.file_path(),
        "tirith/design/pre-alpha-build.md"
    );

    write_all(dir.path(), &nested);
    assert!(
        dir.path()
            .join("tirith/design/pre-alpha-build.md")
            .is_file()
    );

    // And the recursive scan finds it again.
    let loaded = read_all(dir.path());
    assert_eq!(loaded.len(), 1);
    assert!(loaded.get("tirith/design/pre-alpha-build").is_some());

    // Editing through the folder permalink keeps the folder.
    let updated = nested
        .write(
            agent("b"),
            NewMemory::new("Pre-alpha build".to_owned(), "Edited.".to_owned())
                .with_permalink(note.permalink.clone()),
            chrono::Utc::now(),
        )
        .unwrap();
    assert!(!updated.created);
    assert_eq!(
        updated.note.permalink.file_path(),
        "tirith/design/pre-alpha-build.md"
    );
}

// ---------------------------------------------------------------------------
// The memory tools over a real daemon.
// ---------------------------------------------------------------------------

mod mcp {
    use std::net::SocketAddr;
    use std::path::Path;

    use serde_json::{Value, json};
    use tirith::client::call_tool;
    use tirith::memory::{CLAIM_MEMORY_LIMIT, MAX_CONTEXT_DEPTH};
    use tirith::server::{ServeOptions, ServerHandle, start};

    fn options(root: &Path) -> ServeOptions {
        ServeOptions {
            bind: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            repo_root: root.to_path_buf(),
            clock: None,

            registry: None,
        }
    }

    async fn call(handle: &ServerHandle, tool: &str, args: Value) -> Value {
        call_tool(&handle.mcp_url(), tool, args).await.unwrap()
    }

    async fn write_note(handle: &ServerHandle, title: &str, body: &str, paths: Value) -> Value {
        call(
            handle,
            "memory_write",
            json!({
                "agent": "scribe",
                "title": title,
                "body": body,
                "kind": "lesson",
                "paths": paths,
                "tags": ["storage"],
            }),
        )
        .await
    }

    #[tokio::test]
    async fn write_read_and_search_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let handle = start(options(dir.path())).await.unwrap();

        let written = write_note(
            &handle,
            "Persister writes are sequence-ordered",
            "A snapshot older than the last written one is skipped.\n\n- [design] Writes go through spawn_blocking #storage",
            json!(["src/store.rs"]),
        )
        .await;
        assert_eq!(written["status"], "ok");
        assert_eq!(written["created"], true);
        let permalink = written["note"]["permalink"].as_str().unwrap().to_owned();
        assert_eq!(permalink, "persister-writes-are-sequence-ordered");
        assert_eq!(written["note"]["kind"], "lesson");
        assert_eq!(written["note"]["observations"][0]["category"], "design");

        // Writing the same title again updates in place.
        let again = write_note(
            &handle,
            "Persister writes are sequence-ordered",
            "Rewritten body.",
            json!(["src/store.rs"]),
        )
        .await;
        assert_eq!(again["created"], false);
        assert_eq!(again["note"]["id"], written["note"]["id"]);

        let read = call(
            &handle,
            "memory_read",
            json!({ "agent": "reader", "name": permalink }),
        )
        .await;
        assert_eq!(read["status"], "ok");
        assert_eq!(read["note"]["body"], "Rewritten body.");
        assert_eq!(read["related"], json!([]));

        // The update replaced the body, so text that was only in the old
        // body is gone from the index as well as from the note.
        let stale = call(
            &handle,
            "memory_search",
            json!({ "agent": "reader", "query": "spawn_blocking" }),
        )
        .await;
        assert_eq!(stale["status"], "ok");
        assert_eq!(stale["count"], 0);

        // The title survived the update, so it still matches.
        let by_word = call(
            &handle,
            "memory_search",
            json!({ "agent": "reader", "query": "sequence" }),
        )
        .await;
        assert_eq!(by_word["count"], 1);

        let by_title = call(
            &handle,
            "memory_search",
            json!({ "agent": "reader", "query": "persister" }),
        )
        .await;
        assert_eq!(by_title["count"], 1);
        assert!(by_title["notes"][0]["score"].as_u64().unwrap() > 0);
        assert_eq!(by_title["truncated"], false);

        // No query is recent activity.
        let recent = call(&handle, "memory_search", json!({ "agent": "reader" })).await;
        assert_eq!(recent["count"], 1);

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn unknown_notes_and_bad_input_are_refused_not_errors() {
        let dir = tempfile::tempdir().unwrap();
        let handle = start(options(dir.path())).await.unwrap();

        let missing = call(
            &handle,
            "memory_read",
            json!({ "agent": "reader", "name": "no-such-note" }),
        )
        .await;
        assert_eq!(missing["status"], "not_found");

        let too_deep = call(
            &handle,
            "memory_read",
            json!({ "agent": "reader", "name": "x", "depth": MAX_CONTEXT_DEPTH + 1 }),
        )
        .await;
        assert_eq!(too_deep["status"], "invalid");

        let bad_kind = call(
            &handle,
            "memory_write",
            json!({ "agent": "scribe", "title": "T", "body": "b", "kind": "nonsense" }),
        )
        .await;
        assert_eq!(bad_kind["status"], "invalid");

        let empty = call(
            &handle,
            "memory_write",
            json!({ "agent": "scribe", "title": "  ", "body": "b" }),
        )
        .await;
        assert_eq!(empty["status"], "invalid");

        // Targeting a permalink that does not exist is not_found, not a
        // silent create.
        let ghost = call(
            &handle,
            "memory_write",
            json!({ "agent": "scribe", "title": "T", "body": "b", "permalink": "does-not-exist" }),
        )
        .await;
        assert_eq!(ghost["status"], "not_found");

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn notes_survive_a_restart_as_markdown_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let handle = start(options(dir.path())).await.unwrap();
        write_note(
            &handle,
            "Lease renewal",
            "Any call by the owning agent renews all its leases.",
            json!(["src/claims.rs"]),
        )
        .await;
        handle.shutdown().await.unwrap();

        // The note is a readable Markdown file, not a line in a log.
        let file = dir.path().join(".tirith/memory/lease-renewal.md");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("---\n"), "{text}");
        assert!(text.contains("title: Lease renewal"));
        assert!(text.contains("- src/claims.rs"));
        assert!(text.contains("Any call by the owning agent renews all its leases."));

        let handle = start(options(dir.path())).await.unwrap();
        let read = call(
            &handle,
            "memory_read",
            json!({ "agent": "reader", "name": "lease-renewal" }),
        )
        .await;
        assert_eq!(read["status"], "ok");
        assert_eq!(read["note"]["title"], "Lease renewal");
        assert_eq!(read["note"]["paths"], json!(["src/claims.rs"]));
        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_claim_carries_back_the_notes_for_its_paths() {
        let dir = tempfile::tempdir().unwrap();
        let handle = start(options(dir.path())).await.unwrap();

        // More notes about src/ than a claim is allowed to return.
        for index in 0..(CLAIM_MEMORY_LIMIT + 3) {
            write_note(
                &handle,
                &format!("Note number {index}"),
                &format!("Body of note {index}, which is long enough to need an excerpt rather than being sent whole to every agent that claims this directory."),
                json!([format!("src/file{index}.rs")]),
            )
            .await;
        }
        // And one about somewhere else entirely.
        write_note(
            &handle,
            "About the docs",
            "Nothing to do with src.",
            json!(["docs"]),
        )
        .await;

        let claimed = call(
            &handle,
            "claim",
            json!({ "agent": "worker", "paths": ["src"], "reason": "refactor" }),
        )
        .await;
        assert_eq!(claimed["status"], "ok");

        let memory = claimed["memory"].as_array().unwrap();
        assert_eq!(memory.len(), CLAIM_MEMORY_LIMIT);
        for row in memory {
            // An excerpt, never the body, so claim responses stay small.
            assert!(row.get("body").is_none(), "claim leaked a note body");
            assert!(row["excerpt"].as_str().unwrap().len() <= 200);
            assert!(row["permalink"].as_str().is_some());
            assert!(row["title"].as_str().unwrap().starts_with("Note number"));
        }

        // A claim on an unrelated path carries nothing: an empty brief
        // section is omitted rather than sent as [] (ADR-0014).
        let elsewhere = call(
            &handle,
            "claim",
            json!({ "agent": "other", "paths": ["Cargo.toml"], "reason": "bump" }),
        )
        .await;
        assert_eq!(elsewhere["status"], "ok");
        assert!(elsewhere.get("memory").is_none(), "{elsewhere}");
        assert_eq!(elsewhere["more"]["memory"], 0);

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn search_bounds_its_results() {
        let dir = tempfile::tempdir().unwrap();
        let handle = start(options(dir.path())).await.unwrap();
        for index in 0..8 {
            write_note(
                &handle,
                &format!("Storage note {index}"),
                "All of these mention storage.",
                json!(["src/store.rs"]),
            )
            .await;
        }

        let limited = call(
            &handle,
            "memory_search",
            json!({ "agent": "reader", "query": "storage", "limit": 3 }),
        )
        .await;
        assert_eq!(limited["count"], 3);
        assert_eq!(limited["truncated"], true);

        let all = call(
            &handle,
            "memory_search",
            json!({ "agent": "reader", "query": "storage", "limit": 50 }),
        )
        .await;
        assert_eq!(all["count"], 8);
        assert_eq!(all["truncated"], false);

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn relations_are_walked_on_read() {
        let dir = tempfile::tempdir().unwrap();
        let handle = start(options(dir.path())).await.unwrap();
        write_note(&handle, "Root note", "The root.", json!(["src"])).await;
        write_note(
            &handle,
            "Child note",
            "Follows the root.\n\n- follows [[root-note]]",
            json!(["src"]),
        )
        .await;

        let shallow = call(
            &handle,
            "memory_read",
            json!({ "agent": "reader", "name": "child-note" }),
        )
        .await;
        assert_eq!(shallow["related"], json!([]));

        let deep = call(
            &handle,
            "memory_read",
            json!({ "agent": "reader", "name": "child-note", "depth": 1 }),
        )
        .await;
        assert_eq!(deep["related"].as_array().unwrap().len(), 1);
        assert_eq!(deep["related"][0]["title"], "Root note");

        handle.shutdown().await.unwrap();
    }
}

mod mcp_v4 {
    use std::net::SocketAddr;
    use std::path::Path;

    use serde_json::{Value, json};
    use tirith::client::call_tool;
    use tirith::server::{ServeOptions, ServerHandle, start};

    fn options(root: &Path) -> ServeOptions {
        ServeOptions {
            bind: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            repo_root: root.to_path_buf(),
            clock: None,

            registry: None,
        }
    }

    async fn call(handle: &ServerHandle, tool: &str, args: Value) -> Value {
        call_tool(&handle.mcp_url(), tool, args).await.unwrap()
    }

    const LONG_BODY: &str = "A body long enough that a listing must not carry it. It goes on for a while so that the excerpt has to cut it, and so that twenty of these in one search response would cost real context.";

    async fn write(handle: &ServerHandle, title: &str) -> Value {
        call(
            handle,
            "memory_write",
            json!({
                "agent": "scribe", "title": title, "body": LONG_BODY,
                "paths": ["src/store.rs"],
            }),
        )
        .await
    }

    /// Only `memory_read` returns a body. Listings carry digests.
    #[tokio::test]
    async fn search_and_related_rows_are_digests() {
        let dir = tempfile::tempdir().unwrap();
        let handle = start(options(dir.path())).await.unwrap();
        write(&handle, "Root note").await;
        call(
            &handle,
            "memory_write",
            json!({
                "agent": "scribe", "title": "Child note",
                "body": format!("{LONG_BODY}\n\n- follows [[root-note]]"),
            }),
        )
        .await;

        let found = call(
            &handle,
            "memory_search",
            json!({ "agent": "reader", "query": "listing" }),
        )
        .await;
        assert_eq!(found["count"], 2);
        for row in found["notes"].as_array().unwrap() {
            assert!(
                row.get("body").is_none(),
                "search row carried a body: {row}"
            );
            assert!(row.get("observations").is_none());
            assert!(row["excerpt"].as_str().unwrap().len() < LONG_BODY.len());
            assert!(row["score"].as_u64().unwrap() > 0);
            assert!(row["permalink"].as_str().is_some());
        }

        let read = call(
            &handle,
            "memory_read",
            json!({ "agent": "reader", "name": "child-note", "depth": 1 }),
        )
        .await;
        assert_eq!(
            read["note"]["body"],
            LONG_BODY.to_owned() + "\n\n- follows [[root-note]]"
        );
        let related = read["related"].as_array().unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0]["title"], "Root note");
        assert!(
            related[0].get("body").is_none(),
            "related note carried a body"
        );

        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_note_can_be_deleted_and_stays_deleted_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let handle = start(options(dir.path())).await.unwrap();
        write(&handle, "Keep me").await;
        write(&handle, "Leaked secret").await;
        let file = dir.path().join(".tirith/memory/leaked-secret.md");
        handle.shutdown().await.unwrap();
        assert!(file.is_file(), "note was never written");

        let handle = start(options(dir.path())).await.unwrap();
        let deleted = call(
            &handle,
            "memory_delete",
            json!({ "agent": "scribe", "name": "leaked-secret" }),
        )
        .await;
        assert_eq!(deleted["status"], "ok");
        assert_eq!(deleted["removed"]["permalink"], "leaked-secret");
        assert!(deleted["removed"].get("body").is_none());

        let again = call(
            &handle,
            "memory_delete",
            json!({ "agent": "scribe", "name": "leaked-secret" }),
        )
        .await;
        assert_eq!(again["status"], "not_found");
        handle.shutdown().await.unwrap();

        // The file is gone from disk and a restart does not resurrect it.
        assert!(!file.exists(), "deleted note file still on disk");
        let handle = start(options(dir.path())).await.unwrap();
        let read = call(
            &handle,
            "memory_read",
            json!({ "agent": "reader", "name": "leaked-secret" }),
        )
        .await;
        assert_eq!(read["status"], "not_found");
        let kept = call(
            &handle,
            "memory_read",
            json!({ "agent": "reader", "name": "keep-me" }),
        )
        .await;
        assert_eq!(kept["status"], "ok");
        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn a_stale_write_is_a_conflict_not_a_clobber() {
        let dir = tempfile::tempdir().unwrap();
        let handle = start(options(dir.path())).await.unwrap();
        let first = write(&handle, "Shared note").await;
        let seen_at = first["note"]["updated_at"].as_str().unwrap().to_owned();

        // Someone else writes in between. The clock only has whole-second
        // resolution in frontmatter, so make sure the instant differs.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let theirs = call(
            &handle,
            "memory_write",
            json!({ "agent": "other", "title": "Shared note", "body": "Their version." }),
        )
        .await;
        assert_eq!(theirs["status"], "ok");
        assert_ne!(theirs["note"]["updated_at"].as_str().unwrap(), seen_at);

        // Writing with the stale timestamp is refused, and says when the
        // note really changed so the caller can re-read and retry.
        let stale = call(
            &handle,
            "memory_write",
            json!({
                "agent": "scribe", "title": "Shared note", "body": "My version.",
                "if_updated_at": seen_at,
            }),
        )
        .await;
        assert_eq!(stale["status"], "conflict", "{stale}");
        assert_eq!(stale["permalink"], "shared-note");
        assert_eq!(stale["updated_at"], theirs["note"]["updated_at"]);

        let read = call(
            &handle,
            "memory_read",
            json!({ "agent": "reader", "name": "shared-note" }),
        )
        .await;
        assert_eq!(
            read["note"]["body"], "Their version.",
            "stale write clobbered"
        );

        // Passing the current timestamp back succeeds.
        let fresh = call(
            &handle,
            "memory_write",
            json!({
                "agent": "scribe", "title": "Shared note", "body": "Merged version.",
                "if_updated_at": theirs["note"]["updated_at"],
            }),
        )
        .await;
        assert_eq!(fresh["status"], "ok", "{fresh}");
        assert_eq!(fresh["created"], false);

        handle.shutdown().await.unwrap();
    }
}
