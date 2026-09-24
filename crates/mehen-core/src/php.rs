//! PHP packages: Composer constraints, Packagist's compact version lists,
//! and `composer.lock`.

use std::collections::HashMap;
use std::path::Path;

/// A real package (`vendor/name`), not a platform requirement like `php`,
/// `ext-json` or `composer-plugin-api`.
pub fn is_package(name: &str) -> bool {
    name.contains('/')
}

fn release(v: &str) -> Option<Vec<u64>> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let v = v.split(['-', '@', '+']).next()?;
    v.split('.').map(|p| p.parse::<u64>().ok()).collect()
}

fn render(parts: &[u64]) -> String {
    parts.iter().map(u64::to_string).collect::<Vec<_>>().join(".")
}

/// `to` cut to as many parts as `like`: `^2.0` with 3.1.4 gives `3.1`.
fn same_precision(like: &str, to: &str) -> String {
    match (release(like), release(to)) {
        (Some(l), Some(t)) if t.len() > l.len() => render(&t[..l.len()]),
        _ => to.trim_start_matches(['v', 'V']).to_string(),
    }
}

/// A Composer constraint in npm's range syntax, which accepts the same
/// operators: `,` means "and", and a single `|` means "or".
pub fn as_npm_range(constraint: &str) -> String {
    let c = constraint.split('@').next().unwrap_or(constraint);
    c.replace("||", "|").replace('|', "||").replace(',', " ")
}

/// The constraint moved to allow `to`, keeping its style: `^2.0` -> `^3.1`,
/// `~2.1.0` -> `~3.1.4`, `2.1.*` -> `3.1.*`, `>=1.0 <2.0` -> `>=3.1.4 <4.0`,
/// `v1.2.3` -> `v3.1.4`. An "or" list becomes its last alternative moved on;
/// a stability flag (`@stable`) stays.
pub fn rewrite(constraint: &str, to: &str) -> String {
    let (body, flag) = match constraint.split_once('@') {
        Some((b, f)) => (b.trim(), format!("@{f}")),
        None => (constraint.trim(), String::new()),
    };
    if body.is_empty() || body == "*" {
        return constraint.to_string();
    }
    let Some(target) = release(to) else { return constraint.to_string() };
    let to = to.trim_start_matches(['v', 'V']);
    let last = body.split('|').rfind(|a| !a.trim().is_empty()).unwrap_or(body).trim();
    let sep = if last.contains(',') { "," } else { " " };
    let clauses: Vec<String> = last
        .split([',', ' '])
        .filter(|c| !c.is_empty())
        .map(|c| {
            if let Some(v) = c.strip_prefix('^') {
                format!("^{}", same_precision(v, to))
            } else if let Some(v) = c.strip_prefix('~') {
                format!("~{}", same_precision(v, to))
            } else if let Some(prefix) = c.strip_suffix(".*") {
                let keep = release(prefix).map_or(1, |p| p.len()).min(target.len());
                format!("{}.*", render(&target[..keep]))
            } else if c.starts_with(">=") || c.starts_with('>') {
                format!(">={to}")
            } else if c.starts_with('<') && !crate::compat::semver_satisfies(to, c) {
                format!("{}{}.0", if c.starts_with("<=") { "<=" } else { "<" }, target[0] + 1)
            } else if c.starts_with(['<', '!']) {
                c.to_string()
            } else {
                let v = if c.starts_with(['v', 'V']) { "v" } else { "" };
                format!("{v}{to}")
            }
        })
        .collect();
    format!("{}{flag}", clauses.join(sep))
}

/// Packagist's `p2` document lists versions newest first, each naming only
/// what changed from the one before (`"__unset"` removes a key). Returns
/// (version, PHP constraint) pairs with those gaps filled in.
pub fn expand_versions(entries: &[serde_json::Value]) -> Vec<(String, Option<String>)> {
    let mut require: Option<serde_json::Value> = None;
    let mut out = Vec::new();
    for entry in entries {
        match &entry["require"] {
            serde_json::Value::Null => {}
            serde_json::Value::String(s) if s == "__unset" => require = None,
            value => require = Some(value.clone()),
        }
        let Some(version) = entry["version"].as_str() else { continue };
        let php = require.as_ref().and_then(|r| r["php"].as_str()).map(str::to_string);
        out.push((version.to_string(), php));
    }
    out
}

/// Installed versions by package name, from `composer.lock`.
pub fn read_lock(path: &Path) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(path) else { return HashMap::new() };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return HashMap::new() };
    ["packages", "packages-dev"]
        .iter()
        .filter_map(|k| json[k].as_array())
        .flatten()
        .filter_map(|p| Some((p["name"].as_str()?.to_string(), p["version"].as_str()?.to_string())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rewrites_keep_the_style() {
        assert_eq!(rewrite("^2.0", "3.1.4"), "^3.1");
        assert_eq!(rewrite("^2.0.1", "3.1.4"), "^3.1.4");
        assert_eq!(rewrite("~2.1.0", "3.1.4"), "~3.1.4");
        assert_eq!(rewrite("2.1.*", "3.1.4"), "3.1.*");
        assert_eq!(rewrite(">=1.0 <2.0", "3.1.4"), ">=3.1.4 <4.0");
        assert_eq!(rewrite(">=1.0,<2.0", "1.5.0"), ">=1.5.0,<2.0");
        assert_eq!(rewrite("^7.2 || ^8.0", "9.1.0"), "^9.1");
        assert_eq!(rewrite("v1.2.3", "v3.1.4"), "v3.1.4");
        assert_eq!(rewrite("^1.0@stable", "2.0.1"), "^2.0@stable");
        assert_eq!(rewrite("*", "2.0.0"), "*");
    }

    #[test]
    fn composer_ranges_read_as_npm_ranges() {
        assert!(crate::compat::semver_satisfies("8.2.0", &as_npm_range("^7.4 || ^8.1")));
        assert!(!crate::compat::semver_satisfies("8.0.0", &as_npm_range(">=8.1,<9")));
        assert!(crate::compat::semver_satisfies("7.4.3", &as_npm_range("^7.2|^8.0")));
    }

    #[test]
    fn packagist_versions_inherit_from_the_one_before() {
        let entries = vec![
            json!({"version": "3.12.0", "require": {"php": ">=8.1"}}),
            json!({"version": "3.11.0"}),
            json!({"version": "2.9.0", "require": {"php": ">=7.2"}}),
            json!({"version": "1.0.0", "require": "__unset"}),
        ];
        let expanded = expand_versions(&entries);
        assert_eq!(expanded[1], ("3.11.0".to_string(), Some(">=8.1".to_string())));
        assert_eq!(expanded[2].1.as_deref(), Some(">=7.2"));
        assert_eq!(expanded[3].1, None);
        assert!(is_package("monolog/monolog") && !is_package("php") && !is_package("ext-json"));
    }
}
