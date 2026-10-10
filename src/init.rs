//! `tirith init` (ADR-0038): makes a repository ready for agents in one
//! command. It registers `tirith stdio` in `.mcp.json`, puts a short
//! block in the agents' instructions file that says the protocol arrives
//! with the MCP server, and can copy the built-in protocol to
//! [`instructions::OVERRIDE`] for a team
//! that wants to edit it. Every step is idempotent: running it again
//! changes nothing.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use thiserror::Error;

use crate::instructions;

/// The line that opens the block `init` keeps in the agents file.
pub const BEGIN: &str = "<!-- tirith:begin -->";
/// The line that closes it.
pub const END: &str = "<!-- tirith:end -->";

/// The block itself. Short on purpose: the protocol is served on connect
/// and would go stale here.
const BLOCK: &str = "## Tirith

Agents coordinate through the Tirith MCP server (`tirith stdio` in `.mcp.json`).
Its instructions on connect are the protocol: claim before editing, read
the brief, one lead per swarm. On `conflict`, do not edit.";

/// The MCP server entry `init` writes.
fn server_entry() -> Value {
    json!({ "type": "stdio", "command": "tirith", "args": ["stdio"] })
}

/// Why `init` stopped.
#[derive(Debug, Error)]
pub enum InitError {
    /// A file could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The error.
        #[source]
        source: std::io::Error,
    },
    /// `.mcp.json` exists but is not a JSON object.
    #[error("{path}: not a JSON object ({reason})")]
    NotAnObject {
        /// The file.
        path: PathBuf,
        /// What was wrong.
        reason: String,
    },
}

/// What `init` did, file by file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Files written or changed.
    pub written: Vec<PathBuf>,
    /// Files already as `init` would write them.
    pub unchanged: Vec<PathBuf>,
}

impl Report {
    fn record(&mut self, path: PathBuf, changed: bool) {
        if changed {
            self.written.push(path);
        } else {
            self.unchanged.push(path);
        }
    }
}

/// Prepares the repository at `root`: the block in `agents_file`
/// (created when missing), `tirith` in `.mcp.json`, and, with
/// `write_instructions`, the built-in protocol at
/// [`instructions::OVERRIDE`] unless that file already exists.
pub fn init(root: &Path, agents_file: &str, write_instructions: bool) -> Result<Report, InitError> {
    let mut report = Report::default();
    let agents = root.join(agents_file);
    let before = read_or_empty(&agents)?;
    let after = with_block(&before);
    write_if_changed(&agents, &before, &after, &mut report)?;

    let mcp = root.join(".mcp.json");
    let before = read_or_empty(&mcp)?;
    let after = with_server(&mcp, &before)?;
    write_if_changed(&mcp, &before, &after, &mut report)?;

    if write_instructions {
        let path = root.join(instructions::OVERRIDE);
        if path.exists() {
            report.record(path, false);
        } else {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|source| InitError::Io {
                    path: dir.to_path_buf(),
                    source,
                })?;
            }
            write(&path, instructions::DEFAULT)?;
            report.record(path, true);
        }
    }
    Ok(report)
}

/// `text` with the block in place: replaced between the markers when
/// present, appended otherwise.
fn with_block(text: &str) -> String {
    let block = format!("{BEGIN}\n{BLOCK}\n{END}\n");
    if let (Some(start), Some(end)) = (text.find(BEGIN), text.find(END))
        && start < end
    {
        let after = &text[end + END.len()..];
        let after = after.strip_prefix('\n').unwrap_or(after);
        return format!("{}{block}{after}", &text[..start]);
    }
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        block
    } else {
        format!("{trimmed}\n\n{block}")
    }
}

