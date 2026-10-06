//! Lenient version handling that works across every supported ecosystem and action tags.
//! Strict semver would reject NuGet's four-part versions and `v4`-style tags.

use std::cmp::Ordering;

use crate::model::Status;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub parts: Vec<u64>,
    pub prerelease: bool,
    /// The prerelease identifiers: `["rc", "22"]` for `2.0.0-rc.22`.
    pub pre: Vec<String>,
}

impl Version {
    pub fn parse(raw: &str) -> Option<Self> {
        let s = raw.trim().trim_start_matches(['v', 'V']);
        let (core, prerelease, pre) = match s.find(['-', '+']) {
            Some(i) if s[i..].starts_with('-') => {
                let tag = s[i + 1..].split('+').next().unwrap_or_default();
                (&s[..i], true, tag.split('.').map(str::to_string).collect())
            }
            Some(i) => (&s[..i], false, Vec::new()),
            None => (s, false, Vec::new()),
        };
        if core.is_empty() {
            return None;
        }
        let parts = core.split('.').map(|p| p.parse::<u64>().ok()).collect::<Option<Vec<_>>>()?;
        Some(Self { parts, prerelease, pre })
    }

    pub fn part(&self, i: usize) -> u64 {
        self.parts.get(i).copied().unwrap_or(0)
    }

    /// The part a breaking release bumps: the major, or for 0.x the first
    /// part that is not zero (`0.13` breaks at the minor, `0.0.9` at the patch).
    fn breaking_part(&self) -> usize {
        self.parts.iter().position(|p| *p != 0).unwrap_or(self.parts.len().saturating_sub(1)).min(2)
    }
}

/// Semver's rule for prerelease tags: numbers compare as numbers and sort
/// before words, and a shorter tag sorts first when the rest is equal.
fn compare_pre(a: &[String], b: &[String]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        let o = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => x.cmp(y),
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    a.len().cmp(&b.len())
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
        match (self.prerelease, other.prerelease) {
            (true, true) => compare_pre(&self.pre, &other.pre),
            (a, b) => b.cmp(&a),
        }
    }
}

