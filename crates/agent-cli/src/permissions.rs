//! Answering an agent's permission prompts from [`Permissions`], so a run
//! never waits on a person.

use std::path::{Component, Path, PathBuf};

use crate::Permissions;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    Allow,
    /// Refused, with a reason a person can read.
    Deny(String),
}

impl Verdict {
    pub(crate) fn allowed(&self) -> bool {
        matches!(self, Verdict::Allow)
    }
}

/// What a tool does, as far as [`Permissions`] cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Read,
    Edit,
    Command,
    Network,
    /// Anything else (sub-agents, MCP tools, questions for the user) is
    /// refused: a tool this crate does not recognise cannot be judged safe.
    Other,
}

pub(crate) const NO_EDITS: &str = "Changing files is not allowed for this task";
pub(crate) const NO_COMMANDS: &str = "Running commands is not allowed for this task";
pub(crate) const NO_NETWORK: &str = "Using the internet is not allowed for this task";

pub(crate) fn outside(cwd: &Path) -> String {
    format!("Changing files outside {} is not allowed", cwd.display())
}

/// The verdict for one action. `paths` are the files an edit would touch;
/// an edit that names none is refused rather than assumed to stay inside.
pub(crate) fn judge(kind: Kind, name: &str, paths: &[String], cwd: &Path, permissions: Permissions) -> Verdict {
    match kind {
        Kind::Read => Verdict::Allow,
        Kind::Edit if !permissions.edit_files => Verdict::Deny(NO_EDITS.into()),
        Kind::Edit if paths.is_empty() || !paths.iter().all(|p| inside(cwd, Path::new(p))) => Verdict::Deny(outside(cwd)),
        Kind::Edit => Verdict::Allow,
        Kind::Command if permissions.run_commands => Verdict::Allow,
        Kind::Command => Verdict::Deny(NO_COMMANDS.into()),
        Kind::Network if permissions.run_commands => Verdict::Allow,
        Kind::Network => Verdict::Deny(NO_NETWORK.into()),
        Kind::Other => Verdict::Deny(format!("{name} is not available to this task")),
    }
}

/// Whether `path` (relative paths are taken from `cwd`) is `cwd` or inside
/// it. Compared after resolving `..` and, where the folders exist, links, so
/// `src/../../x` and a link pointing out of the folder are both outside.
pub(crate) fn inside(cwd: &Path, path: &Path) -> bool {
    let root = comparable(&resolve(cwd));
    let target = comparable(&resolve(&cwd.join(path)));
    target == root || target.starts_with(&format!("{root}/"))
}

/// `path` relative to `cwd` with forward slashes when inside it, otherwise
/// the full path with any `..` resolved. For showing to a person who knows
/// the project.
pub(crate) fn display_path(cwd: &Path, path: &str) -> String {
    let root = comparable(&resolve(cwd));
    let resolved = resolve(&cwd.join(path));
    let target = comparable(&resolved);
    match target.strip_prefix(&format!("{root}/")) {
        // Cut the original spelling at the same place, so the file keeps its
        // own casing; only the comparison was lowered.
        Some(_) => slashed(&resolved).chars().skip(root.chars().count() + 1).collect(),
        None if cfg!(windows) => slashed(&resolved).replace('/', "\\"),
        None => slashed(&resolved),
    }
}

/// Lexically resolved, then with the longest existing prefix replaced by its
/// real location.
fn resolve(path: &Path) -> PathBuf {
    let mut clean = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other),
        }
    }
    let mut existing = clean.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = existing.canonicalize() {
            return rest.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => return clean,
        }
    }
}

fn slashed(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    // `canonicalize` on Windows answers with a `\\?\` prefix the input lacked.
    let text = text.strip_prefix("//?/UNC/").map(|t| format!("//{t}")).or_else(|| text.strip_prefix("//?/").map(str::to_string)).unwrap_or(text);
    text.trim_end_matches('/').to_string()
}

