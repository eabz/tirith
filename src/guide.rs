//! The content of the `guide` tool: what Tirith is for and how an agent
//! works with it, in one call (ADR-0034). Pure data, no state. The server
//! maps the tool to [`guide`]; a test in `server.rs` pins every tool named
//! here to the tool router and every router tool to this file, so the
//! guide cannot drift from the tool surface.

use serde_json::{Map, Value, json};

/// The topics `guide` accepts, in the order it lists them. `overview` is
/// the default.
pub(crate) const TOPICS: [&str; 10] = [
    "overview",
    "claims",
    "tasks",
    "contracts",
    "notices",
    "decisions",
    "memory",
    "messages",
    "lead",
    "server",
];

/// A tool and the situation that calls for it.
struct Entry {
    tool: &'static str,
    when: &'static str,
}

/// One primitive: what it is for, its tools, and the rules that bite.
struct Section {
    name: &'static str,
    summary: &'static str,
    tools: &'static [Entry],
    rules: &'static [&'static str],
}

const PURPOSE: &str = "Tirith keeps parallel coding agents in one repository from colliding. \
Agents lease the paths they edit, publish the shape of an interface before building either side \
of it, announce changes that break other files, record what is settled, and leave notes for \
whoever touches the same paths next. It stores no conversation history and no embeddings.";

const LOOP: [&str; 7] = [
    "Pick a stable, unique agent name and pass it on every call.",
    "Start with memory_search and no query for recent activity; task_pull if the board is in use.",
    "claim the paths you will edit, before editing. The ok reply is a brief of the unread notices, contracts, decisions and memory notes for those paths: read it first.",
    "On conflict nothing was claimed: do not edit. Take other work, or claim again with wait_secs so the daemon waits for the paths.",
    "contract_publish before implementing an interface another agent consumes; notice_publish after a rename or signature change that reaches files you did not claim.",
    "decision_record what is now settled; memory_write what the next agent should know about these paths.",
    "release when you are done.",
];

const RULES: [&str; 7] = [
    "Every result is JSON with a status: ok, conflict, not_found, none or invalid. Failures are statuses, never MCP errors.",
    "Any call renews your leases. A lease still ends after four TTLs, until you claim or renew.",
    "lost on any reply names leases of yours that ended: stop editing those paths and claim them again.",
    "inbox on any reply carries up to five messages from other agents (inbox_more counts the rest), so there is nothing to poll.",
    "persist_error on a reply means the change stands in memory but was not written to disk.",
    "List tools return 20 rows, newest first. Pass next_before back as before for the next page.",
    "Hold files many agents edit only while writing them: prepare, claim with wait_secs, write, release.",
];

