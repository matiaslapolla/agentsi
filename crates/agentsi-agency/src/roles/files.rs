//! Where the global role files are. The rule is the same for every program:
//!
//! 1. `<data root>/plugin/agents`, when that folder exists;
//! 2. else `$AGENTSI_PLUGIN_DIR/agents`, when the variable is set;
//! 3. else the role files that the build holds.
//!
//! The build holds the files of `plugin/agents/` of this repo. After the
//! switch-over the plugin stays in Claude Code and the Rust programs are on the
//! `PATH`, so a program has no checkout to read. Rule 3 needs no setup.

use std::path::{Path, PathBuf};

/// The role files that the build holds, by role name.
const BUILTIN: [(&str, &str); 5] = [
    ("pm", include_str!("../../../../plugin/agents/pm.md")),
    (
        "social",
        include_str!("../../../../plugin/agents/social.md"),
    ),
    ("dev", include_str!("../../../../plugin/agents/dev.md")),
    ("qa", include_str!("../../../../plugin/agents/qa.md")),
    ("scout", include_str!("../../../../plugin/agents/scout.md")),
];

/// Order of the global roster; any other global role follows alphabetically.
const GLOBAL_ORDER: [&str; 5] = ["pm", "social", "dev", "qa", "scout"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoleFiles {
    Dir(PathBuf),
    Builtin,
}

impl RoleFiles {
    pub fn at(root: &Path) -> RoleFiles {
        let own = root.join("plugin").join("agents");
        if own.exists() {
            return RoleFiles::Dir(own);
        }
        match std::env::var_os("AGENTSI_PLUGIN_DIR").filter(|v| !v.is_empty()) {
            Some(dir) => RoleFiles::Dir(PathBuf::from(dir).join("agents")),
            None => RoleFiles::Builtin,
        }
    }

    /// True when a file `<role>.md` exists.
    pub fn has(&self, role: &str) -> bool {
        match self {
            RoleFiles::Dir(dir) => dir.join(format!("{role}.md")).exists(),
            RoleFiles::Builtin => BUILTIN.iter().any(|(name, _)| *name == role),
        }
    }

    /// The text of `<role>.md`, or `None` when there is none.
    pub fn read(&self, role: &str) -> Option<String> {
        match self {
            RoleFiles::Dir(dir) => {
                let file = dir.join(format!("{role}.md"));
                file.exists()
                    .then(|| std::fs::read_to_string(file).unwrap_or_default())
            }
            RoleFiles::Builtin => BUILTIN
                .iter()
                .find(|(name, _)| *name == role)
                .map(|(_, text)| (*text).to_string()),
        }
    }

    /// What the message of an unknown role names as the missing file.
    pub fn describe(&self, role: &str) -> String {
        match self {
            RoleFiles::Dir(dir) => dir.join(format!("{role}.md")).display().to_string(),
            RoleFiles::Builtin => format!("built-in {role}.md"),
        }
    }

    /// The names of the global roles: the files without a leading `_`, in the
    /// order of the roster (pm, social, dev, qa, scout), then the others.
    pub fn names(&self) -> Result<Vec<String>, String> {
        let mut names: Vec<String> = match self {
            RoleFiles::Dir(dir) => std::fs::read_dir(dir)
                .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .filter(|f| f.ends_with(".md") && !f.starts_with('_'))
                .map(|f| f[..f.len() - 3].to_string())
                .collect(),
            RoleFiles::Builtin => BUILTIN.iter().map(|(n, _)| (*n).to_string()).collect(),
        };
        let rank = |n: &str| {
            GLOBAL_ORDER
                .iter()
                .position(|g| *g == n)
                .unwrap_or(GLOBAL_ORDER.len())
        };
        names.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_holds_the_five_roles_of_the_plugin() {
        let files = RoleFiles::Builtin;
        assert_eq!(
            files.names().unwrap(),
            ["pm", "social", "dev", "qa", "scout"]
        );
        assert!(files.read("pm").unwrap().contains("name: pm"));
        assert!(files.read("ghost").is_none());
        assert!(!files.has("ghost"));
    }

    #[test]
    fn the_data_root_folder_wins_and_other_roles_follow_the_roster_order() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join("plugin/agents");
        std::fs::create_dir_all(&agents).unwrap();
        for f in [
            "zed.md",
            "dev.md",
            "_shared.md",
            "pm.md",
            "alpha.md",
            "notes.txt",
        ] {
            std::fs::write(agents.join(f), "x").unwrap();
        }
        let files = RoleFiles::at(dir.path());
        assert_eq!(files, RoleFiles::Dir(agents.clone()));
        assert_eq!(files.names().unwrap(), ["pm", "dev", "alpha", "zed"]);
        assert!(files.has("zed") && !files.has("qa"));
        assert_eq!(
            files.describe("qa"),
            agents.join("qa.md").display().to_string()
        );
    }
}
