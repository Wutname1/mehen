//! Whether a published package version can be used by a particular project:
//! .NET target frameworks, a crate's minimum Rust version, or npm `engines.node`.
//! Versions that fail are never offered as update targets.

use serde::{Deserialize, Serialize};

use crate::version::Version;

/// What a published version needs from the project using it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Requirement {
    /// NuGet: target frameworks the package ships for.
    Frameworks { frameworks: Vec<String> },
    /// Cargo: minimum Rust version (`rust-version`).
    Rust { version: String },
    /// npm: the `engines.node` range.
    Node { range: String },
    /// npm: `peerDependencies`, packages this version expects the project to already have.
    Peers { peers: Vec<(String, String)> },
}

/// What a project can accept. `None`/empty means unknown, which never blocks.
#[derive(Debug, Clone, Default)]
pub struct ProjectEnv {
    pub frameworks: Vec<String>,
    pub rust: Option<String>,
    pub node: Option<String>,
    /// npm packages the project has installed, name to version, for peer checks.
    pub installed: std::collections::HashMap<String, String>,
}

/// `Err` carries a short reason for the UI, like "only supports net10.0".
pub fn check(requirement: &Requirement, env: &ProjectEnv) -> Result<(), String> {
    match requirement {
        Requirement::Frameworks { frameworks } => {
            let known: Vec<Tfm> = frameworks.iter().map(|f| parse_tfm(f)).filter(|t| t.family != Family::Unknown).collect();
            if env.frameworks.is_empty() || known.is_empty() {
                return Ok(());
            }
            let fits = env.frameworks.iter().map(|f| parse_tfm(f)).any(|project| known.iter().any(|pkg| tfm_compatible(&project, pkg)));
            if fits {
                Ok(())
            } else {
                let mut names: Vec<String> = known.iter().map(Tfm::short).collect();
                names.dedup();
                Err(format!("only supports {}", names.into_iter().take(3).collect::<Vec<_>>().join(", ")))
            }
        }
        Requirement::Rust { version } => match (&env.rust, Version::parse(version)) {
            (Some(have), Some(need)) if Version::parse(have).is_some_and(|h| h < need) => Err(format!("needs Rust {version}")),
            _ => Ok(()),
        },
        Requirement::Peers { peers } => {
            for (name, range) in peers {
                if let Some(have) = env.installed.get(name) {
                    if !semver_satisfies(have, range) {
                        return Err(format!("needs {name} {}; this project has {have}", range.trim()));
                    }
                }
            }
            Ok(())
        }
        Requirement::Node { range } => {
            let Some(have) = env.node.as_deref().and_then(coerce_node) else { return Ok(()) };
            match nodejs_semver::Range::parse(range) {
                Ok(r) if !have.satisfies(&r) => Err(format!("needs Node {}", range.trim())),
                _ => Ok(()),
            }
        }
    }
}

/// Does Node `version` fall inside `range`? Unparseable input counts as yes.
pub fn node_satisfies(version: &str, range: &str) -> bool {
    match (coerce_node(version), nodejs_semver::Range::parse(range)) {
        (Some(v), Ok(r)) => v.satisfies(&r),
        _ => true,
    }
}

/// Does an npm `version` fall inside an npm `range`? Unparseable input counts as yes.
pub fn semver_satisfies(version: &str, range: &str) -> bool {
    match (nodejs_semver::Version::parse(version.trim_start_matches(['v', 'V'])).ok().or_else(|| coerce_node(version)), nodejs_semver::Range::parse(range)) {
        (Some(v), Ok(r)) => v.satisfies(&r),
        _ => true,
    }
}

/// `20`, `v20.11`, `>=18.17` -> a full x.y.z version.
fn coerce_node(raw: &str) -> Option<nodejs_semver::Version> {
    let v = Version::parse(crate::version::from_spec(raw)?.as_str())?;
    nodejs_semver::Version::parse(format!("{}.{}.{}", v.part(0), v.part(1), v.part(2))).ok()
}

// ---------------------------------------------------------------- target frameworks
//
// Ported from nuget-compass's tfmCompat.ts. Not the whole NuGet.Frameworks
// graph: modern net N.0, netcoreapp, netstandard and .NET Framework, which is
// what SDK-style and packages.config projects use.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Net,
    NetCoreApp,
    NetStandard,
    NetFramework,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Tfm {
    family: Family,
    major: u32,
    minor: u32,
    /// `windows`, `android`... empty when not platform-specific.
    platform: String,
}

impl Tfm {
    fn unknown() -> Self {
        Tfm { family: Family::Unknown, major: 0, minor: 0, platform: String::new() }
    }

    fn short(&self) -> String {
        let platform = if self.platform.is_empty() { String::new() } else { format!("-{}", self.platform) };
        match self.family {
            Family::Net => format!("net{}.{}{platform}", self.major, self.minor),
            Family::NetCoreApp => format!("netcoreapp{}.{}", self.major, self.minor),
            Family::NetStandard => format!("netstandard{}.{}", self.major, self.minor),
            Family::NetFramework => format!("net{}{}", self.major, self.minor),
            Family::Unknown => "unknown".into(),
        }
    }
}

fn digits(s: &str) -> Vec<u32> {
    s.split('.').map_while(|p| p.parse().ok()).collect()
}

