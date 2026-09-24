//! Dart and Flutter packages: `pubspec.yaml`, `pubspec.lock`, and rewriting
//! a version constraint in place.

use std::collections::HashMap;
use std::path::Path;

use yaml_rust2::{Yaml, YamlLoader};

/// One entry under `dependencies` or `dev_dependencies`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// The constraint as written: `^1.2.0`, `>=1.0.0 <2.0.0`, `any`, or empty.
    pub constraint: String,
    pub dev: bool,
    /// Not from pub.dev: the Flutter SDK, a folder, git, or another server.
    pub local: Option<&'static str>,
}

#[derive(Debug, Default)]
pub struct Pubspec {
    pub name: Option<String>,
    /// The `environment: sdk:` constraint.
    pub sdk: Option<String>,
    /// Depends on the Flutter SDK, so `flutter` (not `dart`) runs its commands.
    pub flutter: bool,
    pub entries: Vec<Entry>,
}

fn text(y: &Yaml) -> Option<String> {
    match y {
        Yaml::String(s) => Some(s.clone()),
        Yaml::Real(s) => Some(s.clone()),
        Yaml::Integer(i) => Some(i.to_string()),
        _ => None,
    }
}

pub fn parse(source: &str) -> anyhow::Result<Pubspec> {
    let docs = YamlLoader::load_from_str(source)?;
    let Some(doc) = docs.first() else { return Ok(Pubspec::default()) };
    let mut spec = Pubspec { name: text(&doc["name"]), sdk: text(&doc["environment"]["sdk"]), ..Default::default() };
    for (section, dev) in [("dependencies", false), ("dev_dependencies", true)] {
        let Some(map) = doc[section].as_hash() else { continue };
        for (key, value) in map {
            let Some(name) = key.as_str().map(str::to_string) else { continue };
            let (constraint, local) = match value {
                Yaml::Hash(_) if !value["sdk"].is_badvalue() => {
                    spec.flutter |= value["sdk"].as_str() == Some("flutter");
                    (String::new(), Some("part of the Flutter SDK"))
                }
                Yaml::Hash(_) if !value["path"].is_badvalue() => (String::new(), Some("local path")),
                Yaml::Hash(_) if !value["git"].is_badvalue() => (String::new(), Some("git dependency")),
                Yaml::Hash(_) => {
                    let hosted = value["hosted"].as_str().map(str::to_string).or_else(|| text(&value["hosted"]["url"]));
                    let elsewhere = hosted.is_some_and(|url| !url.contains("pub.dev") && !url.contains("pub.dartlang.org"));
                    (text(&value["version"]).unwrap_or_default(), elsewhere.then_some("hosted on another package server"))
                }
                other => (text(other).unwrap_or_default(), None),
            };
            spec.entries.push(Entry { name, constraint, dev, local });
        }
    }
    Ok(spec)
}

/// Installed versions by package name, from `pubspec.lock`.
pub fn read_lock(path: &Path) -> HashMap<String, String> {
    let Ok(source) = std::fs::read_to_string(path) else { return HashMap::new() };
    let Ok(docs) = YamlLoader::load_from_str(&source) else { return HashMap::new() };
    let Some(packages) = docs.first().and_then(|d| d["packages"].as_hash()) else { return HashMap::new() };
    packages.iter().filter_map(|(k, v)| Some((k.as_str()?.to_string(), text(&v["version"])?))).collect()
}

