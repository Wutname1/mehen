//! Reads install and build output for dependency conflicts the package
//! manager reported, and ties each one to a package the update changed, so
//! the user can keep that package on the line it was on.

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::compat::semver_satisfies;
use crate::model::Ecosystem;
use crate::update::PlannedChange;
use crate::version::Version;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    /// One sentence: "eslint-plugin-react-hooks 5.2.0 needs eslint ^8.57.0 || ^9.0.0".
    pub summary: String,
    /// The updated package to keep back, when the conflict points at one.
    pub keep: Option<Keep>,
    /// The step failed over it; otherwise the install only warned.
    pub blocking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Keep {
    pub ecosystem: Ecosystem,
    pub name: String,
    /// `9` for 9.x, `0.13` for 0.13.x.
    pub line: String,
    pub from: String,
    pub to: String,
}

/// What a package manager said, before it is tied to the update.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Finding {
    /// The package whose version does not fit.
    dependency: String,
    /// The package that objects, as `name` and optional version.
    owner: Option<(String, Option<String>)>,
    /// What the owner asks for.
    range: Option<String>,
    /// The version that was picked, when the output says.
    found: Option<String>,
    /// Set when the output explains itself better than "owner needs dependency range".
    text: Option<String>,
}

/// `9` for 9.4.1, `0.13` for 0.13.4 (where minor releases break).
pub fn release_line(version: &str) -> Option<String> {
    let digits = version.trim().trim_start_matches(|c: char| !c.is_ascii_digit());
    let v = Version::parse(digits)?;
    Some(if v.part(0) == 0 { format!("0.{}", v.part(1)) } else { v.part(0).to_string() })
}

/// The ecosystem a step's program works on.
pub fn ecosystem_of(program: &str) -> Option<Ecosystem> {
    match program.to_ascii_lowercase().trim_end_matches(".cmd").trim_end_matches(".exe") {
        "npm" | "pnpm" | "yarn" | "bun" | "npx" => Some(Ecosystem::Npm),
        "cargo" => Some(Ecosystem::Cargo),
        "dotnet" => Some(Ecosystem::Nuget),
        "go" => Some(Ecosystem::Go),
        "uv" | "poetry" | "pdm" | "pipenv" | "pip" | "python" => Some(Ecosystem::Pypi),
        "flutter" | "dart" => Some(Ecosystem::Pub),
        "composer" => Some(Ecosystem::Packagist),
        "bundle" | "bundler" | "gem" | "ruby" => Some(Ecosystem::RubyGems),
        _ => None,
    }
}

/// Conflicts in one step's output. `ok` is whether the step passed; a passing
/// step can still warn about a peer it let through.
pub fn diagnose(output: &str, ok: bool, ecosystem: Ecosystem, changes: &[PlannedChange]) -> Vec<Conflict> {
    let findings = match ecosystem {
        Ecosystem::Npm => [npm(output), pnpm(output), yarn(output)].concat(),
        Ecosystem::Nuget => nuget(output),
        Ecosystem::Cargo => cargo(output),
        Ecosystem::GithubActions | Ecosystem::Go | Ecosystem::Pypi | Ecosystem::Pub | Ecosystem::Packagist | Ecosystem::RubyGems => Vec::new(),
    };
    let mut conflicts: Vec<Conflict> = Vec::new();
    for f in findings {
        let changed = |name: &str| changes.iter().find(|c| c.name.eq_ignore_ascii_case(name));
        // A range the picked version meets is a dependent npm lists for context, not a conflict.
        let picked = changed(&f.dependency).map(|c| c.to.clone()).or_else(|| f.found.clone());
        if let (Some(range), Some(v), Ecosystem::Npm) = (&f.range, &picked, ecosystem) {
            if semver_satisfies(v, range) {
                continue;
            }
        }
        let summary = f.text.clone().unwrap_or_else(|| {
            let wants = format!("{} {}", f.dependency, f.range.as_deref().unwrap_or("a different version"));
            match (&f.owner, &picked) {
                (Some((o, Some(ov))), Some(v)) => format!("{o} {ov} needs {wants}, not {v}"),
                (Some((o, Some(ov))), None) => format!("{o} {ov} needs {wants}"),
                (Some((o, None)), Some(v)) => format!("{o} needs {wants}, not {v}"),
                (Some((o, None)), None) => format!("{o} needs {wants}"),
                (None, _) => format!("Needs {wants}"),
            }
        });
        let keep = changed(&f.dependency)
            .or_else(|| f.owner.as_ref().and_then(|(o, _)| changed(o)))
            .and_then(|c| {
                let from = if c.from.is_empty() { c.written_before.clone() } else { c.from.clone() };
                let line = release_line(&from)?;
                // Keeping it where it already goes would change nothing.
                (release_line(&c.to).as_deref() != Some(line.as_str())).then(|| Keep { ecosystem, name: c.name.clone(), line, from, to: c.to.clone() })
            });
        let conflict = Conflict { summary, keep, blocking: !ok };
        if !conflicts.contains(&conflict) {
            conflicts.push(conflict);
        }
    }
    conflicts
}

