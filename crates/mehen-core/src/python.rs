//! Python packages: requirement lines (PEP 508), version specifiers
//! (PEP 440, plus Poetry's `^` and `~`), lockfiles, and rewriting the version
//! a requirement asks for.

use std::collections::HashMap;
use std::path::Path;

/// PyPI treats `Typing_Extensions` and `typing-extensions` as one name.
pub fn normalize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut dash = false;
    for c in name.chars() {
        if matches!(c, '-' | '_' | '.') {
            dash = true;
            continue;
        }
        if dash && !out.is_empty() {
            out.push('-');
        }
        dash = false;
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// One requirement: `requests[socks]>=2.28,<3 ; python_version >= "3.8"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Req {
    pub name: String,
    /// The version part as written, without brackets or parentheses: `>=2.28,<3`.
    pub spec: String,
    /// Installed straight from a URL or a folder (`name @ https://...`).
    pub direct: bool,
}

/// A requirement line or string, or `None` for options (`-r other.txt`),
/// bare URLs and paths, and blank or comment lines.
pub fn parse_requirement(line: &str) -> Option<Req> {
    let line = line.split(" #").next()?.trim();
    let line = if line.starts_with('#') { "" } else { line };
    let first = line.chars().next()?;
    if !first.is_ascii_alphanumeric() || line.contains("://") && !line.contains('@') {
        return None;
    }
    let body = line.split(';').next()?.trim();
    let end = body.find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))).unwrap_or(body.len());
    let name = body[..end].to_string();
    let mut rest = body[end..].trim_start();
    // After a name comes extras, a version, `@ url` or nothing; `git+https://...` is none of those.
    if !rest.is_empty() && !rest.starts_with(['[', '@', '(', '<', '>', '=', '!', '~', '^']) {
        return None;
    }
    if rest.starts_with('[') {
        rest = rest.find(']').map(|i| rest[i + 1..].trim_start()).unwrap_or("");
    }
    if let Some(url) = rest.strip_prefix('@') {
        return (!url.trim().is_empty()).then(|| Req { name, spec: String::new(), direct: true });
    }
    let spec = rest.trim().trim_start_matches('(').trim_end_matches(')').trim().to_string();
    Some(Req { name, spec, direct: false })
}

/// The numeric release of a version: `1.2.3` from `1.2.3`, `v1.2.3` or
/// `1.2.3+local`. Pre-, post- and dev-releases give `None`.
pub fn release(version: &str) -> Option<Vec<u64>> {
    let v = version.trim().trim_start_matches(['v', 'V']);
    let v = v.split('+').next()?;
    let v = v.rsplit('!').next()?;
    v.split('.').map(|p| p.parse::<u64>().ok()).collect()
}

fn cmp_release(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    let len = a.len().max(b.len());
    (0..len).map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0))).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal)
}

const OPS: [&str; 10] = ["===", "~=", "==", "!=", "<=", ">=", "<", ">", "^", "~"];

/// Splits one clause (`>=2.28`) into its operator and version. A bare
/// version (Poetry's exact pin) reads as `==`.
fn clause(c: &str) -> (&str, &str) {
    let c = c.trim();
    for op in OPS {
        if let Some(v) = c.strip_prefix(op) {
            return (op, v.trim());
        }
    }
    ("==", c)
}

/// Whether `version` meets every clause of `spec`. Anything unreadable
/// counts as met, so it never blocks an update.
pub fn satisfies(version: &str, spec: &str) -> bool {
    let Some(v) = release(version) else { return true };
    spec.split(',').map(str::trim).filter(|c| !c.is_empty() && *c != "*").all(|c| {
        let (op, want) = clause(c);
        if let Some(prefix) = want.strip_suffix(".*") {
            let Some(p) = release(prefix) else { return true };
            let same = v.len() >= p.len() && v[..p.len()] == p[..];
            return if op == "!=" { !same } else { same };
        }
        let Some(w) = release(want) else { return true };
        let ord = cmp_release(&v, &w);
        match op {
            "==" | "===" => ord.is_eq(),
            "!=" => !ord.is_eq(),
            "<=" => ord.is_le(),
            ">=" => ord.is_ge(),
            "<" => ord.is_lt(),
            ">" => ord.is_gt(),
            // ~=1.4.2 means >=1.4.2 and the same 1.4 line; ~=1.4 the same 1 line.
            "~=" => ord.is_ge() && w.len() >= 2 && v.len() >= w.len() - 1 && v[..w.len() - 1] == w[..w.len() - 1],
            // Poetry: ^1.2 means >=1.2,<2; ^0.3 means >=0.3,<0.4.
            "^" => {
                let fixed = w.iter().position(|&x| x != 0).unwrap_or(w.len() - 1) + 1;
                ord.is_ge() && v.len() >= fixed && v[..fixed] == w[..fixed]
            }
            // Poetry: ~1.2 means >=1.2,<1.3; ~1 means >=1,<2.
            "~" => {
                let fixed = if w.len() >= 2 { 2 } else { 1 };
                ord.is_ge() && v.len() >= fixed && v[..fixed] == w[..fixed]
            }
            _ => true,
        }
    })
}

