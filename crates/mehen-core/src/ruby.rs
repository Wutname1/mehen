//! Ruby gems: `Gemfile` lines, `Gemfile.lock`, and RubyGems requirements
//! (`~> 2.7`, `>= 1.2, < 3`).

use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gem {
    pub name: String,
    /// The version requirements as written, joined: `~> 7.1, >= 7.1.3`.
    pub requirement: String,
    pub dev: bool,
    /// Installed from a folder or git, not rubygems.org.
    pub local: Option<&'static str>,
}

/// Quoted strings at the start of a `gem` line's arguments, up to the first
/// option like `require: false`.
fn leading_strings(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in args.split(',') {
        let p = part.trim();
        let quoted = (p.starts_with('\'') && p.ends_with('\'') || p.starts_with('"') && p.ends_with('"')) && p.len() >= 2;
        if !quoted {
            break;
        }
        out.push(p[1..p.len() - 1].to_string());
    }
    out
}

/// The `gem` lines of a Gemfile. Groups are followed through `group ... do`
/// blocks and `group:` options; development and test groups count as dev.
pub fn parse_gemfile(text: &str) -> Vec<Gem> {
    let mut blocks: Vec<bool> = Vec::new();
    let mut gems = Vec::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line == "end" {
            blocks.pop();
            continue;
        }
        let opens = line.ends_with(" do") || line.contains(" do |");
        if let Some(rest) = line.strip_prefix("gem ").or_else(|| line.strip_prefix("gem\t")) {
            let rest = rest.trim();
            let Some(quote) = rest.chars().next().filter(|c| *c == '\'' || *c == '"') else { continue };
            let Some(end) = rest[1..].find(quote) else { continue };
            let name = rest[1..=end].to_string();
            let args = rest[end + 2..].trim_start_matches(',').trim();
            let local = if ["path:", ":path", "git:", ":git", "github:", ":github"].iter().any(|k| args.contains(k)) { Some("installed from a folder or git") } else { None };
            let inline_dev = args.contains("group:") && (args.contains(":development") || args.contains(":test"));
            gems.push(Gem { name, requirement: leading_strings(args).join(", "), dev: inline_dev || blocks.iter().any(|b| *b), local });
        } else if opens {
            blocks.push(line.starts_with("group") && (line.contains(":development") || line.contains(":test")));
        }
    }
    gems
}

/// Installed versions from `Gemfile.lock`'s rubygems.org (`GEM`) sections.
/// A platform suffix like `-x86_64-linux` is dropped.
pub fn read_lock(path: &Path) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(path) else { return HashMap::new() };
    let mut out = HashMap::new();
    let mut in_gem = false;
    for line in text.lines() {
        if !line.starts_with(' ') {
            in_gem = line.trim() == "GEM";
            continue;
        }
        if !in_gem || !line.starts_with("    ") || line.starts_with("     ") {
            continue;
        }
        let Some((name, version)) = line.trim().split_once(" (") else { continue };
        let version = version.trim_end_matches(')').split('-').next().unwrap_or_default();
        out.entry(name.to_string()).or_insert_with(|| version.to_string());
    }
    out
}

fn release(v: &str) -> Option<Vec<u64>> {
    v.trim().split('.').map(|p| p.parse::<u64>().ok()).collect()
}

fn cmp(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    let len = a.len().max(b.len());
    (0..len).map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0))).find(|o| o.is_ne()).unwrap_or(std::cmp::Ordering::Equal)
}

fn split_op(c: &str) -> (&str, &str) {
    let c = c.trim();
    for op in ["~>", ">=", "<=", "!=", "=", ">", "<"] {
        if let Some(v) = c.strip_prefix(op) {
            return (op, v.trim());
        }
    }
    ("=", c)
}

/// Whether `version` meets every requirement. `~> 2.7` means at least 2.7
/// and below 3.0; `~> 2.7.1` means at least 2.7.1 and below 2.8. Anything
/// unreadable (a pre-release like `2.0.0.pre1`) counts as met.
pub fn satisfies(version: &str, requirement: &str) -> bool {
    let Some(v) = release(version) else { return true };
    requirement.split(',').map(str::trim).filter(|c| !c.is_empty()).all(|c| {
        let (op, want) = split_op(c);
        let Some(w) = release(want) else { return true };
        let o = cmp(&v, &w);
        match op {
            "=" => o.is_eq(),
            "!=" => !o.is_eq(),
            ">" => o.is_gt(),
            "<" => o.is_lt(),
            ">=" => o.is_ge(),
            "<=" => o.is_le(),
            "~>" => {
                let keep = w.len().saturating_sub(1).max(1);
                let mut upper = w[..keep].to_vec();
                *upper.last_mut().unwrap() += 1;
                o.is_ge() && cmp(&v, &upper).is_lt()
            }
            _ => true,
        }
    })
}