const SECTIONS: [Section; 9] = [
    Section {
        name: "claims",
        summary: "Leases on files or directories. A path covers everything beneath it, overlaps are refused, and a lease ends when its TTL passes without a call from you.",
        tools: &[
            Entry {
                tool: "claim",
                when: "Before every edit. The ok reply is your brief for those paths. On conflict nothing is claimed; wait_secs lets the daemon wait for the paths instead of you retrying.",
            },
            Entry {
                tool: "release",
                when: "When you finish editing, so others can take the paths. Omit paths to drop everything you hold.",
            },
            Entry {
                tool: "renew",
                when: "Only during long work with no other call; every call already renews.",
            },
            Entry {
                tool: "claims_list",
                when: "To see who holds a path before claiming it, or the whole board with all=true.",
            },
        ],
        rules: &[
            "A lease ends after its TTL without activity, and after four TTLs regardless.",
            "Re-claiming a path you hold renews it; claiming a directory absorbs your claims beneath it.",
            "A path#Symbol entry claims one symbol inside a file (experimental, ADR-0029).",
        ],
    },
    Section {
        name: "tasks",
        summary: "A shared board: tasks with a priority, dependencies, and the paths they expect to touch.",
        tools: &[
            Entry {
                tool: "task_create",
                when: "To put work on the board. depends_on holds it until those tasks are done; paths lets task_pull steer agents away from each other.",
            },
            Entry {
                tool: "task_pull",
                when: "To take the next task: highest priority, dependencies done, preferring paths nobody holds. none means nothing is unblocked; wait_secs waits for work.",
            },
            Entry {
                tool: "task_update",
                when: "To finish (done), hand back (todo) or park (blocked, the note is the reason) a task. Another agent's in_progress task is a conflict unless force.",
            },
            Entry {
                tool: "task_list",
                when: "To inspect the board by status or owner without taking anything.",
            },
        ],
        rules: &[
            "An in_progress task whose owner makes no call for the orphan threshold (30 minutes by default) returns to todo.",
            "A pulled task with waiting_on overlaps paths another agent holds: coordinate before editing them.",
        ],
    },
    Section {
        name: "contracts",
        summary: "An interface shape published before either side implements it, versioned, with the paths that consume it.",
        tools: &[
            Entry {
                tool: "contract_publish",
                when: "Before implementing something another agent will call. Republishing a name makes a new version and notifies its consumers by itself; expected_version refuses a concurrent overwrite.",
            },
            Entry {
                tool: "contract_get",
                when: "To read one contract's full shape and its version history.",
            },
            Entry {
                tool: "contract_list",
                when: "To find the contracts a path consumes, or all of one kind.",
            },
        ],
        rules: &["Omitting consumers on a republish keeps the list; an empty list clears it."],
    },
    Section {
        name: "notices",
        summary: "Something changed and these files care: renames, signature changes, removals, moves, behavior changes.",
        tools: &[
            Entry {
                tool: "notice_publish",
                when: "After a change that breaks or surprises code outside the paths you claimed. Agents holding the affected paths get it in their inbox at once.",
            },
            Entry {
                tool: "notice_list",
                when: "To page the notices for a path, or with unread=true to see what you have not been shown. Listing unread notices marks them seen.",
            },
        ],
        rules: &[
            "A notice is seen when Tirith delivers it, in a brief or an unread listing; there is nothing to acknowledge.",
            "A contract republish emits its own notice; do not publish a second one.",
        ],
    },
    Section {
        name: "decisions",
        summary: "Settled choices with their rationale, so nothing is decided twice.",
        tools: &[
            Entry {
                tool: "decision_record",
                when: "Once a choice is final. It becomes one committed Markdown file under .tirith/decisions/.",
            },
            Entry {
                tool: "decision_list",
                when: "Before deciding, to check what is already settled for a path or a topic.",
            },
        ],
        rules: &[],
    },
    Section {
        name: "memory",
        summary: "Durable notes about repository paths: lessons, traps, handoffs. The next agent to claim those paths gets an excerpt in its brief.",
        tools: &[
            Entry {
                tool: "memory_search",
                when: "With no query at the start of a session, for recent activity; with a query, path, kind or tag to find a note. Rows are digests, never bodies.",
            },
            Entry {
                tool: "memory_read",
                when: "To read one note in full: the only call that returns a body. depth adds related notes.",
            },
            Entry {
                tool: "memory_write",
                when: "To leave what you learned about paths. A known title updates that note; pass if_updated_at from your read so a concurrent edit is refused instead of lost.",
            },
            Entry {
                tool: "memory_delete",
                when: "To retract a note that holds a secret or a wrong fact.",
            },
        ],
        rules: &[
            "Notes are committed Markdown under .tirith/memory/. Memory is for what the repository does not already say.",
        ],
    },
    Section {
        name: "messages",
        summary: "Short notes between agents, delivered on the recipient's next call whatever it asked for.",
        tools: &[
            Entry {
                tool: "message_send",
                when: "To coordinate with an agent by name, with every agent active in the last hour (*), or with the human queue (human).",
            },
            Entry {
                tool: "message_list",
                when: "To page your conversations. New messages already arrive as inbox on any reply.",
            },
        ],
        rules: &[
            "Messages are runtime state, dropped after 24 hours; they are not a record.",
            "A client's own session messaging is invisible to other clients: talk through Tirith.",
        ],
    },
    Section {
        name: "lead",
        summary: "A swarm has one lead: whoever holds a claim on exactly .tirith/lead.",
        tools: &[
            Entry {
                tool: "claim",
                when: "The session that spawns other agents claims .tirith/lead first, with ttl_secs 3600 and a reason naming the swarm, claims it again on lost, and releases it last.",
            },
            Entry {
                tool: "status",
                when: "To find the current lead, reported as lead.",
            },
            Entry {
                tool: "message_send",
                when: "Workers escalate to the lead by name. Only the lead messages human, with a text written for the human.",
            },
        ],
        rules: &["Workers never claim .tirith/lead."],
    },
    Section {
        name: "server",
        summary: "The daemon itself.",
        tools: &[
            Entry {
                tool: "status",
                when: "For a health check: counts, the lead, persistence and load problems. verbose=true adds who holds what.",
            },
            Entry {
                tool: "guide",
                when: "When you are unsure how the pieces fit, or which tool a situation calls for.",
            },
        ],
        rules: &[],
    },
];

