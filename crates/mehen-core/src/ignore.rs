//! Rules for what not to scan: a folder (usually a repo), a single project's
//! manifest, or a name pattern like `_spikes` or `**/fixtures/**`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::model::Inventory;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IgnoreKind {
    /// An absolute folder; everything inside is skipped.
    Folder,
    /// One manifest file (package.json, Cargo.toml, a .csproj, a `.github` folder).
    Project,
    /// A glob matched against paths relative to a watched folder. A pattern
    /// without `/` matches a folder or file of that name anywhere.
    Pattern,
}

impl IgnoreKind {
    pub fn as_str(self) -> &'static str {
        match self {
            IgnoreKind::Folder => "folder",
            IgnoreKind::Project => "project",
            IgnoreKind::Pattern => "pattern",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "folder" => Some(IgnoreKind::Folder),
            "project" => Some(IgnoreKind::Project),
            "pattern" => Some(IgnoreKind::Pattern),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnoreRule {
    pub id: i64,
    pub kind: IgnoreKind,
    pub value: String,
    pub note: Option<String>,
}

/// Windows paths compare case-insensitively and with either slash.
pub fn normalize(path: &str) -> String {
    path.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

fn is_within(path: &str, folder: &str) -> bool {
    path == folder || (path.starts_with(folder) && path.as_bytes().get(folder.len()) == Some(&b'\\'))
}

#[derive(Default, Clone)]
pub struct IgnoreSet {
    roots: Vec<String>,
    folders: Vec<(i64, String)>,
    projects: Vec<(i64, String)>,
    patterns: Option<GlobSet>,
    pattern_ids: Vec<i64>,
}

impl IgnoreSet {
    pub fn new(rules: &[IgnoreRule], roots: &[PathBuf]) -> Self {
        let mut set = IgnoreSet { roots: roots.iter().map(|r| normalize(&r.display().to_string())).collect(), ..Default::default() };
        let mut builder = GlobSetBuilder::new();
        for rule in rules {
            match rule.kind {
                IgnoreKind::Folder => set.folders.push((rule.id, normalize(&rule.value))),
                IgnoreKind::Project => set.projects.push((rule.id, normalize(&rule.value))),
                IgnoreKind::Pattern => {
                    let p = rule.value.trim().trim_matches('/').replace('\\', "/");
                    if p.is_empty() {
                        continue;
                    }
                    let variants = if p.contains('/') { vec![p.clone(), format!("{p}/**")] } else { vec![format!("**/{p}"), format!("**/{p}/**")] };
                    for v in variants {
                        if let Ok(glob) = GlobBuilder::new(&v).case_insensitive(true).literal_separator(true).build() {
                            builder.add(glob);
                            set.pattern_ids.push(rule.id);
                        }
                    }
                }
            }
        }
        set.patterns = builder.build().ok().filter(|g| !g.is_empty());
        set
    }

    fn relative<'a>(&self, path: &'a str) -> Option<&'a str> {
        self.roots.iter().find(|r| is_within(path, r)).map(|r| path[r.len()..].trim_start_matches('\\'))
    }

    /// Which rule, if any, excludes this path. Folder and pattern rules apply
    /// to everything beneath them; project rules only to the exact manifest.
    pub fn matching_rule(&self, path: &Path) -> Option<i64> {
        let p = normalize(&path.display().to_string());
        if let Some((id, _)) = self.projects.iter().find(|(_, m)| *m == p) {
            return Some(*id);
        }
        if let Some((id, _)) = self.folders.iter().find(|(_, f)| is_within(&p, f)) {
            return Some(*id);
        }
        let patterns = self.patterns.as_ref()?;
        let rel = self.relative(&p)?.replace('\\', "/");
        if rel.is_empty() {
            return None;
        }
        patterns.matches(&rel).first().map(|i| self.pattern_ids[*i])
    }

    pub fn is_empty(&self) -> bool {
        self.folders.is_empty() && self.projects.is_empty() && self.patterns.is_none()
    }

    /// Drops ignored projects from an existing result, and any advisory no
    /// remaining project is affected by. Used when a rule is added after a scan.
    pub fn apply(&self, inventory: &mut Inventory) {
        inventory.projects.retain(|p| self.matching_rule(Path::new(&p.manifest)).is_none() && self.matching_rule(Path::new(&p.dir)).is_none());
        let still_used: BTreeSet<&str> = inventory.projects.iter().flat_map(|p| p.dependencies.iter()).flat_map(|d| d.vulns.iter().map(String::as_str)).collect();
        inventory.vulnerabilities.retain(|v| still_used.contains(v.id.as_str()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: i64, kind: IgnoreKind, value: &str) -> IgnoreRule {
        IgnoreRule { id, kind, value: value.into(), note: None }
    }

    #[test]
    fn folder_project_and_pattern_rules() {
        let rules = [
            rule(1, IgnoreKind::Folder, "C:\\code\\opencode"),
            rule(2, IgnoreKind::Project, "C:/code/Mehen/package.json"),
            rule(3, IgnoreKind::Pattern, "_spikes"),
            rule(4, IgnoreKind::Pattern, "nuget-compass/fixtures"),
        ];
        let set = IgnoreSet::new(&rules, &[PathBuf::from("C:\\code")]);
        assert_eq!(set.matching_rule(Path::new("C:\\code\\opencode\\packages\\web")), Some(1));
        assert_eq!(set.matching_rule(Path::new("C:\\code\\opencode-other")), None);
        assert_eq!(set.matching_rule(Path::new("c:\\code\\mehen\\package.json")), Some(2));
        assert_eq!(set.matching_rule(Path::new("C:\\code\\Mehen\\src\\package.json")), None);
        assert_eq!(set.matching_rule(Path::new("C:\\code\\_spikes\\vscode-npm-gui\\package.json")), Some(3));
        assert_eq!(set.matching_rule(Path::new("C:\\code\\nuget-compass\\fixtures\\a\\a.csproj")), Some(4));
        assert_eq!(set.matching_rule(Path::new("C:\\code\\other\\fixtures\\a.csproj")), None);
    }
}
