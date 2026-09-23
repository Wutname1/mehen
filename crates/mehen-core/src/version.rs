//! Lenient version handling that works across npm, Cargo, NuGet and action tags.
//! Strict semver would reject NuGet's four-part versions and `v4`-style tags.

use std::cmp::Ordering;

use crate::model::Status;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub parts: Vec<u64>,
    pub prerelease: bool,
}

impl Version {
    pub fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim().trim_start_matches(['v', 'V']);
        let (core, prerelease) = match s.find(['-', '+']) {
            Some(i) => (&s[..i], s[i..].starts_with('-')),
            None => (s, false),
        };
        if core.is_empty() {
            return None;
        }
        let parts = core.split('.').map(|p| p.parse::<u64>().ok()).collect::<Option<Vec<_>>>()?;
        Some(Self { parts, prerelease })
    }

    pub fn part(&self, i: usize) -> u64 {
        self.parts.get(i).copied().unwrap_or(0)
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let len = self.parts.len().max(other.parts.len());
        for i in 0..len {
            match self.part(i).cmp(&other.part(i)) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        // A prerelease sorts before its release.
        other.prerelease.cmp(&self.prerelease)
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Highest stable version in the list, falling back to the highest prerelease.
/// On a tie the most specific spelling wins, so `v7.0.1` beats `v7`.
pub fn max_version<'a>(versions: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let parsed: Vec<(Version, &str)> = versions.into_iter().filter_map(|v| Version::parse(v).map(|p| (p, v))).collect();
    let by_version = |a: &&(Version, &str), b: &&(Version, &str)| a.0.cmp(&b.0).then(a.0.parts.len().cmp(&b.0.parts.len()));
    parsed
        .iter()
        .filter(|(v, _)| !v.prerelease)
        .max_by(by_version)
        .or_else(|| parsed.iter().max_by(by_version))
        .map(|(_, raw)| raw.to_string())
}

/// Newest stable version on the same release line as `current` - same major,
/// or same minor for 0.x, where minor bumps are the breaking ones. `None`
/// when nothing newer exists on that line.
pub fn safe_target(current: &str, versions: &[String]) -> Option<String> {
    let c = Version::parse(current)?;
    let same_line = |v: &Version| if c.part(0) == 0 { v.part(0) == 0 && v.part(1) == c.part(1) } else { v.part(0) == c.part(0) };
    versions
        .iter()
        .filter_map(|raw| Version::parse(raw).map(|v| (v, raw)))
        .filter(|(v, _)| !v.prerelease && same_line(v) && *v > c)
        .max_by(|a, b| a.0.cmp(&b.0).then(a.0.parts.len().cmp(&b.0.parts.len())))
        .map(|(_, raw)| raw.clone())
}

/// Compares only as precisely as `current` was written, so a floating `v7`
/// tag counts as up to date against `7.0.1`, and `1.2` against `1.2.9`.
pub fn compare(current: &str, latest: &str) -> Status {
    let (Some(c), Some(l)) = (Version::parse(current), Version::parse(latest)) else {
        return Status::Unknown;
    };
    let precision = c.parts.len();
    for i in 0..precision {
        match c.part(i).cmp(&l.part(i)) {
            Ordering::Less => {
                return match i {
                    0 => Status::Major,
                    1 => Status::Minor,
                    _ => Status::Patch,
                };
            }
            Ordering::Greater => return Status::UpToDate,
            Ordering::Equal => {}
        }
    }
    Status::UpToDate
}

/// Pulls a concrete version out of a range spec: `^18.2.0` -> `18.2.0`,
/// `[1.0,2.0)` -> `1.0`, `>=1.2 <2` -> `1.2`, `1.*` -> `1`.
pub fn from_spec(spec: &str) -> Option<String> {
    let s = spec.trim();
    let s = s.split("||").next()?.trim();
    let s = s.split_whitespace().next()?;
    let s = s.trim_start_matches(['^', '~', '=', '>', '<', '[', '(', 'v']);
    let s = s.split([',', ')', ']']).next()?;
    let s = s.trim_end_matches(".*").trim_end_matches(".x").trim_end_matches('*');
    Version::parse(s).map(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_handles_mixed_precision_and_prerelease() {
        assert!(Version::parse("1.2.10").unwrap() > Version::parse("1.2.9").unwrap());
        assert!(Version::parse("2.0.0").unwrap() > Version::parse("2.0.0-beta.1").unwrap());
        assert!(Version::parse("v7").unwrap() > Version::parse("v6.9.9").unwrap());
        assert_eq!(max_version(["1.0.0", "2.0.0-rc1", "1.5.0"]), Some("1.5.0".into()));
    }

    #[test]
    fn compare_respects_written_precision() {
        assert_eq!(compare("7", "7.0.1"), Status::UpToDate);
        assert_eq!(compare("4", "7.0.1"), Status::Major);
        assert_eq!(compare("18.2.0", "18.3.1"), Status::Minor);
        assert_eq!(compare("13.0.1", "13.0.3"), Status::Patch);
        assert_eq!(compare("1.2", "1.2.9"), Status::UpToDate);
    }

    #[test]
    fn safe_target_stays_on_release_line() {
        let versions: Vec<String> = ["4.1.0", "4.9.2", "5.0.0", "5.1.0-beta", "0.3.9"].iter().map(|s| s.to_string()).collect();
        assert_eq!(safe_target("4.1.0", &versions).as_deref(), Some("4.9.2"));
        assert_eq!(safe_target("4.9.2", &versions), None);
        assert_eq!(safe_target("0.3.1", &versions).as_deref(), Some("0.3.9"));
        assert_eq!(safe_target("5.0.0", &versions), None);
    }

    #[test]
    fn spec_extraction() {
        assert_eq!(from_spec("^18.2.0").as_deref(), Some("18.2.0"));
        assert_eq!(from_spec("[1.0,2.0)").as_deref(), Some("1.0"));
        assert_eq!(from_spec(">=1.2 <2").as_deref(), Some("1.2"));
        assert_eq!(from_spec("1.*").as_deref(), Some("1"));
        assert_eq!(from_spec("workspace:*"), None);
        assert_eq!(from_spec("latest"), None);
    }
}