/// A failed build or test that the package manager said nothing about: the
/// packages the update moved to a new major version that the output points
/// at, so the user can keep one back and let the rest through. `changes`
/// pairs each change with the manifest it was made in.
///
/// A mention of `name@version` (a tool saying which copy it loaded) outranks
/// weaker hints, like a file inside the package or its name and new major
/// together ("TypeScript 7"). Stack frames are skipped: they name the tools
/// that ran, not what broke.
pub fn suspects(output: &str, ecosystem: Ecosystem, changes: &[(&str, &PlannedChange)]) -> Vec<Conflict> {
    let from_of = |c: &PlannedChange| if c.from.is_empty() { c.written_before.clone() } else { c.from.clone() };
    let jumps: Vec<(&str, &PlannedChange, String)> = changes
        .iter()
        .filter_map(|(manifest, c)| {
            let c: &PlannedChange = c;
            let line = release_line(&from_of(c))?;
            (release_line(&c.to).as_deref() != Some(line.as_str())).then_some((*manifest, c, line))
        })
        .collect();
    if jumps.is_empty() {
        return Vec::new();
    }
    let text: String = output.lines().filter(|l| !l.trim_start().starts_with("at ")).collect::<Vec<_>>().join("\n");
    let lower = text.to_lowercase();
    let slashed = lower.replace('\\', "/");

    let loaded = |c: &PlannedChange| lower.contains(&format!("{}@{}", c.name.to_lowercase(), c.to.to_lowercase()));
    let hinted = |manifest: &str, c: &PlannedChange| {
        let name = c.name.to_lowercase();
        let inside = match ecosystem {
            Ecosystem::Npm => slashed.contains(&format!("node_modules/{name}/")),
            Ecosystem::Nuget => slashed.contains(&format!("packages/{name}/{}/", c.to.to_lowercase())),
            _ => false,
        };
        let major = Version::parse(&c.to).map(|v| v.part(0)).filter(|m| *m > 0);
        let named = major.is_some_and(|m| Regex::new(&format!(r"(?i)(?:^|[^\w@/.-]){}\s+{m}\b", regex::escape(&c.name))).is_ok_and(|re| re.is_match(&text)));
        // The compiler's own diagnostics.
        let compiler = ecosystem == Ecosystem::Npm && name == "typescript" && Regex::new(r"error TS\d{4,5}\b").is_ok_and(|re| re.is_match(&text));
        // MSBuild ends each error with the project it came from.
        let project = ecosystem == Ecosystem::Nuget && {
            let manifest = manifest.to_lowercase().replace('\\', "/");
            slashed.lines().any(|l| l.contains(": error ") && l.trim_end().ends_with(&format!("[{manifest}]")))
        };
        inside || named || compiler || project
    };

    let strong: Vec<_> = jumps.iter().filter(|(_, c, _)| loaded(c)).collect();
    let picked: Vec<_> = if strong.is_empty() { jumps.iter().filter(|(m, c, _)| hinted(m, c)).collect() } else { strong };
    let mut conflicts: Vec<Conflict> = Vec::new();
    for &&(_, c, ref line) in &picked {
        if conflicts.iter().any(|x| x.keep.as_ref().is_some_and(|k| k.name.eq_ignore_ascii_case(&c.name))) {
            continue;
        }
        let from = from_of(c);
        let jump = if line.starts_with("0.") { "a new 0.x version" } else { "a new major version" };
        conflicts.push(Conflict {
            summary: format!("{} {} is {jump} (the project had {from}), and the failure points at it", c.name, c.to),
            keep: Some(Keep { ecosystem, name: c.name.clone(), line: line.clone(), from, to: c.to.clone() }),
            blocking: true,
        });
    }
    conflicts
}