fn render(parts: &[u64]) -> String {
    parts.iter().map(u64::to_string).collect::<Vec<_>>().join(".")
}

/// `to`, cut to as many parts as `like` has: `~=1.4` with 2.3.1 gives `2.3`.
fn same_precision(like: &str, to: &str) -> String {
    match (release(like), release(to)) {
        (Some(l), Some(t)) if t.len() > l.len() => render(&t[..l.len()]),
        _ => to.to_string(),
    }
}

/// The spec moved to allow `to`, keeping its style: `==2.31.0` -> `==2.32.3`,
/// `>=2.28,<3` -> `>=4.0.1,<5`, `^1.2` -> `^2.0`, `~=1.4` -> `~=2.3`. An
/// unversioned `*` or empty spec stays as it is.
pub fn rewrite_spec(spec: &str, to: &str) -> String {
    let trimmed = spec.trim();
    if trimmed.is_empty() || trimmed == "*" {
        return spec.to_string();
    }
    let Some(target) = release(to) else { return spec.to_string() };
    let sep = if spec.contains(", ") { ", " } else { "," };
    let mut clauses: Vec<String> = Vec::new();
    for c in trimmed.split(',').map(str::trim).filter(|c| !c.is_empty()) {
        let (op, want) = clause(c);
        let written = |op: &str, v: String| format!("{op}{v}");
        let next = match op {
            "==" | "===" if want.ends_with(".*") => {
                let prefix_len = release(want.trim_end_matches(".*")).map_or(1, |p| p.len()).min(target.len());
                Some(written(op, format!("{}.*", render(&target[..prefix_len]))))
            }
            // A bare Poetry pin has no operator to keep.
            "==" if !c.starts_with("==") => Some(to.to_string()),
            "==" | "===" | ">=" => Some(written(op, to.to_string())),
            ">" => Some(written(">=", to.to_string())),
            "~=" | "^" | "~" => Some(written(op, same_precision(want, to))),
            "<" | "<=" if !satisfies(to, c) => Some(written(op, (target[0] + 1).to_string())),
            "!=" if satisfies(want, &format!("=={to}")) => None,
            _ => Some(c.to_string()),
        };
        clauses.extend(next);
    }
    clauses.join(sep)
}

/// The lowest Python a project supports: `3.10` from `>=3.10`, `^3.10`,
/// `~=3.10`, `3.10.*` or a `.python-version` line like `3.12.1`.
pub fn lowest_python(spec: &str) -> Option<String> {
    spec.split(',').map(str::trim).find_map(|c| {
        let (op, want) = clause(c);
        match op {
            ">=" | "~=" | "^" | "~" | "==" | "===" => {
                let want = want.trim_end_matches(".*");
                release(want).map(|_| want.to_string())
            }
            _ => None,
        }
    })
}

/// Installed versions by normalized name, from whichever lockfile is given:
/// `uv.lock`, `poetry.lock` and `pdm.lock` list `[[package]]` tables;
/// `Pipfile.lock` is JSON with `==` pins.
pub fn read_lock(path: &Path) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(path) else { return HashMap::new() };
    if path.file_name().is_some_and(|n| n == "Pipfile.lock") {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return HashMap::new() };
        return ["default", "develop"]
            .iter()
            .filter_map(|section| json[section].as_object())
            .flat_map(|packages| packages.iter())
            .filter_map(|(name, pkg)| pkg["version"].as_str().map(|v| (normalize(name), v.trim_start_matches("==").to_string())))
            .collect();
    }
    let Ok(doc) = toml::from_str::<toml::Table>(&text) else { return HashMap::new() };
    doc.get("package")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter_map(|pkg| Some((normalize(pkg.get("name")?.as_str()?), pkg.get("version")?.as_str()?.to_string())))
        .collect()
}

/// The tool that owns a Python project's lockfile, by the lockfile next to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Uv,
    Poetry,
    Pdm,
    Pipenv,
}