/// `text` as `.mcp.json` with `tirith` registered, every other server
/// kept.
fn with_server(path: &Path, text: &str) -> Result<String, InitError> {
    let mut root: Value = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(text).map_err(|e| InitError::NotAnObject {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?
    };
    let Some(object) = root.as_object_mut() else {
        return Err(InitError::NotAnObject {
            path: path.to_path_buf(),
            reason: "the top level is not an object".to_owned(),
        });
    };
    let servers = object.entry("mcpServers").or_insert_with(|| json!({}));
    let Some(servers) = servers.as_object_mut() else {
        return Err(InitError::NotAnObject {
            path: path.to_path_buf(),
            reason: "mcpServers is not an object".to_owned(),
        });
    };
    servers.insert("tirith".to_owned(), server_entry());
    // Pretty JSON never fails to serialize from a `Value`.
    Ok(format!(
        "{}\n",
        serde_json::to_string_pretty(&root).unwrap_or_default()
    ))
}

fn read_or_empty(path: &Path) -> Result<String, InitError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(source) => Err(InitError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn write(path: &Path, text: &str) -> Result<(), InitError> {
    std::fs::write(path, text).map_err(|source| InitError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn write_if_changed(
    path: &Path,
    before: &str,
    after: &str,
    report: &mut Report,
) -> Result<(), InitError> {
    let changed = before != after;
    if changed {
        write(path, after)?;
    }
    report.record(path.to_path_buf(), changed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_repository_gets_the_block_and_the_server_and_nothing_twice() {
        let dir = tempfile::tempdir().unwrap();
        let first = init(dir.path(), "AGENTS.md", false).unwrap();
        assert_eq!(first.written.len(), 2, "{first:?}");
        assert!(first.unchanged.is_empty(), "{first:?}");
        let agents = std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap();
        assert!(agents.starts_with(BEGIN), "{agents}");
        assert!(agents.ends_with(&format!("{END}\n")), "{agents}");
        let mcp: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap())
                .unwrap();
        assert_eq!(mcp["mcpServers"]["tirith"], server_entry());

        let second = init(dir.path(), "AGENTS.md", false).unwrap();
        assert!(second.written.is_empty(), "{second:?}");
        assert_eq!(second.unchanged.len(), 2, "{second:?}");
    }

    #[test]
    fn an_existing_block_is_replaced_in_place_and_other_servers_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("CLAUDE.md"),
            format!("# Rules\n\n{BEGIN}\nold text\n{END}\n\n## After\n"),
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".mcp.json"),
            r#"{"mcpServers":{"serena":{"command":"serena","args":["start-mcp-server"]}}}"#,
        )
        .unwrap();
        init(dir.path(), "CLAUDE.md", false).unwrap();
        let agents = std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert_eq!(agents.matches(BEGIN).count(), 1, "{agents}");
        assert!(!agents.contains("old text"), "{agents}");
        assert!(agents.starts_with("# Rules\n\n"), "{agents}");
        assert!(agents.ends_with("\n\n## After\n"), "{agents}");
        let mcp: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap())
                .unwrap();
        assert_eq!(mcp["mcpServers"]["serena"]["command"], "serena");
        assert_eq!(mcp["mcpServers"]["tirith"]["args"], json!(["stdio"]));
    }

    #[test]
    fn the_protocol_copy_is_written_once_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let first = init(dir.path(), "AGENTS.md", true).unwrap();
        let path = dir.path().join(instructions::OVERRIDE);
        assert!(first.written.contains(&path), "{first:?}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            instructions::DEFAULT
        );
        std::fs::write(&path, "# Ours\n").unwrap();
        let second = init(dir.path(), "AGENTS.md", true).unwrap();
        assert!(second.unchanged.contains(&path), "{second:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Ours\n");
    }

    #[test]
    fn a_broken_mcp_json_is_an_error_not_a_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".mcp.json"), "[1, 2]").unwrap();
        let error = init(dir.path(), "AGENTS.md", false).unwrap_err();
        assert!(matches!(error, InitError::NotAnObject { .. }), "{error}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap(),
            "[1, 2]"
        );
    }
}
