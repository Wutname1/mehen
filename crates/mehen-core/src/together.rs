//! Packages that can only move together. A framework like Angular pins its
//! parts to each other (`@angular/core 22.2.0` needs `@angular/compiler
//! 22.2.0`, and the installed `@angular/platform-browser-dynamic 16.2.12`
//! needs `@angular/compiler 16.2.12`), so no single part can be updated on
//! its own. This finds the set that has to move, and to which versions.

use std::collections::{BTreeMap, HashMap};

use crate::compat::semver_satisfies;
use crate::version::Version;

/// One of the project's own packages, as the search sees it.
#[derive(Debug, Clone, Default)]
pub struct Pkg {
    pub current: String,
    /// Stable versions at or above `current`, newest first.
    pub versions: Vec<String>,
    /// The peer ranges each version asks for, only for the project's own packages.
    pub peers: HashMap<String, Vec<(String, String)>>,
}

impl Pkg {
    fn peers_of(&self, version: &str) -> &[(String, String)] {
        self.peers.get(version).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// Stops a search that is not settling (and keeps a "group" of half the
/// project out of the list).
const MAX_ROUNDS: usize = 64;
const MAX_MEMBERS: usize = 40;

/// The packages that must move with `seed` for it to reach its newest
/// version `allowed` accepts, and where each goes. `allowed(name, version)`
/// is everything besides peers: holds, Node, and so on. `None` when no such
/// set exists, or when `seed` can move alone.
pub fn resolve(seed: &str, pkgs: &HashMap<String, Pkg>, allowed: &dyn Fn(&str, &str) -> bool) -> Option<BTreeMap<String, String>> {
    let first = pkgs.get(seed)?.versions.iter().find(|v| allowed(seed, v))?.clone();
    if Some(&first) == pkgs.get(seed).map(|p| &p.current) {
        return None;
    }
    let mut moved: BTreeMap<String, String> = BTreeMap::from([(seed.to_string(), first)]);
    // A fixed order, so the same project always gives the same group.
    let mut names: Vec<&String> = pkgs.keys().collect();
    names.sort();
    for _ in 0..MAX_ROUNDS {
        let version_of = |name: &str, moved: &BTreeMap<String, String>| moved.get(name).cloned().unwrap_or_else(|| pkgs[name].current.clone());
        // The first peer range that some package's version does not meet.
        let conflict = names.iter().find_map(|owner| {
            let pkg = &pkgs[*owner];
            let at = version_of(owner, &moved);
            pkg.peers_of(&at)
                .iter()
                .filter(|(peer, _)| pkgs.contains_key(peer))
                .find(|(peer, range)| !semver_satisfies(&version_of(peer, &moved), range))
                .map(|(peer, range)| (owner.to_string(), peer.clone(), range.clone()))
        });
        let Some((owner, peer, range)) = conflict else {
            return (moved.len() > 1).then_some(moved);
        };
        // Move whichever side has not moved yet: the peer up into the range,
        // or the owner up to a version that accepts where the moved packages
        // are going. What else that version needs becomes the next conflict.
        let (name, pick) = if !moved.contains_key(&peer) {
            let pick = pkgs[&peer].versions.iter().find(|v| allowed(&peer, v) && semver_satisfies(v, &range)).cloned();
            (peer, pick)
        } else {
            let accepts = |v: &str| pkgs[&owner].peers_of(v).iter().filter(|(p, _)| moved.contains_key(p)).all(|(p, r)| semver_satisfies(&moved[p], r));
            let pick = pkgs[&owner].versions.iter().find(|v| allowed(&owner, v) && accepts(v)).cloned();
            (owner, pick)
        };
        let pick = pick?;
        if moved.get(&name) == Some(&pick) || pick == pkgs[&name].current && !moved.contains_key(&name) {
            // Nothing new to try: this set cannot settle.
            return None;
        }
        moved.insert(name, pick);
        if moved.len() > MAX_MEMBERS {
            return None;
        }
    }
    None
}

/// Stable versions from `current` up, newest first, for `Pkg::versions`.
pub fn candidates(current: &str, versions: &[String]) -> Vec<String> {
    let Some(floor) = Version::parse(current) else { return Vec::new() };
    let mut out: Vec<(Version, String)> =
        versions.iter().filter_map(|v| Version::parse(v).map(|p| (p, v.clone()))).filter(|(p, _)| !p.prerelease && *p >= floor).collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.into_iter().map(|(_, v)| v).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(current: &str, versions: &[&str], peers: &[(&str, &[(&str, &str)])]) -> Pkg {
        Pkg {
            current: current.into(),
            versions: versions.iter().map(|v| v.to_string()).collect(),
            peers: peers.iter().map(|(v, list)| (v.to_string(), list.iter().map(|(n, r)| (n.to_string(), r.to_string())).collect())).collect(),
        }
    }

    fn angular() -> HashMap<String, Pkg> {
        let mut pkgs = HashMap::new();
        pkgs.insert("@angular/core".into(), pkg("16.2.12", &["22.2.0", "16.2.12"], &[("22.2.0", &[("@angular/compiler", "22.2.0"), ("zone.js", "~0.15.0")]), ("16.2.12", &[("zone.js", "~0.13.0")])]));
        pkgs.insert("@angular/compiler".into(), pkg("16.2.12", &["22.2.0", "16.2.12"], &[("22.2.0", &[("@angular/core", "22.2.0")])]));
        pkgs.insert(
            "@angular/platform-browser-dynamic".into(),
            pkg("16.2.12", &["22.2.0", "16.2.12"], &[("22.2.0", &[("@angular/core", "22.2.0"), ("@angular/compiler", "22.2.0")]), ("16.2.12", &[("@angular/core", "16.2.12"), ("@angular/compiler", "16.2.12")])]),
        );
        pkgs.insert("zone.js".into(), pkg("0.13.3", &["0.16.3", "0.15.1", "0.13.3"], &[]));
        // Its newer version also needs a package that has not moved yet.
        pkgs.insert("@angular/common".into(), pkg("16.2.12", &["22.2.0", "16.2.12"], &[("22.2.0", &[("@angular/core", "22.2.0")]), ("16.2.12", &[("@angular/core", "16.2.12")])]));
        pkgs.insert(
            "@angular/cdk".into(),
            pkg("16.2.14", &["22.2.0", "16.2.14"], &[("22.2.0", &[("@angular/core", "^22.0.0"), ("@angular/common", "^22.0.0")]), ("16.2.14", &[("@angular/core", "^16.0.0 || ^17.0.0"), ("@angular/common", "^16.0.0 || ^17.0.0")])]),
        );
        pkgs.insert("rxjs".into(), pkg("7.8.1", &["7.8.2", "7.8.1"], &[]));
        pkgs
    }

    #[test]
    fn a_framework_moves_as_one() {
        let pkgs = angular();
        let moved = resolve("@angular/core", &pkgs, &|_, _| true).unwrap();
        let expected: BTreeMap<String, String> =
            [("@angular/cdk", "22.2.0"), ("@angular/common", "22.2.0"), ("@angular/compiler", "22.2.0"), ("@angular/core", "22.2.0"), ("@angular/platform-browser-dynamic", "22.2.0"), ("zone.js", "0.15.1")]
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect();
        assert_eq!(moved, expected, "zone.js goes to the newest the new core accepts, not the newest there is; rxjs stays");
    }

    #[test]
    fn nothing_when_a_hold_or_runtime_blocks_a_member() {
        let pkgs = angular();
        let no_new_zone = |name: &str, v: &str| !(name == "zone.js" && v != "0.13.3");
        assert_eq!(resolve("@angular/core", &pkgs, &no_new_zone), None);
    }

    #[test]
    fn nothing_for_a_package_that_can_move_alone() {
        let pkgs = angular();
        assert_eq!(resolve("rxjs", &pkgs, &|_, _| true), None);
    }

    #[test]
    fn candidates_are_stable_and_newest_first() {
        let versions: Vec<String> = ["16.2.12", "22.2.0", "22.3.0-rc.1", "17.0.0", "15.0.0"].iter().map(|v| v.to_string()).collect();
        assert_eq!(candidates("16.2.12", &versions), ["22.2.0", "17.0.0", "16.2.12"]);
    }
}