fn parse_tfm(input: &str) -> Tfm {
    let lower = input.trim().to_ascii_lowercase();

    // Long forms from package metadata: ".NETStandard2.0", ".NETFramework4.6.2",
    // ".NETCoreApp,Version=v3.1".
    for (prefix, family) in [(".netstandard", Family::NetStandard), (".netframework", Family::NetFramework), (".netcoreapp", Family::NetCoreApp)] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let rest = rest.trim_start_matches(',').trim().trim_start_matches("version=").trim_start_matches('v');
            let d = digits(rest);
            return match d.first() {
                Some(&major) => Tfm { family, major, minor: d.get(1).copied().unwrap_or(0), platform: String::new() },
                None => Tfm::unknown(),
            };
        }
    }

    // Old-style csproj: <TargetFrameworkVersion>v4.7.2</TargetFrameworkVersion>.
    if let Some(rest) = lower.strip_prefix('v') {
        let d = digits(rest);
        if d.len() >= 2 {
            return Tfm { family: Family::NetFramework, major: d[0], minor: d[1], platform: String::new() };
        }
    }

    // Short forms: net8.0, net8.0-windows, netcoreapp3.1, netstandard2.1, net48, net472.
    let (base, platform) = match lower.split_once('-') {
        Some((b, p)) => (b, p.to_string()),
        None => (lower.as_str(), String::new()),
    };
    for (prefix, family) in [("netcoreapp", Family::NetCoreApp), ("netstandard", Family::NetStandard)] {
        if let Some(rest) = base.strip_prefix(prefix) {
            let d = digits(rest);
            return match d.first() {
                Some(&major) => Tfm { family, major, minor: d.get(1).copied().unwrap_or(0), platform },
                None => Tfm::unknown(),
            };
        }
    }
    if let Some(rest) = base.strip_prefix("net") {
        if rest.contains('.') {
            let d = digits(rest);
            if let Some(&major) = d.first() {
                return Tfm { family: Family::Net, major, minor: d.get(1).copied().unwrap_or(0), platform };
            }
        }
        // Compact .NET Framework monikers: net48 -> 4.8, net472 -> 4.7(.2).
        if let Ok(n) = rest.parse::<u32>() {
            if (10..100).contains(&n) {
                return Tfm { family: Family::NetFramework, major: n / 10, minor: n % 10, platform };
            }
            if (100..1000).contains(&n) {
                return Tfm { family: Family::NetFramework, major: n / 100, minor: (n % 100) / 10, platform };
            }
        }
    }
    Tfm::unknown()
}

fn le(a: &Tfm, major: u32, minor: u32) -> bool {
    (a.major, a.minor) <= (major, minor)
}

/// Can a project targeting `project` install a package built for `pkg`?
fn tfm_compatible(project: &Tfm, pkg: &Tfm) -> bool {
    if project.family == Family::Unknown || pkg.family == Family::Unknown {
        return false;
    }
    // A platform-specific package (net8.0-windows) needs the same platform.
    if !pkg.platform.is_empty() && pkg.platform != project.platform {
        return false;
    }
    match project.family {
        Family::Net => match pkg.family {
            Family::Net => le(pkg, project.major, project.minor),
            Family::NetCoreApp => le(pkg, 3, 1),
            Family::NetStandard => le(pkg, 2, 1),
            _ => false,
        },
        Family::NetCoreApp => match pkg.family {
            Family::NetCoreApp => le(pkg, project.major, project.minor),
            Family::NetStandard => le(pkg, 2, if project.major >= 3 { 1 } else { 0 }),
            _ => false,
        },
        Family::NetStandard => pkg.family == Family::NetStandard && le(pkg, project.major, project.minor),
        Family::NetFramework => match pkg.family {
            Family::NetFramework => le(pkg, project.major, project.minor),
            Family::NetStandard => le(pkg, 2, 0),
            _ => false,
        },
        Family::Unknown => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frameworks(list: &[&str]) -> Requirement {
        Requirement::Frameworks { frameworks: list.iter().map(|s| s.to_string()).collect() }
    }

    fn env(frameworks: &[&str]) -> ProjectEnv {
        ProjectEnv { frameworks: frameworks.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn target_frameworks() {
        assert!(check(&frameworks(&["net10.0"]), &env(&["net8.0"])).is_err());
        assert!(check(&frameworks(&["net8.0", "net10.0"]), &env(&["net8.0"])).is_ok());
        assert!(check(&frameworks(&[".NETStandard2.0"]), &env(&["net8.0"])).is_ok());
        assert!(check(&frameworks(&[".NETStandard2.1"]), &env(&["net48"])).is_err());
        assert!(check(&frameworks(&[".NETStandard2.0"]), &env(&["v4.7.2"])).is_ok());
        assert!(check(&frameworks(&[".NETFramework4.6.2"]), &env(&["net472"])).is_ok());
        assert!(check(&frameworks(&["net8.0-windows7.0"]), &env(&["net8.0"])).is_err());
        assert!(check(&frameworks(&["net8.0"]), &env(&["net8.0-windows"])).is_ok());
        assert_eq!(check(&frameworks(&["net10.0"]), &env(&["net8.0"])).unwrap_err(), "only supports net10.0");
        // Unknown project frameworks never block.
        assert!(check(&frameworks(&["net10.0"]), &env(&[])).is_ok());
    }

    #[test]
    fn rust_and_node() {
        let rust = ProjectEnv { rust: Some("1.80".into()), ..Default::default() };
        assert!(check(&Requirement::Rust { version: "1.85".into() }, &rust).is_err());
        assert!(check(&Requirement::Rust { version: "1.70.0".into() }, &rust).is_ok());

        let node = ProjectEnv { node: Some(">=18".into()), ..Default::default() };
        assert!(check(&Requirement::Node { range: ">=20.0.0".into() }, &node).is_err());
        assert!(check(&Requirement::Node { range: "^18.17.0 || >=20".into() }, &ProjectEnv { node: Some("20".into()), ..Default::default() }).is_ok());
        assert!(check(&Requirement::Node { range: ">=14".into() }, &node).is_ok());
    }
}