fn render(parts: &[u64]) -> String {
    parts.iter().map(u64::to_string).collect::<Vec<_>>().join(".")
}

/// The requirement moved to allow `to`, keeping its style: `~> 1.2` ->
/// `~> 2.0`, `>= 1.0, < 2` -> `>= 2.1.3, < 3`, `1.2.3` -> `2.1.3`. Empty
/// (any version) stays empty.
pub fn rewrite(requirement: &str, to: &str) -> String {
    let Some(target) = release(to) else { return requirement.to_string() };
    requirement
        .split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .filter_map(|c| {
            let (op, want) = split_op(c);
            let written = c.starts_with(op);
            Some(match op {
                "~>" => {
                    let keep = release(want).map_or(target.len(), |w| w.len()).min(target.len());
                    format!("~> {}", render(&target[..keep]))
                }
                ">=" | ">" => format!(">= {to}"),
                "<" | "<=" if !satisfies(to, c) => format!("{op} {}", target[0] + 1),
                "!=" if satisfies(want, &format!("= {to}")) => return None,
                "=" if written => format!("= {to}"),
                "=" => to.to_string(),
                _ => c.to_string(),
            })
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const GEMFILE: &str = "source 'https://rubygems.org'\nruby '3.2.2'\n\ngem 'rails', '~> 7.1', '>= 7.1.3'\ngem \"puma\", \">= 5.0\" # web\ngem 'bootsnap', require: false\ngem 'mylib', path: '../mylib'\n\ngroup :development, :test do\n  gem 'rspec-rails', '~> 6.1.0'\n  platforms :mri do\n    gem 'debug'\n  end\nend\n\ngem 'rubocop', group: :development\n";

    #[test]
    fn reads_gems_groups_and_sources() {
        let gems = parse_gemfile(GEMFILE);
        let by = |n: &str| gems.iter().find(|g| g.name == n).unwrap().clone();
        assert_eq!(by("rails").requirement, "~> 7.1, >= 7.1.3");
        assert_eq!(by("puma").requirement, ">= 5.0");
        assert_eq!(by("bootsnap").requirement, "");
        assert!(by("mylib").local.is_some());
        assert!(by("rspec-rails").dev && by("debug").dev && by("rubocop").dev);
        assert!(!by("rails").dev);
    }

    #[test]
    fn reads_the_lockfile() {
        let dir = std::env::temp_dir().join("mehen-ruby-lock");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Gemfile.lock"), "PATH\n  remote: ../mylib\n  specs:\n    mylib (0.1.0)\n\nGEM\n  remote: https://rubygems.org/\n  specs:\n    nokogiri (1.16.0-x86_64-linux)\n      racc (~> 1.4)\n    rails (7.1.3)\n\nPLATFORMS\n  x86_64-linux\n").unwrap();
        let lock = read_lock(&dir.join("Gemfile.lock"));
        assert_eq!(lock.get("nokogiri").map(String::as_str), Some("1.16.0"));
        assert_eq!(lock.get("rails").map(String::as_str), Some("7.1.3"));
        assert!(!lock.contains_key("racc"), "a dependency's requirement is not an installed gem");
        assert!(!lock.contains_key("mylib"), "local gems come from PATH, not rubygems.org");
    }

    #[test]
    fn requirements() {
        assert!(satisfies("2.9.1", "~> 2.7") && !satisfies("3.0.0", "~> 2.7"));
        assert!(satisfies("2.7.5", "~> 2.7.1") && !satisfies("2.8.0", "~> 2.7.1"));
        assert!(satisfies("3.2.2", ">= 3.1") && !satisfies("3.0.6", ">= 3.1"));
        assert!(satisfies("7.1.5", "~> 7.1, >= 7.1.3") && !satisfies("7.1.2", "~> 7.1, >= 7.1.3"));
        assert_eq!(rewrite("~> 7.1, >= 7.1.3", "8.0.1"), "~> 8.0, >= 8.0.1");
        assert_eq!(rewrite(">= 1.0, < 2", "2.1.3"), ">= 2.1.3, < 3");
        assert_eq!(rewrite("~> 6.1.0", "7.0.2"), "~> 7.0.2");
        assert_eq!(rewrite("1.2.3", "2.0.0"), "2.0.0");
        assert_eq!(rewrite("", "2.0.0"), "");
    }
}
