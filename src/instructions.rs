//! The protocol a session receives when it connects (ADR-0038): the MCP
//! `instructions` every client puts in front of its model. The built-in
//! text is `instructions.md` next to this file; a repository that runs its
//! own protocol replaces it with [`OVERRIDE`], read on every handshake so
//! an edit needs no restart.

use std::path::Path;

/// The built-in protocol, served when the repository has no
/// [`OVERRIDE`].
pub const DEFAULT: &str = include_str!("instructions.md");

/// The repository's own protocol, relative to its root. When this file
/// exists and is not blank, it is served instead of [`DEFAULT`].
pub const OVERRIDE: &str = ".tirith/instructions.md";

/// The instructions the daemon serving `root` sends on connect.
pub fn load(root: &Path) -> String {
    std::fs::read_to_string(root.join(OVERRIDE))
        .ok()
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| DEFAULT.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_override_wins_unless_it_is_missing_or_blank() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), DEFAULT);
        let path = dir.path().join(OVERRIDE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "  \n").unwrap();
        assert_eq!(load(dir.path()), DEFAULT, "a blank file is no protocol");
        std::fs::write(&path, "# Ours\n\nClaim first.\n").unwrap();
        assert_eq!(load(dir.path()), "# Ours\n\nClaim first.\n");
    }
}