/// The release line `version` is on, as far as breaking changes go: `9` for
/// 9.4.1, `0.13` for 0.13.4, and `0.0.9` for 0.0.9, where every release can
/// break. The same lines Cargo and npm's `^` keep to.
pub fn release_line(version: &str) -> Option<String> {
    let digits = version.trim().trim_start_matches(|c: char| !c.is_ascii_digit());
    let v = Version::parse(digits)?;
    let upto = v.breaking_part().min(v.parts.len() - 1);
    Some(v.parts[..=upto].iter().map(u64::to_string).collect::<Vec<_>>().join("."))
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

/// Newest stable version on the same release line as `current` (see
/// [`release_line`]). `None` when nothing newer exists on that line, and for
/// a prerelease, which promises nothing about the next release.
pub fn safe_target(current: &str, versions: &[String]) -> Option<String> {
    let c = Version::parse(current)?;
    if c.prerelease {
        return None;
    }
    let upto = c.breaking_part();
    let same_line = |v: &Version| (0..=upto).all(|i| v.part(i) == c.part(i));
    versions
        .iter()
        .filter_map(|raw| Version::parse(raw).map(|v| (v, raw)))
        .filter(|(v, _)| !v.prerelease && same_line(v) && *v > c)
        .max_by(|a, b| a.0.cmp(&b.0).then(a.0.parts.len().cmp(&b.0.parts.len())))
        .map(|(_, raw)| raw.clone())
}

/// Newest stable version with the same major and minor as `current`: bug
/// fixes only. `None` when nothing newer exists on that line, and for 0.0.x
/// or a prerelease, where no release is only a bug fix.
pub fn patch_target(current: &str, versions: &[String]) -> Option<String> {
    let c = Version::parse(current)?;
    if c.prerelease || c.breaking_part() >= 2 {
        return None;
    }
    versions
        .iter()
        .filter_map(|raw| Version::parse(raw).map(|v| (v, raw)))
        .filter(|(v, _)| !v.prerelease && v.part(0) == c.part(0) && v.part(1) == c.part(1) && *v > c)
        .max_by(|a, b| a.0.cmp(&b.0).then(a.0.parts.len().cmp(&b.0.parts.len())))
        .map(|(_, raw)| raw.clone())
}

/// Compares only as precisely as `current` was written, so a floating `v7`
/// tag counts as up to date against `7.0.1`, and `1.2` against `1.2.9`.
/// Levels follow [`release_line`], so 0.12 to 0.13 is a major move, and so
/// is any move off a prerelease.
pub fn compare(current: &str, latest: &str) -> Status {
    let (Some(c), Some(l)) = (Version::parse(current), Version::parse(latest)) else {
        return Status::Unknown;
    };
    let shift = c.breaking_part();
    for i in 0..c.parts.len() {
        match c.part(i).cmp(&l.part(i)) {
            Ordering::Less if c.prerelease || i <= shift => return Status::Major,
            Ordering::Less if i == shift + 1 => return Status::Minor,
            Ordering::Less => return Status::Patch,
            Ordering::Greater => return Status::UpToDate,
            Ordering::Equal => {}
        }
    }
    if c.prerelease && c < l {
        return Status::Major;
    }
    Status::UpToDate
}

/// Where a prerelease can go: the newest stable release past it, else the
/// newest prerelease past it. `None` for a stable `current`, which is never
/// moved onto a prerelease.
pub fn prerelease_target<'a>(current: &str, versions: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let c = Version::parse(current)?;
    if !c.prerelease {
        return None;
    }
    max_version(versions.into_iter().filter(|v| Version::parse(v).is_some_and(|v| v > c)))
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
    fn leading_zeros_move_the_breaking_part() {
        assert_eq!(compare("0.12.28", "0.13.5"), Status::Major);
        assert_eq!(compare("0.12.28", "0.12.30"), Status::Minor);
        assert_eq!(compare("0.0.9", "0.0.12"), Status::Major);
        assert_eq!(release_line("0.0.9").as_deref(), Some("0.0.9"));
        assert_eq!(release_line("^0.13.4").as_deref(), Some("0.13"));
        assert_eq!(release_line("9.4.1").as_deref(), Some("9"));
        let versions: Vec<String> = ["0.0.10", "0.0.12", "0.1.0"].iter().map(|s| s.to_string()).collect();
        assert_eq!(safe_target("0.0.9", &versions), None);
        assert_eq!(patch_target("0.0.9", &versions), None);
    }

    #[test]
    fn prereleases_order_by_their_tags() {
        assert!(Version::parse("2.0.0-rc.25").unwrap() > Version::parse("2.0.0-rc.22").unwrap());
        assert!(Version::parse("2.0.0-rc.10").unwrap() > Version::parse("2.0.0-rc.9").unwrap());
        assert!(Version::parse("2.0.0-rc.1").unwrap() > Version::parse("2.0.0-beta.4").unwrap());
        assert!(Version::parse("2.0.0-alpha.1").unwrap() > Version::parse("2.0.0-alpha").unwrap());
        assert_eq!(compare("2.0.0-rc.22", "2.0.0-rc.25"), Status::Major);
        assert_eq!(compare("2.0.0-rc.22", "2.0.0-rc.22"), Status::UpToDate);
        let versions = ["1.0.5", "2.0.0-rc.21", "2.0.0-rc.22", "2.0.0-rc.25"];
        assert_eq!(prerelease_target("2.0.0-rc.22", versions).as_deref(), Some("2.0.0-rc.25"));
        assert_eq!(prerelease_target("2.0.0-rc.22", ["1.0.5", "2.0.0-rc.25", "2.0.0"]).as_deref(), Some("2.0.0"));
        assert_eq!(prerelease_target("1.0.5", versions), None);
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

    #[test]
    fn patch_target_stays_on_the_minor_line() {
        let versions: Vec<String> = ["1.2.3", "1.2.9", "1.3.0", "2.0.0", "1.2.10-beta.1"].iter().map(|s| s.to_string()).collect();
        assert_eq!(patch_target("1.2.3", &versions).as_deref(), Some("1.2.9"));
        assert_eq!(patch_target("1.3.0", &versions), None);
    }
}
