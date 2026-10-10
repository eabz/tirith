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

/// The `lead` field of a lead-only refusal (ADR-0036).
const LEAD: &str = "On a lead-only conflict: whom to message.";

/// What any result may carry besides its own fields.
const PIGGYBACKS: &str = "May also carry lost, inbox, inbox_more, persist_error.";

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
    properties
        .entry("message".to_owned())
        .or_insert_with(|| typed("string", "Detail, on outcomes other than ok."));
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
        ("total", typed("integer", "Rows matched, all pages.")),
        ("truncated", typed("boolean", "Older rows were left out.")),
        (
            "next_before",
            typed(
                "string",
                "When truncated: pass as before for the next page.",
            ),
        ),
        (key, rows(row)),
    ]
}

const TASK: &str = "id, title, description, priority, depends_on, paths, created_by, \
created_at, updated_at, notes, status, owner or by, reason (blocked).";

const DIGEST: &str = "permalink, title, kind, paths, tags, updated_at, and a 160-character excerpt";

fn claims(tool: &str) -> Option<Map<String, Value>> {
    Some(match tool {
        "claim" => schema(
            "ok: all paths yours. conflict: nothing claimed. cancelled: caller left. invalid.",
            vec![
                (
                    "claim_id",
                    nullable("string", "The lease; null when only renewed."),
                ),
                ("new_paths", strings("Paths claimed by this call.")),
                ("renewed_paths", strings("Paths you already held, renewed.")),
                (
                    "absorbed_paths",
                    strings("Your paths folded into a directory just claimed."),
                ),
                ("expires_at", typed("string", "RFC 3339 lease end.")),
                (
                    "previous_owner",
                    rows(
                        "Paths another agent lost in the last hour: path, owner, reaped_at; may be half-edited.",
                    ),
                ),
                (
                    "notices",
                    rows("Brief: up to 5 unread notices (id, kind, summary, by), now seen."),
                ),
                (
                    "contracts",
                    rows("Brief: up to 5 contracts (name, version, kind)."),
                ),
                ("decisions", rows("Brief: up to 5 decisions (id, title).")),
                ("memory", rows("Brief: up to 5 note digests.")),
                ("more", typed("object", "Per brief section, rows left out.")),
                (
                    "conflicts",
                    rows("On conflict: path, overlaps, owner, reason, expires_at."),
                ),
            ],
        ),
        "release" => schema(
            "ok. not_found: a path you do not hold; nothing released. invalid.",
            vec![
                ("agent", typed("string", "The caller.")),
                ("released", strings("The paths released.")),
            ],
        ),
        "renew" => schema(
            "ok. not_found: no claims. invalid.",
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
            "ok. invalid.",
            listing(
                "claims",
                "id, owner, paths, reason, claimed_at, expires_at, ttl_secs.",
            ),
        ),
        _ => return None,
    })
}

fn tasks(tool: &str) -> Option<Map<String, Value>> {
    Some(match tool {
        "task_create" => schema(
            "ok. conflict: lead-only. not_found: unknown depends_on. invalid.",
            vec![
                ("task", typed("object", &format!("The new task: {TASK}"))),
                ("lead", untyped(LEAD)),
            ],
        ),
        "task_pull" => schema(
            "ok: yours, in_progress. none: nothing unblocked. cancelled: caller left. invalid.",
            vec![
                (
                    "task",
                    typed("object", &format!("The task you now own: {TASK}")),
                ),
                ("waiting_on", rows("Paths others hold: path, owner.")),
            ],
        ),
        "task_update" => schema(
            "ok. conflict: another agent's in_progress task without force, or force is lead-only. not_found. invalid.",
            vec![
                (
                    "task",
                    typed("object", &format!("The task after the change: {TASK}")),
                ),
                ("lead", untyped(LEAD)),
                ("owner", untyped("On conflict: who has it.")),
                ("since", untyped("On conflict: since when.")),
            ],
        ),
        "task_list" => schema(
            "ok. invalid.",
            listing("tasks", "Compact task rows, most recently updated first."),
        ),
        _ => return None,
    })
}

