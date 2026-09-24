//! Go modules: reading `go.mod` and rewriting the version of a `require`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Require {
    pub path: String,
    pub version: String,
    /// Marked `// indirect`: pulled in by another module, not imported here.
    pub indirect: bool,
}

#[derive(Debug, Default)]
pub struct GoMod {
    pub module: Option<String>,
    pub requires: Vec<Require>,
    /// Module path -> what replaces it (another module or a local folder).
    pub replaced: Vec<(String, String)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    Require,
    Replace,
    Other,
}

/// One `go.mod` line without its comment, the block it opens or sits in,
/// and whether the comment marks it `indirect`.
struct Line<'a> {
    words: Vec<&'a str>,
    indirect: bool,
}

fn split(line: &str) -> Line<'_> {
    let (code, comment) = line.split_once("//").unwrap_or((line, ""));
    let indirect = comment.trim().split(';').next().is_some_and(|c| c.trim() == "indirect");
    Line { words: code.split_whitespace().map(|w| w.trim_matches('"')).collect(), indirect }
}

/// Calls `entry` with each `require` or `replace` entry; returns the module path.
fn walk(text: &str, mut entry: impl FnMut(Block, &Line)) -> Option<String> {
    let mut block: Option<Block> = None;
    let mut module = None;
    for raw in text.lines() {
        let line = split(raw);
        let Some(first) = line.words.first().copied() else { continue };
        match block {
            Some(_) if first == ")" => block = None,
            Some(kind) => entry(kind, &line),
            None => {
                let kind = match first {
                    "require" => Block::Require,
                    "replace" => Block::Replace,
                    "module" => {
                        module = line.words.get(1).map(|m| m.to_string());
                        continue;
                    }
                    _ => Block::Other,
                };
                if line.words.get(1) == Some(&"(") {
                    block = Some(kind);
                } else if kind != Block::Other {
                    let rest = Line { words: line.words[1..].to_vec(), indirect: line.indirect };
                    entry(kind, &rest);
                }
            }
        }
    }
    module
}

pub fn parse(text: &str) -> GoMod {
    let mut requires = Vec::new();
    let mut replaced = Vec::new();
    let module = walk(text, |kind, line| match kind {
        Block::Require => {
            if let [path, version, ..] = line.words[..] {
                requires.push(Require { path: path.to_string(), version: version.to_string(), indirect: line.indirect });
            }
        }
        Block::Replace => {
            if let Some(arrow) = line.words.iter().position(|w| *w == "=>") {
                if let Some(from) = line.words.first() {
                    replaced.push((from.to_string(), line.words[arrow + 1..].join(" ")));
                }
            }
        }
        Block::Other => {}
    });
    GoMod { module, requires, replaced }
}

/// `github.com/acme/tool/v2` -> `tool`.
pub fn module_name(module: &str) -> String {
    let mut parts = module.rsplit('/');
    let last = parts.next().unwrap_or(module);
    let is_major = last.len() > 1 && last.starts_with('v') && last[1..].chars().all(|c| c.is_ascii_digit());
    if is_major { parts.next().unwrap_or(last).to_string() } else { last.to_string() }
}

/// Moves `path` from `from` to `to` in its `require` line, leaving `replace`
/// lines and everything else as written. `None` when no such line exists.
pub fn set_version(text: &str, path: &str, from: &str, to: &str) -> Option<String> {
    let mut block: Option<Block> = None;
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    for raw in text.split_inclusive('\n') {
        let line = split(raw);
        let first = line.words.first().copied();
        let is_entry = match (block, first) {
            (Some(_), Some(")")) => {
                block = None;
                false
            }
            (Some(Block::Require), Some(p)) => p == path && line.words.get(1) == Some(&from),
            (Some(_), _) | (None, None) => false,
            (None, Some(keyword)) => {
                if line.words.get(1) == Some(&"(") {
                    block = Some(match keyword {
                        "require" => Block::Require,
                        "replace" => Block::Replace,
                        _ => Block::Other,
                    });
                    false
                } else {
                    keyword == "require" && line.words.get(1) == Some(&path) && line.words.get(2) == Some(&from)
                }
            }
        };
        if is_entry && !changed {
            let after_path = raw.find(path).map(|i| i + path.len()).unwrap_or(0);
            if let Some(at) = raw[after_path..].find(from).map(|i| i + after_path) {
                out.push_str(&raw[..at]);
                out.push_str(to);
                out.push_str(&raw[at + from.len()..]);
                changed = true;
                continue;
            }
        }
        out.push_str(raw);
    }
    changed.then_some(out)
}

/// The module proxy spells capitals as `!` plus the lower-case letter.
pub fn proxy_escape(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if c.is_ascii_uppercase() {
            out.push('!');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GO_MOD: &str = "module github.com/mstrhakr/audplexus/v2\n\ngo 1.26\n\nrequire (\n\tgithub.com/gin-gonic/gin v1.12.0\n\tgithub.com/google/uuid v1.6.0\n\tgithub.com/local/thing v0.1.0\n\tgithub.com/bytedance/sonic v1.15.1 // indirect\n)\n\nrequire github.com/BurntSushi/toml v1.4.0\n\nreplace github.com/local/thing => ../thing\n\nexclude github.com/google/uuid v1.5.0\n";

    #[test]
    fn reads_direct_and_indirect_requires_and_replacements() {
        let m = parse(GO_MOD);
        assert_eq!(m.module.as_deref(), Some("github.com/mstrhakr/audplexus/v2"));
        let direct: Vec<(&str, &str)> = m.requires.iter().filter(|r| !r.indirect).map(|r| (r.path.as_str(), r.version.as_str())).collect();
        assert_eq!(direct, [("github.com/gin-gonic/gin", "v1.12.0"), ("github.com/google/uuid", "v1.6.0"), ("github.com/local/thing", "v0.1.0"), ("github.com/BurntSushi/toml", "v1.4.0")]);
        assert!(m.requires.iter().any(|r| r.path == "github.com/bytedance/sonic" && r.indirect));
        assert_eq!(m.replaced, [("github.com/local/thing".to_string(), "../thing".to_string())]);
        assert_eq!(module_name("github.com/mstrhakr/audplexus/v2"), "audplexus");
        assert_eq!(module_name("golang.org/x/crypto"), "crypto");
    }

    #[test]
    fn rewrites_only_the_require_line() {
        let text = "require (\r\n\tgithub.com/google/uuid v1.6.0\r\n)\r\n\r\nreplace github.com/google/uuid v1.6.0 => github.com/fork/uuid v1.6.1\r\n";
        let out = set_version(text, "github.com/google/uuid", "v1.6.0", "v1.7.0").unwrap();
        assert!(out.contains("\tgithub.com/google/uuid v1.7.0\r\n"), "{out}");
        assert!(out.contains("replace github.com/google/uuid v1.6.0 =>"), "replace line untouched: {out}");
        let single = set_version(GO_MOD, "github.com/BurntSushi/toml", "v1.4.0", "v1.5.0").unwrap();
        assert!(single.contains("require github.com/BurntSushi/toml v1.5.0\n"));
        assert!(set_version(GO_MOD, "github.com/google/uuid", "v1.5.0", "v1.7.0").is_none(), "exclude lines are not requires");
    }

    #[test]
    fn escapes_capitals_for_the_proxy() {
        assert_eq!(proxy_escape("github.com/BurntSushi/toml"), "github.com/!burnt!sushi/toml");
    }
}
