//! The output schema each tool declares in `tools/list` (ADR-0034), so an
//! agent knows a result's shape before it calls.
//!
//! The schemas document; they never constrain. Every one is an object
//! that requires only `status`, allows additional properties, and types a
//! property only when its JSON type is certain: a field that can be null
//! is typed `[T, "null"]` or left untyped. A client that validates
//! structured content against them must never reject a real result, and
//! `tests/budgets.rs` checks that against a seeded daemon.
//!
//! The shapes restate `docs/1-about/04-primitives.md`, which remains the
//! source of truth.

use serde_json::{Map, Value, json};

/// Every status a result can carry. Each tool's `status` description says
/// which of them that tool produces.
const STATUSES: [&str; 6] = [
    "ok",
    "conflict",
    "not_found",
    "none",
    "invalid",
    "cancelled",
];

/// What any result may carry besides its own fields.
const PIGGYBACKS: &str =
    "Any result may also carry lost, inbox, inbox_more and persist_error; see guide.";

type Fields = Vec<(&'static str, Value)>;

fn typed(kind: &str, description: &str) -> Value {
    json!({ "type": kind, "description": description })
}

fn nullable(kind: &str, description: &str) -> Value {
    json!({ "type": [kind, "null"], "description": description })
}

fn untyped(description: &str) -> Value {
    json!({ "description": description })
}

fn strings(description: &str) -> Value {
    json!({ "type": "array", "items": { "type": "string" }, "description": description })
}

fn rows(description: &str) -> Value {
    json!({ "type": "array", "items": { "type": "object" }, "description": description })
}

/// The schema of a result: `status` with the outcomes this tool produces,
/// the tool's own `fields`, and `message`.
fn schema(outcomes: &str, fields: Fields) -> Map<String, Value> {
    let mut properties = Map::new();
    properties.insert(
        "status".to_owned(),
        json!({ "type": "string", "enum": STATUSES, "description": outcomes }),
    );
    for (name, field) in fields {
        properties.insert(name.to_owned(), field);
    }
    // `message_send` returns the message it sent under this key, so a tool
    // that declares `message` itself keeps its own definition.
    properties.entry("message".to_owned()).or_insert_with(|| {
        typed(
            "string",
            "Human-readable detail, on most outcomes other than ok.",
        )
    });
    let mut schema = Map::new();
    schema.insert("type".to_owned(), json!("object"));
    schema.insert("description".to_owned(), json!(PIGGYBACKS));
    schema.insert("required".to_owned(), json!(["status"]));
    schema.insert("properties".to_owned(), Value::Object(properties));
    schema
}

/// The fields every list tool returns around its rows.
fn listing(key: &'static str, row: &str) -> Fields {
    vec![
        ("count", typed("integer", "Rows in this response.")),
        (
            "total",
            typed("integer", "Rows that matched, across all pages."),
        ),
        (
            "truncated",
            typed("boolean", "True when older rows were left out."),
        ),
        (
            "next_before",
            typed(
                "string",
                "Present when truncated: pass it back as before for the next older page.",
            ),
        ),
        (key, rows(row)),
    ]
}

const TASK: &str = "id, title, description, priority, depends_on, paths, created_by, \
created_at, updated_at, notes, and state: its status with owner (in_progress), owner and \
reason (blocked), or by (done).";

const DIGEST: &str = "permalink, title, kind, paths, tags, updated_at, and a 160-character excerpt";

fn claims(tool: &str) -> Option<Map<String, Value>> {
    Some(match tool {
        "claim" => schema(
            "ok: every path is yours. conflict: nothing was claimed. cancelled: a wait whose caller left. invalid: bad input.",
            vec![
                (
                    "claim_id",
                    nullable(
                        "string",
                        "The new lease; null when every path was only renewed.",
                    ),
                ),
                ("new_paths", strings("Paths claimed by this call.")),
                ("renewed_paths", strings("Paths you already held, renewed.")),
                (
                    "absorbed_paths",
                    strings("Paths you held beneath a directory just claimed, folded into it."),
                ),
                (
                    "expires_at",
                    typed("string", "RFC 3339 end of the lease unless renewed."),
                ),
                (
                    "previous_owner",
                    rows(
                        "For a new path whose lease another agent lost in the last hour: path, owner, reaped_at. It may be half-edited.",
                    ),
                ),
                (
                    "notices",
                    rows(
                        "Brief: up to 5 unread notices for these paths (id, kind, summary, by). Now marked seen by you.",
                    ),
                ),
                (
                    "contracts",
                    rows(
                        "Brief: up to 5 contracts these paths consume (name, version, kind). Bodies via contract_get.",
                    ),
                ),
                (
                    "decisions",
                    rows("Brief: up to 5 decisions affecting these paths (id, title)."),
                ),
                (
                    "memory",
                    rows(
                        "Brief: up to 5 memory notes about these paths, as digests with an excerpt. Bodies via memory_read.",
                    ),
                ),
                (
                    "more",
                    typed(
                        "object",
                        "Per brief section, how many matching rows were left out; page with the list tools when not zero.",
                    ),
                ),
                (
                    "conflicts",
                    rows(
                        "On conflict, one per overlapping path: path, overlaps, owner, reason, expires_at.",
                    ),
                ),
            ],
        ),
        "release" => schema(
            "ok, or not_found when you do not hold a named path (nothing is released). invalid: bad input.",
            vec![
                ("agent", typed("string", "The caller.")),
                ("released", strings("The paths released.")),
            ],
        ),
        "renew" => schema(
            "ok. not_found: you hold no claims. invalid: bad input.",
            vec![
                ("agent", typed("string", "The caller.")),
                ("count", typed("integer", "Leases renewed.")),
                (
                    "expires_at",
                    nullable("string", "RFC 3339 latest expiry among the renewed leases."),
                ),
            ],
        ),
        "claims_list" => schema(
            "ok. invalid: bad input.",
            listing(
                "claims",
                "id, owner, paths, reason, claimed_at, expires_at, ttl_secs. Expired claims are never listed.",
            ),
        ),
        _ => return None,
    })
}

fn tasks(tool: &str) -> Option<Map<String, Value>> {
    Some(match tool {
        "task_create" => schema(
            "ok. not_found: an unknown depends_on id. invalid: bad input or an ambiguous id prefix.",
            vec![("task", typed("object", &format!("The new task: {TASK}")))],
        ),
        "task_pull" => schema(
            "ok: the task is yours, in_progress. none: no todo task is unblocked. cancelled: a wait whose caller left. invalid: bad input.",
            vec![
                (
                    "task",
                    typed("object", &format!("The task you now own: {TASK}")),
                ),
                (
                    "waiting_on",
                    rows(
                        "Present when the task's paths overlap what others hold: path, owner. Coordinate before editing those.",
                    ),
                ),
            ],
        ),
        "task_update" => schema(
            "ok. conflict: another agent has it in_progress and force was not set. not_found: no such task. invalid: bad status or an ambiguous id prefix.",
            vec![
                (
                    "task",
                    typed("object", &format!("The task after the change: {TASK}")),
                ),
                (
                    "owner",
                    untyped("On conflict: the agent that has the task in_progress."),
                ),
                (
                    "since",
                    untyped("On conflict: RFC 3339 time that agent took it."),
                ),
            ],
        ),
        "task_list" => schema(
            "ok. invalid: bad input.",
            listing(
                "tasks",
                "Most recently updated first; null fields and empty arrays are omitted, ids shortened to 8 characters.",
            ),
        ),
        _ => return None,
    })
}

fn contracts_and_notices(tool: &str) -> Option<Map<String, Value>> {
    Some(match tool {
        "contract_publish" => schema(
            "ok. conflict: expected_version was set and the contract is at another version. invalid: bad input.",
            vec![
                (
                    "contract",
                    typed(
                        "object",
                        "id, name, kind, consumers, current (version, shape, notes, published_by, published_at), history.",
                    ),
                ),
                (
                    "previous_version",
                    untyped("The version this one replaced; absent or null on a first publish."),
                ),
                (
                    "notice_id",
                    untyped("The change notice emitted to consumers of a republished contract."),
                ),
                (
                    "current_version",
                    typed("integer", "On conflict: the version the contract is at."),
                ),
                (
                    "published_by",
                    untyped("On conflict: who published that version."),
                ),
            ],
        ),
        "contract_get" => schema(
            "ok. not_found: no such contract. invalid: an ambiguous id prefix.",
            vec![(
                "contract",
                typed(
                    "object",
                    "In full: id, name, kind, consumers, current (version, shape, notes, published_by, published_at), and history, oldest first.",
                ),
            )],
        ),
        "contract_list" => schema(
            "ok. invalid: bad input.",
            listing(
                "contracts",
                "Most recently published first, compact: id, name, kind, consumers, current version. Shapes via contract_get.",
            ),
        ),
        "notice_publish" => schema(
            "ok. not_found: an unknown contract_id. invalid: bad kind or input.",
            vec![(
                "notice",
                typed(
                    "object",
                    "id, kind, summary, from, to, affected_paths, contract_id, published_by, published_at.",
                ),
            )],
        ),
        "notice_list" => schema(
            "ok, with zero rows and a message when unread is scoped to paths and you hold none. invalid: bad input.",
            listing(
                "notices",
                "Newest first: id, kind, summary, from, to, affected_paths, published_by, published_at. Rows returned for unread=true are now marked seen by you.",
            ),
        ),
        _ => return None,
    })
}

fn decisions_and_memory(tool: &str) -> Option<Map<String, Value>> {
    Some(match tool {
        "decision_record" => schema(
            "ok. invalid: bad input.",
            vec![(
                "decision",
                typed(
                    "object",
                    "id, title, decision, rationale, alternatives, affects_paths, recorded_by, recorded_at, and the permalink of its committed file.",
                ),
            )],
        ),
        "decision_list" => schema(
            "ok. invalid: bad input.",
            listing(
                "decisions",
                "Newest first: id, title, decision, rationale, alternatives, affects_paths, recorded_by, recorded_at.",
            ),
        ),
        "memory_write" => schema(
            "ok. conflict: if_updated_at was set and the note changed since. not_found: an explicit permalink that does not exist. invalid: bad kind or input.",
            vec![
                (
                    "note",
                    typed(
                        "object",
                        "The note as stored: id, permalink, title, kind, body, observations, relations, paths, tags, author, updated_by, created_at, updated_at.",
                    ),
                ),
                (
                    "created",
                    typed(
                        "boolean",
                        "True for a new note, false when an existing title or permalink was updated.",
                    ),
                ),
                ("permalink", untyped("On conflict: the note that changed.")),
                (
                    "updated_at",
                    untyped("On conflict: its real updated_at; read again and retry."),
                ),
            ],
        ),
        "memory_read" => schema(
            "ok. not_found: no such note. invalid: depth above 3.",
            vec![
                (
                    "note",
                    typed(
                        "object",
                        "The whole note, body included: id, permalink, title, kind, body, observations, relations, paths, tags, author, updated_by, created_at, updated_at.",
                    ),
                ),
                (
                    "related",
                    rows(&format!(
                        "With depth above 0, at most 20 linked notes as digests: {DIGEST}."
                    )),
                ),
            ],
        ),
        "memory_search" => schema(
            "ok. invalid: bad kind, since, or input.",
            vec![
                ("count", typed("integer", "Rows in this response.")),
                (
                    "truncated",
                    typed("boolean", "True when limit hid further matches."),
                ),
                (
                    "notes",
                    rows(&format!(
                        "Best match first, or newest first with no query. Digests, never bodies: {DIGEST}, and score."
                    )),
                ),
            ],
        ),
        "memory_delete" => schema(
            "ok. not_found: no such note.",
            vec![(
                "removed",
                typed(
                    "object",
                    &format!("A digest of the deleted note: {DIGEST}."),
                ),
            )],
        ),
        _ => return None,
    })
}

fn messages_and_server(tool: &str) -> Option<Map<String, Value>> {
    Some(match tool {
        "message_send" => schema(
            "ok. not_found: an unknown reply_to. invalid: empty or over-long text, a bad recipient, or agent human.",
            vec![(
                "message",
                json!({
                    "type": ["object", "string"],
                    "description": "On ok, the message as sent: id, from, to, text, reply_to, paths, at; audience lists the recipients of a broadcast. On any other outcome, why, as text.",
                }),
            )],
        ),
        "message_list" => schema(
            "ok. invalid: bad input.",
            listing(
                "messages",
                "Newest first, sent or received by you: id, from, to, text, reply_to, paths, at.",
            ),
        ),
        "status" => schema(
            "ok. invalid: a malformed agent name.",
            vec![
                ("version", typed("string", "The daemon's version.")),
                ("started_at", typed("string", "RFC 3339.")),
                ("now", typed("string", "RFC 3339 daemon clock.")),
                ("uptime_secs", typed("integer", "Seconds since start.")),
                ("seq", typed("integer", "Mutations applied since start.")),
                ("claims", typed("integer", "Live claims.")),
                ("tasks_open", typed("integer", "Tasks not done.")),
                ("tasks_done", typed("integer", "Tasks done.")),
                (
                    "tasks_orphaned",
                    typed(
                        "integer",
                        "Tasks returned to todo because their owner went silent.",
                    ),
                ),
                ("contracts", typed("integer", "Contracts.")),
                ("notices", typed("integer", "Notices.")),
                ("decisions", typed("integer", "Decisions.")),
                ("memory", typed("integer", "Memory notes.")),
                (
                    "agents_active",
                    typed("integer", "Agents holding a claim or a task."),
                ),
                (
                    "lead",
                    nullable("string", "The agent holding .tirith/lead, or null."),
                ),
                (
                    "persist_error",
                    nullable(
                        "string",
                        "The last failure to write state to disk, or null.",
                    ),
                ),
                (
                    "load_errors",
                    rows("Files or lines skipped at startup: path, line, error."),
                ),
                (
                    "agents",
                    rows(
                        "With verbose, at most 50: agent, paths_count, expires_at, tasks_in_progress.",
                    ),
                ),
                (
                    "agents_truncated",
                    typed("boolean", "With verbose: more agents than were listed."),
                ),
                (
                    "lead_expires_at",
                    untyped("With verbose: RFC 3339 end of the lead's lease."),
                ),
            ],
        ),
        "guide" => schema(
            "ok. invalid: an unknown topic; message lists the valid ones.",
            vec![
                ("topic", typed("string", "The page returned.")),
                ("purpose", typed("string", "Overview: what Tirith is for.")),
                ("loop", strings("Overview: the working loop, in order.")),
                (
                    "rules",
                    strings("Rules every reply follows (overview), or this topic's rules."),
                ),
                (
                    "tools",
                    untyped(
                        "Overview: topic to tool names. A topic: tool and when to call it, one per tool.",
                    ),
                ),
                ("topics", strings("Overview: every topic guide accepts.")),
                (
                    "summary",
                    typed("string", "A topic: what the primitive is for."),
                ),
            ],
        ),
        _ => return None,
    })
}

/// The output schema of `tool`, or `None` for a name this file does not
/// know. A test in `server.rs` fails when a routed tool has none.
pub(crate) fn output_schema(tool: &str) -> Option<Map<String, Value>> {
    claims(tool)
        .or_else(|| tasks(tool))
        .or_else(|| contracts_and_notices(tool))
        .or_else(|| decisions_and_memory(tool))
        .or_else(|| messages_and_server(tool))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn every_schema_requires_only_status_and_describes_every_property() {
        for tool in [
            "claim",
            "task_pull",
            "contract_publish",
            "memory_write",
            "status",
            "guide",
        ] {
            let schema = output_schema(tool).unwrap();
            assert_eq!(schema["type"], "object", "{tool}");
            assert_eq!(schema["required"], json!(["status"]), "{tool}");
            assert!(schema.get("additionalProperties").is_none(), "{tool}");
            for (name, property) in schema["properties"].as_object().unwrap() {
                let description = property["description"].as_str().unwrap_or("");
                assert!(!description.is_empty(), "{tool}.{name} has no description");
            }
        }
    }

    #[test]
    fn a_list_tool_documents_its_paging_fields() {
        let schema = output_schema("notice_list").unwrap();
        let properties = schema["properties"].as_object().unwrap();
        for key in ["count", "total", "truncated", "next_before", "notices"] {
            assert!(properties.contains_key(key), "missing {key}");
        }
    }

    #[test]
    fn message_send_keeps_its_own_message_which_is_an_object_on_ok() {
        let schema = output_schema("message_send").unwrap();
        assert_eq!(
            schema["properties"]["message"]["type"],
            json!(["object", "string"])
        );
        let other = output_schema("release").unwrap();
        assert_eq!(other["properties"]["message"]["type"], "string");
    }

    #[test]
    fn an_unknown_tool_has_no_schema() {
        assert!(output_schema("no_such_tool").is_none());
    }
}
