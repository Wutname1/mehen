//! Reads installed npm versions from lockfiles, for projects whose
//! node_modules folder is not installed.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use yaml_rust2::{Yaml, YamlLoader};

static TRAILING_COMMA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r",(\s*[}\]])").unwrap());

pub enum Lockfile {
    /// package-lock.json: "packages" keys like `node_modules/x` and `apps/web/node_modules/x`.
    Npm(serde_json::Value),
    /// pnpm-lock.yaml: importers keyed by relative project path.
    Pnpm(Yaml),
    /// bun.lock: "packages" keyed by name, value `["name@1.2.3", ...]`.
    Bun(serde_json::Value),
}

#[derive(Default)]
pub struct LockfileCache {
    loaded: HashMap<PathBuf, Option<Lockfile>>,
}

impl LockfileCache {
    /// Finds the nearest lockfile at or above `dir` (stopping at `root`) and
    /// returns the locked version of `name` plus the lockfile's file name.
    pub fn npm_version(&mut self, root: &Path, dir: &Path, name: &str) -> Option<(String, &'static str)> {
        for base in dir.ancestors().take_while(|d| d.starts_with(root)) {
            for file in ["package-lock.json", "pnpm-lock.yaml", "bun.lock"] {
                let path = base.join(file);
                if !path.is_file() {
                    continue;
                }
                let rel = dir.strip_prefix(base).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
                let lock = self.loaded.entry(path.clone()).or_insert_with(|| load(&path, file));
                return lock.as_ref().and_then(|l| lookup(l, &rel, name)).map(|v| (v, file));
            }
        }
        None
    }
}

fn load(path: &Path, file: &str) -> Option<Lockfile> {
    let text = fs::read_to_string(path).ok()?;
    match file {
        "package-lock.json" => serde_json::from_str(&text).ok().map(Lockfile::Npm),
        "pnpm-lock.yaml" => YamlLoader::load_from_str(&text).ok()?.into_iter().next().map(Lockfile::Pnpm),
        "bun.lock" => serde_json::from_str(&TRAILING_COMMA.replace_all(&text, "$1")).ok().map(Lockfile::Bun),
        _ => None,
    }
}

fn lookup(lock: &Lockfile, rel: &str, name: &str) -> Option<String> {
    match lock {
        Lockfile::Npm(json) => {
            let packages = &json["packages"];
            let nested = (!rel.is_empty()).then(|| format!("{rel}/node_modules/{name}"));
            nested
                .and_then(|k| packages[k.as_str()]["version"].as_str())
                .or_else(|| packages[format!("node_modules/{name}").as_str()]["version"].as_str())
                .or_else(|| json["dependencies"][name]["version"].as_str())
                .map(str::to_string)
        }
        Lockfile::Pnpm(yaml) => {
            let importer = &yaml["importers"][if rel.is_empty() { "." } else { rel }];
            ["dependencies", "devDependencies", "optionalDependencies"].iter().find_map(|section| {
                let entry = &importer[*section][name];
                // `version: 10.0.1(eslint@10.7.0)` - drop the peer suffix.
                let raw = entry["version"].as_str().or_else(|| entry.as_str())?;
                let version = raw.split('(').next()?.trim();
                (!version.starts_with("link:")).then(|| version.to_string())
            })
        }
        Lockfile::Bun(json) => {
            let spec = json["packages"][name][0].as_str()?;
            spec.rsplit_once('@').map(|(_, v)| v.to_string()).filter(|v| !v.is_empty())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_each_lockfile_format() {
        let npm = Lockfile::Npm(serde_json::json!({
            "packages": { "node_modules/react": { "version": "18.3.1" }, "apps/web/node_modules/react": { "version": "19.1.0" } }
        }));
        assert_eq!(lookup(&npm, "", "react").as_deref(), Some("18.3.1"));
        assert_eq!(lookup(&npm, "apps/web", "react").as_deref(), Some("19.1.0"));
        assert_eq!(lookup(&npm, "apps/other", "react").as_deref(), Some("18.3.1"));

        let pnpm = Lockfile::Pnpm(
            YamlLoader::load_from_str("importers:\n  .:\n    devDependencies:\n      eslint:\n        specifier: ^10\n        version: 10.7.0(jiti@2.0.0)\n")
                .unwrap()
                .remove(0),
        );
        assert_eq!(lookup(&pnpm, "", "eslint").as_deref(), Some("10.7.0"));

        let bun_text = r#"{ "lockfileVersion": 1, "packages": { "@types/node": ["@types/node@26.1.1", "", {}, "sha512-x"], }, }"#;
        let bun = Lockfile::Bun(serde_json::from_str(&TRAILING_COMMA.replace_all(bun_text, "$1")).unwrap());
        assert_eq!(lookup(&bun, "", "@types/node").as_deref(), Some("26.1.1"));
    }
}
