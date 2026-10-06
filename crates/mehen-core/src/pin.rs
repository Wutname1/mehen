//! Exact pins (`=2.0.0-rc.22`, `"1.2.3"` in package.json, `==2.31.0`) and
//! the range an update can write instead, so later compatible releases (and
//! their security fixes) come in without another edit.

use std::sync::LazyLock;

use regex::Regex;

use crate::model::Ecosystem;
use crate::version::{Version, release_line};

static SEMVER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^v?\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$").expect("valid"));

/// Whether `requested`, as written in the manifest, allows exactly one version.
pub fn is_exact(ecosystem: Ecosystem, requested: &str) -> bool {
    let s = requested.trim();
    match ecosystem {
        Ecosystem::Npm => SEMVER.is_match(s.strip_prefix('=').unwrap_or(s).trim()),
        Ecosystem::Cargo => s.starts_with('=') && !s.contains(',') && Version::parse(s[1..].trim()).is_some(),
        // `==2.31.0`, or Poetry's bare `2.31.0`.
        Ecosystem::Pypi => {
            let v = s.strip_prefix("==").unwrap_or(if s.starts_with(|c: char| c.is_ascii_digit()) { s } else { "" }).trim();
            !v.is_empty() && !v.contains(['*', ',', ';']) && crate::python::release(v).is_some()
        }
        Ecosystem::Packagist => {
            let v = s.trim_start_matches('=').trim();
            !v.contains([' ', ',', '|', '*', '@']) && Version::parse(v).is_some()
        }
        Ecosystem::Pub => SEMVER.is_match(s) && !s.starts_with('v'),
        Ecosystem::RubyGems => {
            let v = s.strip_prefix('=').unwrap_or(s).trim();
            !v.contains(',') && v.starts_with(|c: char| c.is_ascii_digit()) && Version::parse(v).is_some()
        }
        // NuGet takes the lowest version a range allows, so loosening gains
        // nothing; Go has no ranges; actions pin tags or commits on purpose.
        Ecosystem::Nuget | Ecosystem::Go | Ecosystem::GithubActions => false,
    }
}

/// The first version past `to`'s release line: `3` for 2.32.3, `0.14` for 0.13.4.
fn next_line(to: &str) -> Option<String> {
    let line = release_line(to)?;
    let mut parts: Vec<u64> = line.split('.').filter_map(|p| p.parse().ok()).collect();
    *parts.last_mut()? += 1;
    Some(parts.iter().map(u64::to_string).collect::<Vec<_>>().join("."))
}

/// An exact pin `old` rewritten as a range from `to` up to its next breaking
/// release, in the ecosystem's own style. `None` when `old` is not an exact
/// pin this ecosystem can loosen.
pub fn loosened(ecosystem: Ecosystem, old: &str, to: &str) -> Option<String> {
    if !is_exact(ecosystem, old) {
        return None;
    }
    let to = to.trim().trim_start_matches(['v', 'V']);
    Some(match ecosystem {
        Ecosystem::Npm | Ecosystem::Packagist | Ecosystem::Pub => format!("^{to}"),
        // A bare Cargo requirement is already `^`.
        Ecosystem::Cargo => to.to_string(),
        Ecosystem::Pypi if old.trim().starts_with("==") => format!(">={to},<{}", next_line(to)?),
        Ecosystem::Pypi => format!("^{to}"),
        Ecosystem::RubyGems => {
            let v = Version::parse(to)?;
            if v.part(0) == 0 { format!("~> {to}") } else { format!("~> {}.{}, >= {to}", v.part(0), v.part(1)) }
        }
        Ecosystem::Nuget | Ecosystem::Go | Ecosystem::GithubActions => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_pins_by_ecosystem() {
        assert!(is_exact(Ecosystem::Npm, "1.2.3"));
        assert!(is_exact(Ecosystem::Npm, "=1.2.3-beta.1"));
        assert!(!is_exact(Ecosystem::Npm, "^1.2.3"));
        assert!(!is_exact(Ecosystem::Npm, "1.2"));
        assert!(is_exact(Ecosystem::Cargo, "=2.0.0-rc.22"));
        assert!(!is_exact(Ecosystem::Cargo, "2.0.0"));
        assert!(!is_exact(Ecosystem::Cargo, "~2.12.0"));
        assert!(is_exact(Ecosystem::Pypi, "==2.31.0"));
        assert!(!is_exact(Ecosystem::Pypi, "==2.*"));
        assert!(!is_exact(Ecosystem::Pypi, ">=2.31"));
        assert!(is_exact(Ecosystem::Packagist, "v1.2.3"));
        assert!(!is_exact(Ecosystem::Packagist, "^1.2"));
        assert!(is_exact(Ecosystem::RubyGems, "= 1.2.3"));
        assert!(!is_exact(Ecosystem::RubyGems, "~> 1.2"));
        assert!(!is_exact(Ecosystem::Nuget, "[1.2.3]"));
    }

    #[test]
    fn loosened_ranges_stop_before_the_next_breaking_release() {
        assert_eq!(loosened(Ecosystem::Npm, "1.2.3", "1.4.0").as_deref(), Some("^1.4.0"));
        assert_eq!(loosened(Ecosystem::Cargo, "=2.0.0-rc.22", "2.0.0-rc.25").as_deref(), Some("2.0.0-rc.25"));
        assert_eq!(loosened(Ecosystem::Pypi, "==2.31.0", "2.32.3").as_deref(), Some(">=2.32.3,<3"));
        assert_eq!(loosened(Ecosystem::Pypi, "==0.13.1", "0.13.4").as_deref(), Some(">=0.13.4,<0.14"));
        assert_eq!(loosened(Ecosystem::Pypi, "2.31.0", "2.32.3").as_deref(), Some("^2.32.3"));
        assert_eq!(loosened(Ecosystem::RubyGems, "= 7.1.3", "7.2.1").as_deref(), Some("~> 7.2, >= 7.2.1"));
        assert_eq!(loosened(Ecosystem::RubyGems, "0.13.1", "0.13.4").as_deref(), Some("~> 0.13.4"));
        assert_eq!(loosened(Ecosystem::Npm, "^1.2.3", "1.4.0"), None);
    }
}