fn comparable(path: &Path) -> String {
    let text = slashed(path);
    if cfg!(windows) { text.to_lowercase() } else { text }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Permissions {
        Permissions { edit_files: true, run_commands: true }
    }

    fn root() -> PathBuf {
        std::env::temp_dir().join("agent-cli-permissions-test")
    }

    #[test]
    fn reads_are_always_allowed_and_unknown_tools_never() {
        for permissions in [Permissions::default(), all()] {
            assert_eq!(judge(Kind::Read, "Read", &[], &root(), permissions), Verdict::Allow);
            assert!(!judge(Kind::Other, "mcp__mail__send", &[], &root(), permissions).allowed());
        }
    }

    #[test]
    fn edits_need_permission_and_must_stay_in_the_folder() {
        let inside_file = vec!["src/lib.rs".to_string()];
        assert_eq!(judge(Kind::Edit, "Edit", &inside_file, &root(), Permissions::default()), Verdict::Deny(NO_EDITS.into()));
        assert_eq!(judge(Kind::Edit, "Edit", &inside_file, &root(), all()), Verdict::Allow);
        let escaping = vec!["src/../../elsewhere.rs".to_string()];
        assert_eq!(judge(Kind::Edit, "Edit", &escaping, &root(), all()), Verdict::Deny(outside(&root())));
        assert!(!judge(Kind::Edit, "Edit", &[], &root(), all()).allowed(), "an edit that names no file is not assumed to stay inside");
        let one_outside = vec!["a.rs".to_string(), std::env::temp_dir().join("b.rs").to_string_lossy().into_owned()];
        assert!(!judge(Kind::Edit, "apply_patch", &one_outside, &root(), all()).allowed(), "every file is judged, not just the first");
    }

    #[test]
    fn commands_and_the_network_go_with_run_commands() {
        let edit_only = Permissions { edit_files: true, run_commands: false };
        assert_eq!(judge(Kind::Command, "Bash", &[], &root(), edit_only), Verdict::Deny(NO_COMMANDS.into()));
        assert_eq!(judge(Kind::Network, "WebFetch", &[], &root(), edit_only), Verdict::Deny(NO_NETWORK.into()));
        assert_eq!(judge(Kind::Command, "Bash", &[], &root(), all()), Verdict::Allow);
        assert_eq!(judge(Kind::Network, "WebFetch", &[], &root(), all()), Verdict::Allow);
    }

    #[test]
    fn a_sibling_folder_with_the_same_prefix_is_outside() {
        let root = root();
        let sibling = format!("{}2/x.rs", root.display());
        assert!(!inside(&root, Path::new(&sibling)));
        assert!(inside(&root, &root.join("deep").join("x.rs")));
        assert!(inside(&root, Path::new("x.rs")));
        assert!(!inside(&root, Path::new("..")));
    }

    #[test]
    fn windows_compares_without_case_or_slash_direction() {
        if !cfg!(windows) {
            return;
        }
        let root = PathBuf::from(r"C:\Work\Repo");
        assert!(inside(&root, Path::new("c:/work/repo/src/a.rs")));
        assert_eq!(display_path(&root, r"C:\work\REPO\src\Main.rs"), "src/Main.rs");
    }

    #[test]
    fn paths_are_shown_inside_the_project_when_they_are() {
        let root = root();
        assert_eq!(display_path(&root, &root.join("src").join("Lib.rs").to_string_lossy()), "src/Lib.rs");
        assert_eq!(display_path(&root, "src/a.rs"), "src/a.rs");
        let elsewhere = root.join("..").join("other.rs").to_string_lossy().into_owned();
        let shown = display_path(&root, &elsewhere);
        assert!(shown.ends_with("other.rs") && !shown.contains(".."), "outside the folder, shown in full with .. resolved: {shown}");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_folder_is_outside() {
        let root = std::env::temp_dir().join(format!("agent-cli-link-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let link = root.join("escape");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink("/", &link).unwrap();
        assert!(!inside(&root, Path::new("escape/etc/passwd")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