fn parts(v: &str) -> Vec<u64> {
    let core = v.trim().trim_start_matches(['^', '>', '=', '<', '~']).split(['+', '-']).next().unwrap_or("");
    core.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// The constraint moved to allow `to`, keeping its style: `^1.1.0` ->
/// `^1.4.2`, `>=1.0.0 <2.0.0` -> `>=2.1.0 <3.0.0`, `1.2.3` -> `1.4.2`. `any`
/// and an empty constraint already allow everything and stay as written.
pub fn rewrite(constraint: &str, to: &str) -> String {
    let c = constraint.trim();
    if c.is_empty() || c == "any" {
        return constraint.to_string();
    }
    if c.starts_with('^') {
        return format!("^{to}");
    }
    if !c.contains(' ') && !c.starts_with(['>', '<', '=']) {
        return to.to_string();
    }
    let target = parts(to);
    c.split_whitespace()
        .map(|clause| {
            if clause.starts_with(">=") || clause.starts_with('>') {
                format!(">={to}")
            } else if clause.starts_with('<') && !crate::compat::semver_satisfies(to, clause) {
                // Dart ranges stop before the next breaking release: 0.x counts its minor.
                if target.first() == Some(&0) { format!("<0.{}.0", target.get(1).unwrap_or(&0) + 1) } else { format!("<{}.0.0", target[0] + 1) }
            } else {
                clause.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Rewrites `name`'s constraint in `pubspec.yaml` text, both as
/// `  name: ^1.2.0` and as a nested `    version: ^1.2.0`, keeping quotes,
/// comments and layout. Returns the text and the constraint before and after.
pub fn set_constraint(source: &str, name: &str, to: &str) -> Option<(String, String, String)> {
    let mut out = String::with_capacity(source.len());
    let mut section = "";
    let mut entry_indent: Option<usize> = None;
    let mut found: Option<(String, String)> = None;
    for raw in source.split_inclusive('\n') {
        let line = raw.trim_end_matches(['\r', '\n']);
        let indent = line.len() - line.trim_start().len();
        let content = line.split(" #").next().unwrap_or(line).trim();
        if indent == 0 && !content.is_empty() && !content.starts_with('#') {
            section = content.trim_end_matches(':');
            entry_indent = None;
        }
        let in_deps = matches!(section, "dependencies" | "dev_dependencies");
        let mut rewrite_value = |key: &str| -> Option<String> {
            let (k, value) = content.split_once(':')?;
            if k.trim() != key {
                return None;
            }
            let value = value.trim();
            let bare = value.trim_matches(['\'', '"']);
            if bare.is_empty() || found.is_some() {
                return None;
            }
            let new = rewrite(bare, to);
            if new == bare {
                return None;
            }
            let at = line.find(bare)?;
            found = Some((bare.to_string(), new.clone()));
            Some(format!("{}{}{}", &line[..at], new, &raw[at + bare.len()..]))
        };
        if in_deps && indent > 0 && entry_indent.is_none_or(|i| indent <= i) {
            entry_indent = content.split_once(':').filter(|(k, _)| k.trim() == name).map(|_| indent);
            if entry_indent.is_some() {
                if let Some(next) = rewrite_value(name) {
                    out.push_str(&next);
                    entry_indent = None;
                    continue;
                }
            }
        } else if in_deps && entry_indent.is_some_and(|i| indent > i) {
            if let Some(next) = rewrite_value("version") {
                out.push_str(&next);
                continue;
            }
        }
        out.push_str(raw);
    }
    found.map(|(before, after)| (out, before, after))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBSPEC: &str = "name: absorb\nversion: 1.10.0+264\n\nenvironment:\n  sdk: '>=3.10.0 <4.0.0'\n\ndependencies:\n  flutter:\n    sdk: flutter\n  intl: any\n\n  # Networking\n  http: ^1.1.0\n  just_audio:\n    path: packages/just_audio\n  shared:\n    version: \">=2.0.0 <3.0.0\"\n    hosted: https://pub.dev\n  private_pkg:\n    hosted:\n      url: https://pub.example.com\n    version: ^1.0.0\n\ndev_dependencies:\n  flutter_lints: 4.0.0 # pinned\n";

    #[test]
    fn reads_entries_and_their_sources() {
        let p = parse(PUBSPEC).unwrap();
        assert_eq!(p.name.as_deref(), Some("absorb"));
        assert_eq!(p.sdk.as_deref(), Some(">=3.10.0 <4.0.0"));
        assert!(p.flutter);
        let by = |n: &str| p.entries.iter().find(|e| e.name == n).unwrap().clone();
        assert_eq!(by("http").constraint, "^1.1.0");
        assert_eq!(by("intl").constraint, "any");
        assert_eq!(by("just_audio").local, Some("local path"));
        assert_eq!(by("flutter").local, Some("part of the Flutter SDK"));
        assert_eq!((by("shared").constraint.as_str(), by("shared").local), (">=2.0.0 <3.0.0", None));
        assert_eq!(by("private_pkg").local, Some("hosted on another package server"));
        assert!(by("flutter_lints").dev);
    }

    #[test]
    fn rewrites_keep_the_style() {
        assert_eq!(rewrite("^1.1.0", "1.4.2"), "^1.4.2");
        assert_eq!(rewrite(">=1.0.0 <2.0.0", "2.1.0"), ">=2.1.0 <3.0.0");
        assert_eq!(rewrite(">=0.3.0 <0.4.0", "0.5.1"), ">=0.5.1 <0.6.0");
        assert_eq!(rewrite("4.0.0", "5.0.0"), "5.0.0");
        assert_eq!(rewrite("any", "9.0.0"), "any");
    }

    #[test]
    fn edits_inline_and_nested_constraints_in_place() {
        let (out, before, after) = set_constraint(PUBSPEC, "http", "1.4.0").unwrap();
        assert_eq!((before.as_str(), after.as_str()), ("^1.1.0", "^1.4.0"));
        assert!(out.contains("  # Networking\n  http: ^1.4.0\n"));
        let (out, _, after) = set_constraint(PUBSPEC, "shared", "3.2.0").unwrap();
        assert_eq!(after, ">=3.2.0 <4.0.0");
        assert!(out.contains("    version: \">=3.2.0 <4.0.0\"\n"), "{out}");
        let (out, _, _) = set_constraint(PUBSPEC, "flutter_lints", "5.0.0").unwrap();
        assert!(out.contains("  flutter_lints: 5.0.0 # pinned\n"));
        assert!(set_constraint(PUBSPEC, "intl", "0.20.0").is_none(), "any has nothing to rewrite");
    }
}