fn rx(pattern: &str) -> Regex {
    Regex::new(pattern).expect("diagnose patterns are valid")
}

/// Splits `name@version`, keeping the scope of `@scope/name@1.0.0`.
fn split_at_version(spec: &str) -> (String, Option<String>) {
    match spec[1..].rfind('@') {
        Some(i) => (spec[..=i].to_string(), Some(spec[i + 2..].to_string())),
        None => (spec.to_string(), None),
    }
}

fn npm(output: &str) -> Vec<Finding> {
    static PEER: LazyLock<Regex> = LazyLock::new(|| rx(r#"peer(?:Optional)? (@?[^@\s"]+)@"([^"]+)" from (@?[^@\s"]+@[^\s"]+)"#));
    static FOUND: LazyLock<Regex> = LazyLock::new(|| rx(r#"(?:Found|Conflicting peer dependency): (@?[^@\s"]+)@([^\s"]+)"#));
    let found: Vec<(String, String)> = FOUND.captures_iter(output).map(|c| (c[1].to_string(), c[2].to_string())).collect();
    PEER.captures_iter(output)
        .map(|c| {
            let dependency = c[1].to_string();
            Finding {
                found: found.iter().find(|(n, _)| *n == dependency).map(|(_, v)| v.clone()),
                owner: Some(split_at_version(&c[3])),
                range: Some(c[2].to_string()),
                dependency,
                text: None,
            }
        })
        .collect()
}

fn pnpm(output: &str) -> Vec<Finding> {
    static UNMET: LazyLock<Regex> = LazyLock::new(|| rx(r#"(?:unmet|missing) peer (@?[^@\s"]+)@"?([^":]+?)"?(?:: found (\S+))?\s*$"#));
    static OWNER: LazyLock<Regex> = LazyLock::new(|| rx(r"[┬─]\s+(@?\S+)\s+(\d\S*)\s*$"));
    let mut owner: Option<(String, Option<String>)> = None;
    let mut findings = Vec::new();
    for line in output.lines() {
        if let Some(c) = UNMET.captures(line) {
            findings.push(Finding {
                dependency: c[1].to_string(),
                owner: owner.clone(),
                range: Some(c[2].trim().to_string()),
                found: c.get(3).map(|m| m.as_str().to_string()),
                text: None,
            });
        } else if let Some(c) = OWNER.captures(line) {
            owner = Some((c[1].to_string(), Some(c[2].to_string())));
        }
    }
    findings
}

fn yarn(output: &str) -> Vec<Finding> {
    static CLASSIC: LazyLock<Regex> = LazyLock::new(|| rx(r#""(?:[^"]*> )?(@?[^@"\s]+@[^"\s]+)" has (?:incorrect|unmet) peer dependency "(@?[^@"]+)@([^"]+)""#));
    static BERRY: LazyLock<Regex> =
        LazyLock::new(|| rx(r"(@?[^\s│]+) is listed by your project with version (\S+?),? which doesn't satisfy what (@?[^\s]+) (?:\([^)]*\) )?requests \(([^)]+)\)"));
    let classic = CLASSIC.captures_iter(output).map(|c| Finding {
        dependency: c[2].to_string(),
        owner: Some(split_at_version(&c[1])),
        range: Some(c[3].to_string()),
        found: None,
        text: None,
    });
    let berry = BERRY.captures_iter(output).map(|c| Finding {
        dependency: c[1].to_string(),
        owner: Some((c[3].to_string(), None)),
        range: Some(c[4].to_string()),
        found: Some(c[2].to_string()),
        text: None,
    });
    classic.chain(berry).collect()
}

fn nuget(output: &str) -> Vec<Finding> {
    static INCOMPATIBLE: LazyLock<Regex> = LazyLock::new(|| rx(r"NU1202: Package (\S+) (\S+) is not compatible with (\S+)"));
    static SUPPORTS: LazyLock<Regex> = LazyLock::new(|| rx(r"supports:\s*(?:\r?\n\s*-\s*)?(\S+)"));
    static OUTSIDE: LazyLock<Regex> =
        LazyLock::new(|| rx(r"NU1608: Detected package version outside of dependency constraint: (\S+) (\S+) requires (\S+) \(([^)]+)\) but version \S+ (\S+) was resolved"));
    static DOWNGRADE: LazyLock<Regex> = LazyLock::new(|| rx(r"NU1605: (?:Warning As Error: )?Detected package downgrade: (\S+) from (\S+) to ([^\s.]+(?:\.[^\s.]+)*)"));
    static PATH: LazyLock<Regex> = LazyLock::new(|| rx(r"-> (\S+) (\d\S*) -> (\S+) \(([^)]+)\)"));

    let mut findings = Vec::new();
    for c in INCOMPATIBLE.captures_iter(output) {
        let rest = &output[c.get(0).unwrap().end()..];
        let supports = SUPPORTS.captures(rest.lines().take(3).collect::<Vec<_>>().join("\n").as_str()).map(|s| s[1].trim_end_matches('.').to_string());
        let (name, version, tfm) = (&c[1], &c[2], &c[3]);
        findings.push(Finding {
            dependency: name.to_string(),
            owner: None,
            range: None,
            found: Some(version.to_string()),
            text: Some(match supports {
                Some(s) => format!("{name} {version} needs {s}; this project targets {tfm}"),
                None => format!("{name} {version} does not support {tfm}"),
            }),
        });
    }
    for c in OUTSIDE.captures_iter(output) {
        findings.push(Finding {
            dependency: c[3].to_string(),
            owner: Some((c[1].to_string(), Some(c[2].to_string()))),
            range: Some(c[4].to_string()),
            found: Some(c[5].to_string()),
            text: None,
        });
    }
    for c in DOWNGRADE.captures_iter(output) {
        let (name, needed, have) = (c[1].to_string(), c[2].to_string(), c[3].to_string());
        let owner = PATH.captures_iter(output).find(|p| p[3].eq_ignore_ascii_case(&name)).map(|p| (p[1].to_string(), Some(p[2].to_string())));
        let text = match &owner {
            Some((o, Some(ov))) => format!("{o} {ov} needs {name} {needed} or newer; this project asks for {have}"),
            _ => format!("Another package needs {name} {needed} or newer; this project asks for {have}"),
        };
        findings.push(Finding { dependency: name, owner, range: None, found: Some(have), text: Some(text) });
    }
    if output.contains("NU1107") {
        for p in PATH.captures_iter(output) {
            findings.push(Finding {
                dependency: p[3].to_string(),
                owner: Some((p[1].to_string(), Some(p[2].to_string()))),
                range: Some(p[4].to_string()),
                found: None,
                text: None,
            });
        }
    }
    findings
}

fn cargo(output: &str) -> Vec<Finding> {
    static SELECT: LazyLock<Regex> = LazyLock::new(|| rx(r"failed to select a version for (?:the requirement )?`([^`\s=]+)[^`]*`"));
    static REQUIRED: LazyLock<Regex> = LazyLock::new(|| rx(r"required by package `(\S+) v([^\s`]+)"));
    SELECT
        .captures_iter(output)
        .map(|c| {
            let rest = &output[c.get(0).unwrap().end()..];
            let owner = REQUIRED.captures(rest).map(|r| (r[1].to_string(), Some(r[2].to_string())));
            let name = c[1].to_string();
            Finding {
                text: Some(match &owner {
                    Some((o, Some(v))) => format!("No version of {name} fits both this project and {o} {v}"),
                    _ => format!("No version of {name} fits every package that uses it"),
                }),
                dependency: name,
                owner,
                range: None,
                found: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(name: &str, from: &str, to: &str) -> PlannedChange {
        PlannedChange { name: name.into(), from: from.into(), to: to.into(), written_before: format!("^{from}"), written_after: format!("^{to}") }
    }

    #[test]
    fn npm_eresolve_points_at_the_updated_package() {
        let output = r#"npm error code ERESOLVE
npm error ERESOLVE unable to resolve dependency tree
npm error
npm error While resolving: web@0.1.0
npm error Found: eslint@10.0.1
npm error node_modules/eslint
npm error   dev eslint@"^10.0.1" from the root project
npm error   peer eslint@"^6.0.0 || ^7.0.0 || >=8.0.0" from @eslint-community/eslint-utils@4.4.0
npm error
npm error Could not resolve dependency:
npm error peer eslint@"^8.57.0 || ^9.0.0" from eslint-plugin-react-hooks@5.2.0
npm error node_modules/eslint-plugin-react-hooks
npm error   dev eslint-plugin-react-hooks@"^5.2.0" from the root project"#;
        let conflicts = diagnose(output, false, Ecosystem::Npm, &[change("eslint", "9.39.2", "10.0.1")]);
        assert_eq!(conflicts.len(), 1, "the satisfied eslint-utils peer is context, not a conflict: {conflicts:?}");
        assert_eq!(conflicts[0].summary, "eslint-plugin-react-hooks 5.2.0 needs eslint ^8.57.0 || ^9.0.0, not 10.0.1");
        let keep = conflicts[0].keep.as_ref().unwrap();
        assert_eq!((keep.name.as_str(), keep.line.as_str()), ("eslint", "9"));
        assert!(conflicts[0].blocking);
    }

    #[test]
    fn a_plugin_update_that_needs_a_newer_peer_keeps_the_plugin() {
        let output = r#"npm ERR! Could not resolve dependency:
npm ERR! peer @typescript-eslint/parser@"^8.0.0" from @typescript-eslint/eslint-plugin@8.1.0
npm ERR! Conflicting peer dependency: @typescript-eslint/parser@7.18.0"#;
        let conflicts = diagnose(output, false, Ecosystem::Npm, &[change("@typescript-eslint/eslint-plugin", "7.18.0", "8.1.0")]);
        assert_eq!(conflicts[0].summary, "@typescript-eslint/eslint-plugin 8.1.0 needs @typescript-eslint/parser ^8.0.0, not 7.18.0");
        assert_eq!(conflicts[0].keep.as_ref().map(|k| k.name.as_str()), Some("@typescript-eslint/eslint-plugin"));
    }

    #[test]
    fn pnpm_peer_warnings_are_not_blocking() {
        let output = " WARN  Issues with peer dependencies found\n.\n└─┬ eslint-plugin-react-hooks 5.2.0\n  └── ✕ unmet peer eslint@\"^8.57.0 || ^9.0.0\": found 10.0.1\n";
        let conflicts = diagnose(output, true, Ecosystem::Npm, &[change("eslint", "9.39.2", "10.0.1")]);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].summary, "eslint-plugin-react-hooks 5.2.0 needs eslint ^8.57.0 || ^9.0.0, not 10.0.1");
        assert!(!conflicts[0].blocking);
    }

    #[test]
    fn yarn_classic_and_berry() {
        let classic = r#"warning " > eslint-plugin-react-hooks@5.2.0" has incorrect peer dependency "eslint@^8.57.0 || ^9.0.0"."#;
        let berry = "➤ YN0060: │ eslint is listed by your project with version 10.0.1, which doesn't satisfy what eslint-plugin-react-hooks (p1a2b3) requests (^8.57.0 || ^9.0.0).";
        let changes = [change("eslint", "9.39.2", "10.0.1")];
        assert_eq!(diagnose(classic, true, Ecosystem::Npm, &changes)[0].keep.as_ref().unwrap().line, "9");
        assert_eq!(diagnose(berry, true, Ecosystem::Npm, &changes)[0].summary, "eslint-plugin-react-hooks needs eslint ^8.57.0 || ^9.0.0, not 10.0.1");
    }

    #[test]
    fn nuget_framework_and_downgrade_errors() {
        let incompatible = "error NU1202: Package Microsoft.EntityFrameworkCore 10.0.1 is not compatible with net8.0 (.NETCoreApp,Version=v8.0). Package Microsoft.EntityFrameworkCore 10.0.1 supports: net10.0 (.NETCoreApp,Version=v10.0)";
        let c = diagnose(incompatible, false, Ecosystem::Nuget, &[change("Microsoft.EntityFrameworkCore", "8.0.11", "10.0.1")]);
        assert_eq!(c[0].summary, "Microsoft.EntityFrameworkCore 10.0.1 needs net10.0; this project targets net8.0");
        assert_eq!(c[0].keep.as_ref().unwrap().line, "8");

        let downgrade = "error NU1605: Warning As Error: Detected package downgrade: Microsoft.Extensions.Logging from 9.0.0 to 8.0.0. Reference the package directly from the project to select a different version.\n Api -> Serilog.Extensions.Logging 9.0.0 -> Microsoft.Extensions.Logging (>= 9.0.0)\n Api -> Microsoft.Extensions.Logging (>= 8.0.0)";
        let c = diagnose(downgrade, false, Ecosystem::Nuget, &[change("Serilog.Extensions.Logging", "8.0.0", "9.0.0")]);
        assert_eq!(c[0].summary, "Serilog.Extensions.Logging 9.0.0 needs Microsoft.Extensions.Logging 9.0.0 or newer; this project asks for 8.0.0");
        assert_eq!(c[0].keep.as_ref().unwrap().name, "Serilog.Extensions.Logging");

        let outside = "warning NU1608: Detected package version outside of dependency constraint: Pomelo.EntityFrameworkCore.MySql 8.0.2 requires Microsoft.EntityFrameworkCore.Relational (>= 8.0.2 && <= 8.0.999) but version Microsoft.EntityFrameworkCore.Relational 9.0.0 was resolved.";
        let c = diagnose(outside, true, Ecosystem::Nuget, &[change("Microsoft.EntityFrameworkCore.Relational", "8.0.11", "9.0.0")]);
        assert_eq!(c[0].keep.as_ref().unwrap().line, "8");
        assert!(!c[0].blocking);
    }

    #[test]
    fn cargo_version_selection() {
        let output = "error: failed to select a version for `windows-sys`.\n    ... required by package `mio v1.0.2`\n    ... which satisfies dependency `mio = \"^1.0\"` of package `tokio v1.40.0`\nversions that meet the requirements `^0.52` are: 0.52.0";
        let c = diagnose(output, false, Ecosystem::Cargo, &[change("windows-sys", "0.52.0", "0.59.0")]);
        assert_eq!(c[0].summary, "No version of windows-sys fits both this project and mio 1.0.2");
        assert_eq!(c[0].keep.as_ref().unwrap().line, "0.52");
    }

    #[test]
    fn nothing_to_keep_when_the_update_stays_on_its_line() {
        let output = r#"npm error peer react@"^18.0.0" from some-lib@1.0.0
npm error Found: react@19.0.0"#;
        let c = diagnose(output, false, Ecosystem::Npm, &[change("react", "19.0.0", "19.1.0")]);
        assert!(c[0].keep.is_none());
    }

    fn keeps(conflicts: &[Conflict]) -> Vec<(String, String)> {
        conflicts.iter().filter_map(|c| c.keep.as_ref().map(|k| (k.name.clone(), k.line.clone()))).collect()
    }

    #[test]
    fn a_tool_naming_the_copy_it_loaded_outranks_other_hints() {
        let output = r"ncc: Using typescript@7.0.2 (local user-provided)
Error: Module build failed (from ./node_modules/@vercel/ncc/dist/ncc/loaders/ts-loader.js):
TypeError: Cannot read properties of undefined (reading 'fileExists')
    at findConfigFile (C:\app\node_modules\@vercel\ncc\dist\ncc\loaders/ts-loader.js.cache.js:38:11157)";
        let (ts, ncc, yaml) = (change("typescript", "6.0.3", "7.0.2"), change("@vercel/ncc", "0.44.1", "0.45.0"), change("js-yaml", "5.2.1", "5.4.2"));
        let c = suspects(output, Ecosystem::Npm, &[("package.json", &ts), ("package.json", &ncc), ("package.json", &yaml)]);
        assert_eq!(keeps(&c), [("typescript".to_string(), "6".to_string())]);
        assert!(c[0].blocking);
        assert_eq!(c[0].summary, "typescript 7.0.2 is a new major version (the project had 6.0.3), and the failure points at it");
    }

    #[test]
    fn a_missing_export_points_at_the_package_not_the_bundler_in_the_stack() {
        let output = r#"vite v8.3.1 building client environment for production...
[MISSING_EXPORT] "Github" is not exported by "node_modules/lucide-react/dist/esm/lucide-react.mjs".
    at aggregateBindingErrorsIntoJsError (file:///C:/app/node_modules/rolldown/dist/shared/error.mjs:48:18)
    at async buildEnvironment (file:///C:/app/node_modules/vite/dist/node/chunks/node.js:34445:66)"#;
        let (lucide, vite, react) = (change("lucide-react", "0.468.0", "1.48.0"), change("vite", "6.4.3", "8.3.1"), change("react", "18.3.1", "19.3.0"));
        let c = suspects(output, Ecosystem::Npm, &[("package.json", &lucide), ("package.json", &vite), ("package.json", &react)]);
        assert_eq!(keeps(&c), [("lucide-react".to_string(), "0.468".to_string())]);
        assert!(c[0].summary.contains("a new 0.x version"), "{}", c[0].summary);
    }

    #[test]
    fn typescript_is_named_by_its_new_major_or_its_own_errors() {
        let ts = change("typescript", "6.0.3", "7.0.2");
        let plugin = "Error: [unplugin-dts] The installed \"typescript\" package does not provide the JavaScript Compiler API (this happens with TypeScript 7+)";
        assert_eq!(keeps(&suspects(plugin, Ecosystem::Npm, &[("package.json", &ts)])), [("typescript".to_string(), "6".to_string())]);
        let tsc = "tsconfig.json(25,5): error TS5102: Option 'downlevelIteration' has been removed. Please remove it from your configuration.";
        assert_eq!(suspects(tsc, Ecosystem::Npm, &[("package.json", &ts)]).len(), 1);
        // The same errors with TypeScript on its old line are not the update's doing.
        assert!(suspects(tsc, Ecosystem::Npm, &[("package.json", &change("typescript", "6.0.2", "6.0.3"))]).is_empty());
    }

    #[test]
    fn dotnet_errors_point_at_the_project_whose_package_jumped() {
        let unit = r"C:\Code\App\App.Tests\App.Tests.csproj";
        let output = r"C:\Code\App\App.Web\App.Web.csproj : warning NU1510: PackageReference X will not be pruned.
C:\Code\App\App.Tests\Services\ApiTests.cs(164,24): error CS1061: 'Task<Exception?>' does not contain a definition for 'Message' [C:\Code\App\App.Tests\App.Tests.csproj]";
        let (nunit, msal) = (change("NUnit", "4.6.1", "5.0.0"), change("Microsoft.Identity.Client", "4.90.0", "4.90.1"));
        let c = suspects(output, Ecosystem::Nuget, &[(unit, &nunit), (r"C:\Code\App\App.Data\App.Data.csproj", &msal)]);
        assert_eq!(keeps(&c), [("NUnit".to_string(), "4".to_string())]);
        // A project with no big jump in it is left alone.
        assert!(suspects(output, Ecosystem::Nuget, &[(r"C:\Code\App\App.Web\App.Web.csproj", &nunit)]).is_empty());
    }

    #[test]
    fn nothing_is_blamed_without_a_hint() {
        let output = "src/views/Admin.tsx(237,12): Type '{ children: Element[]; }' is not assignable to type 'TableProps'.";
        assert!(suspects(output, Ecosystem::Npm, &[("package.json", &change("@types/node", "25.9.1", "26.6.3"))]).is_empty());
    }

    #[test]
    fn release_lines() {
        assert_eq!(release_line("^5.1.2").as_deref(), Some("5"));
        assert_eq!(release_line("0.13.4").as_deref(), Some("0.13"));
        assert_eq!(release_line("v4").as_deref(), Some("4"));
        assert_eq!(release_line("latest"), None);
    }
}