impl Manager {
    pub const LOCKS: [(&'static str, Manager); 4] = [("uv.lock", Manager::Uv), ("poetry.lock", Manager::Poetry), ("pdm.lock", Manager::Pdm), ("Pipfile.lock", Manager::Pipenv)];

    /// The lockfile and its tool for a manifest: `Pipfile` pairs only with
    /// `Pipfile.lock`, `pyproject.toml` with the others. `requirements.txt` has none.
    pub fn for_manifest(manifest: &Path) -> Option<(Manager, std::path::PathBuf)> {
        let dir = manifest.parent()?;
        let file = manifest.file_name()?.to_str()?;
        Self::LOCKS
            .iter()
            .filter(|(_, m)| match file {
                "Pipfile" => *m == Manager::Pipenv,
                "pyproject.toml" => *m != Manager::Pipenv,
                _ => false,
            })
            .map(|(lock, m)| (*m, dir.join(lock)))
            .find(|(_, path)| path.is_file())
    }

    pub fn program(self) -> &'static str {
        match self {
            Manager::Uv => "uv",
            Manager::Poetry => "poetry",
            Manager::Pdm => "pdm",
            Manager::Pipenv => "pipenv",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_compare_the_way_pypi_does() {
        assert_eq!(normalize("Typing_Extensions"), "typing-extensions");
        assert_eq!(normalize("zope.interface"), "zope-interface");
        assert_eq!(normalize("a__b--c"), "a-b-c");
    }

    #[test]
    fn requirement_lines() {
        let r = parse_requirement("requests[socks, security]>=2.28,<3 ; python_version >= \"3.8\"  # http").unwrap();
        assert_eq!((r.name.as_str(), r.spec.as_str(), r.direct), ("requests", ">=2.28,<3", false));
        assert_eq!(parse_requirement("Django==4.2.7").unwrap().spec, "==4.2.7");
        assert_eq!(parse_requirement("flask").unwrap().spec, "");
        assert_eq!(parse_requirement("pytest (>=7.0)").unwrap().spec, ">=7.0");
        assert!(parse_requirement("mylib @ https://example.com/mylib.whl").unwrap().direct);
        for skipped in ["-r base.txt", "--index-url https://x", "# comment", "", "git+https://github.com/a/b.git", "git+https://git@github.com/a/b.git", "./local/pkg"] {
            assert_eq!(parse_requirement(skipped), None, "{skipped}");
        }
    }

    #[test]
    fn specifiers() {
        assert!(satisfies("2.31.0", ">=2.28,<3"));
        assert!(!satisfies("3.0.0", ">=2.28,<3"));
        assert!(satisfies("1.4.9", "~=1.4.2") && !satisfies("1.5.0", "~=1.4.2"));
        assert!(satisfies("1.9", "~=1.4") && !satisfies("2.0", "~=1.4"));
        assert!(satisfies("1.9.0", "^1.2") && !satisfies("2.0.0", "^1.2"));
        assert!(satisfies("0.3.9", "^0.3") && !satisfies("0.4.0", "^0.3"));
        assert!(satisfies("3.12", ">=3.10") && !satisfies("3.9", ">=3.10"));
        assert!(satisfies("3.10.4", "==3.10.*") && !satisfies("3.11.0", "==3.10.*"));
        assert!(!satisfies("3.8", ">=3.9, !=3.9.0"));
        assert!(satisfies("1.0", ">=weird"), "unreadable never blocks");
    }

    #[test]
    fn rewrites_keep_the_style() {
        assert_eq!(rewrite_spec("==2.31.0", "2.32.3"), "==2.32.3");
        assert_eq!(rewrite_spec(">=2.28,<3", "4.0.1"), ">=4.0.1,<5");
        assert_eq!(rewrite_spec(">=2.28, <3", "2.32.3"), ">=2.32.3, <3");
        assert_eq!(rewrite_spec("~=1.4", "2.3.1"), "~=2.3");
        assert_eq!(rewrite_spec("^1.2", "2.0.4"), "^2.0");
        assert_eq!(rewrite_spec("1.2.3", "1.4.0"), "1.4.0");
        assert_eq!(rewrite_spec("==1.*", "2.5.0"), "==2.*");
        assert_eq!(rewrite_spec(">=1.0,!=2.0.1", "2.0.1"), ">=2.0.1");
        assert_eq!(rewrite_spec("*", "9.9.9"), "*");
    }

    #[test]
    fn lowest_python_from_any_spelling() {
        assert_eq!(lowest_python(">=3.10").as_deref(), Some("3.10"));
        assert_eq!(lowest_python("^3.11").as_deref(), Some("3.11"));
        assert_eq!(lowest_python(">=3.9,<4").as_deref(), Some("3.9"));
        assert_eq!(lowest_python("3.12.1").as_deref(), Some("3.12.1"));
        assert_eq!(lowest_python("<4"), None);
    }

    #[test]
    fn reads_lockfiles() {
        let dir = std::env::temp_dir().join("mehen-python-locks");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("uv.lock"), "version = 1\n\n[[package]]\nname = \"Requests\"\nversion = \"2.31.0\"\n\n[[package]]\nname = \"typing_extensions\"\nversion = \"4.9.0\"\n").unwrap();
        std::fs::write(dir.join("Pipfile.lock"), r#"{"default": {"flask": {"version": "==3.0.0"}}, "develop": {"pytest": {"version": "==8.0.0"}}}"#).unwrap();
        let uv = read_lock(&dir.join("uv.lock"));
        assert_eq!(uv.get("requests").map(String::as_str), Some("2.31.0"));
        assert_eq!(uv.get("typing-extensions").map(String::as_str), Some("4.9.0"));
        let pipenv = read_lock(&dir.join("Pipfile.lock"));
        assert_eq!(pipenv.get("pytest").map(String::as_str), Some("8.0.0"));
    }
}