/// The guide for `topic`, or the overview when it is `None`, blank, or
/// `overview`. Topic names match ASCII case-insensitively.
///
/// # Errors
///
/// An unknown topic, as a message that names the valid ones.
pub(crate) fn guide(topic: Option<&str>) -> Result<Value, String> {
    let wanted = topic
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("overview");
    if wanted.eq_ignore_ascii_case("overview") {
        return Ok(overview());
    }
    SECTIONS
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(wanted))
        .map(section)
        .ok_or_else(|| format!("unknown topic `{wanted}`; one of: {}", TOPICS.join(", ")))
}

/// The whole protocol on one page: purpose, the working loop, the rules
/// every reply follows, and the tools grouped by topic.
fn overview() -> Value {
    let tools: Map<String, Value> = SECTIONS
        .iter()
        .map(|s| {
            let names: Vec<&str> = s.tools.iter().map(|e| e.tool).collect();
            (s.name.to_owned(), json!(names))
        })
        .collect();
    let topics: Vec<&str> = TOPICS.iter().copied().skip(1).collect();
    json!({
        "topic": "overview",
        "message": format!("guide: overview; topics: {}", topics.join(", ")),
        "purpose": PURPOSE,
        "loop": LOOP,
        "rules": RULES,
        "tools": tools,
        "topics": TOPICS,
    })
}

/// One topic: what it is for, when to call each of its tools, its rules.
fn section(section: &Section) -> Value {
    let tools: Vec<Value> = section
        .tools
        .iter()
        .map(|e| json!({ "tool": e.tool, "when": e.when }))
        .collect();
    json!({
        "topic": section.name,
        "message": format!("guide: {}; {} tools", section.name, tools.len()),
        "summary": section.summary,
        "tools": tools,
        "rules": section.rules,
    })
}

/// Every tool the guide names, for the test that pins it to the router.
#[cfg(test)]
pub(crate) fn tool_names() -> Vec<&'static str> {
    let mut names: Vec<&str> = SECTIONS
        .iter()
        .flat_map(|s| s.tools.iter().map(|e| e.tool))
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn the_default_and_overview_are_the_same_page() {
        let default = guide(None).unwrap();
        assert_eq!(default, guide(Some("overview")).unwrap());
        assert_eq!(default, guide(Some("  ")).unwrap());
        assert_eq!(default["topic"], "overview");
        assert_eq!(default["loop"].as_array().unwrap().len(), LOOP.len());
        assert_eq!(default["topics"].as_array().unwrap().len(), TOPICS.len());
    }

    #[test]
    fn every_topic_but_overview_is_a_section_and_resolves() {
        for topic in TOPICS.iter().skip(1) {
            let page = guide(Some(topic)).unwrap();
            assert_eq!(page["topic"], *topic);
            assert!(!page["summary"].as_str().unwrap().is_empty(), "{topic}");
            assert!(!page["tools"].as_array().unwrap().is_empty(), "{topic}");
        }
        assert_eq!(SECTIONS.len() + 1, TOPICS.len());
    }

    #[test]
    fn topic_names_ignore_ascii_case() {
        assert_eq!(guide(Some("Claims")).unwrap()["topic"], "claims");
    }

    #[test]
    fn an_unknown_topic_names_the_valid_ones() {
        let error = guide(Some("leases")).unwrap_err();
        assert!(error.contains("unknown topic `leases`"), "{error}");
        for topic in TOPICS {
            assert!(error.contains(topic), "{error} lacks {topic}");
        }
    }

    #[test]
    fn every_message_fits_the_summary_line() {
        for topic in TOPICS {
            let page = guide(Some(topic)).unwrap();
            let message = page["message"].as_str().unwrap();
            assert!(message.len() <= 160, "{topic}: {} chars", message.len());
        }
    }
}