fn contracts_and_notices(tool: &str) -> Option<Map<String, Value>> {
    Some(match tool {
        "contract_publish" => schema(
            "ok. conflict: expected_version mismatch, or lead-only. invalid.",
            vec![
                ("lead", untyped(LEAD)),
                (
                    "contract",
                    typed(
                        "object",
                        "id, name, kind, consumers, current (version, shape, notes, published_by, published_at), history.",
                    ),
                ),
                (
                    "previous_version",
                    untyped("Version replaced; null on first publish."),
                ),
                (
                    "notice_id",
                    untyped("Notice sent to consumers on a republish."),
                ),
                (
                    "current_version",
                    typed("integer", "On conflict: current version."),
                ),
                ("published_by", untyped("On conflict: its publisher.")),
            ],
        ),
        "contract_get" => schema(
            "ok. not_found. invalid: ambiguous prefix.",
            vec![(
                "contract",
                typed(
                    "object",
                    "id, name, kind, consumers, current (version, shape, notes, published_by, published_at), history oldest first.",
                ),
            )],
        ),
        "contract_list" => schema(
            "ok. invalid.",
            listing(
                "contracts",
                "id, name, kind, consumers, version; no shapes.",
            ),
        ),
        "notice_publish" => schema(
            "ok. not_found: unknown contract_id. invalid.",
            vec![(
                "notice",
                typed(
                    "object",
                    "id, kind, summary, from, to, affected_paths, contract_id, published_by, published_at.",
                ),
            )],
        ),
        "notice_list" => schema(
            "ok (zero rows and a message when unread and you hold nothing). invalid.",
            listing(
                "notices",
                "id, kind, summary, from, to, affected_paths, published_by, published_at; unread rows now seen.",
            ),
        ),
        _ => return None,
    })
}

fn decisions_and_memory(tool: &str) -> Option<Map<String, Value>> {
    Some(match tool {
        "decision_record" => schema(
            "ok. conflict: lead-only. invalid.",
            vec![
                (
                    "decision",
                    typed(
                        "object",
                        "id, title, decision, rationale, alternatives, affects_paths, recorded_by, recorded_at, permalink.",
                    ),
                ),
                ("lead", untyped(LEAD)),
            ],
        ),
        "decision_list" => schema(
            "ok. invalid.",
            listing(
                "decisions",
                "id, title, decision, rationale, alternatives, affects_paths, recorded_by, recorded_at.",
            ),
        ),
        "memory_write" => schema(
            "ok. conflict: changed since if_updated_at. not_found: unknown permalink. invalid.",
            vec![
                (
                    "note",
                    typed(
                        "object",
                        "id, permalink, title, kind, body, observations, relations, paths, tags, author, updated_by, created_at, updated_at.",
                    ),
                ),
                ("created", typed("boolean", "New note, or an update.")),
                ("permalink", untyped("On conflict: the note that changed.")),
                ("updated_at", untyped("On conflict: its real updated_at.")),
            ],
        ),
        "memory_read" => schema(
            "ok. not_found. invalid: depth above 3.",
            vec![
                (
                    "note",
                    typed(
                        "object",
                        "id, permalink, title, kind, body, observations, relations, paths, tags, author, updated_by, created_at, updated_at.",
                    ),
                ),
                (
                    "related",
                    rows(&format!("With depth: up to 20 linked digests ({DIGEST}).")),
                ),
            ],
        ),
        "memory_search" => schema(
            "ok. invalid.",
            vec![
                ("count", typed("integer", "Rows in this response.")),
                ("truncated", typed("boolean", "limit hid matches.")),
                (
                    "notes",
                    rows(&format!("Digests ({DIGEST}, score), best match first.")),
                ),
            ],
        ),
        "memory_delete" => schema(
            "ok. conflict: lead-only. not_found.",
            vec![
                ("lead", untyped(LEAD)),
                (
                    "removed",
                    typed("object", &format!("Digest of the deleted note: {DIGEST}.")),
                ),
            ],
        ),
        _ => return None,
    })
}

fn messages_and_server(tool: &str) -> Option<Map<String, Value>> {
    Some(match tool {
        "message_send" => schema(
            "ok. not_found: unknown reply_to. invalid.",
            vec![(
                "message",
                json!({
                    "type": ["object", "string"],
                    "description": "On ok: id, from, to, text, reply_to, paths, at, audience (broadcast). Otherwise: detail.",
                }),
            )],
        ),
        "message_list" => schema(
            "ok. invalid.",
            listing("messages", "id, from, to, text, reply_to, paths, at."),
        ),
        "status" => schema(
            "ok. invalid.",
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
                    typed("integer", "Tasks returned to todo from silent owners."),
                ),
                ("contracts", typed("integer", "Contracts.")),
                ("notices", typed("integer", "Notices.")),
                ("decisions", typed("integer", "Decisions.")),
                ("memory", typed("integer", "Memory notes.")),
                (
                    "agents_active",
                    typed("integer", "Agents with a claim or task."),
                ),
                (
                    "lead",
                    nullable("string", "Holder of .tirith/lead, or null."),
                ),
                (
                    "persist_error",
                    nullable("string", "Last write failure, or null."),
                ),
                (
                    "load_errors",
                    rows("Skipped at startup: path, line, error."),
                ),
                (
                    "agents",
                    rows(
                        "With verbose, max 50: agent, paths_count, expires_at, tasks_in_progress.",
                    ),
                ),
                (
                    "agents_truncated",
                    typed("boolean", "With verbose: more agents than listed."),
                ),
                ("lead_expires_at", untyped("With verbose: lead lease end.")),
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
