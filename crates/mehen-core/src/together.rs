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
    settle(seed, &first, pkgs, allowed).filter(|moved| moved.len() > 1)
}

/// Everything that has to move for `seed` to be at `version`, and where
/// each goes, including `seed` itself; just `seed` when nothing else
/// needs to. `None` when no set of versions fits.
pub fn settle(seed: &str, version: &str, pkgs: &HashMap<String, Pkg>, allowed: &dyn Fn(&str, &str) -> bool) -> Option<BTreeMap<String, String>> {
    pkgs.get(seed)?;
    if !allowed(seed, version) {
        return None;
    }
    let mut moved: BTreeMap<String, String> = BTreeMap::from([(seed.to_string(), version.to_string())]);
    // Packages released in step with the seed (on the same line today, like a
    // framework's parts) follow it to its exact version, else its line, so
    // picking an older line does not drag the others up to the newest one.
    let line = |v: &str| Version::parse(v).map(|p| if p.part(0) == 0 { (0, p.part(1)) } else { (p.part(0), 0) });
    let seed_line = line(&pkgs[seed].current);
    let pick = |name: &str, fits: &dyn Fn(&str) -> bool| -> Option<String> {
        let versions = &pkgs[name].versions;
        let first = |want: &dyn Fn(&str) -> bool| versions.iter().find(|v| want(v) && fits(v)).cloned();
        let in_step = seed_line.is_some() && line(&pkgs[name].current) == seed_line;
        (in_step.then(|| first(&|v| v == version).or_else(|| first(&|v| line(v) == line(version)))).flatten()).or_else(|| first(&|_| true))
    };
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
            return Some(moved);
        };
        // Move whichever side has not moved yet: the peer up into the range,
        // or the owner up to a version that accepts where the moved packages
        // are going. What else that version needs becomes the next conflict.
        let (name, pick) = if !moved.contains_key(&peer) {
            let pick = pick(&peer, &|v| allowed(&peer, v) && semver_satisfies(v, &range));
            (peer, pick)
        } else {
            let accepts = |v: &str| pkgs[&owner].peers_of(v).iter().filter(|(p, _)| moved.contains_key(p)).all(|(p, r)| semver_satisfies(&moved[p], r));
            let pick = pick(&owner, &|v| allowed(&owner, v) && accepts(v));
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
    fn a_chosen_older_line_brings_the_matching_versions() {
        let mut pkgs = angular();
        let core = pkgs.get_mut("@angular/core").unwrap();
        core.versions.insert(1, "21.2.3".into());
        core.peers.insert("21.2.3".into(), vec![("@angular/compiler".into(), "21.2.3".into()), ("zone.js".into(), "~0.15.0".into())]);
        let compiler = pkgs.get_mut("@angular/compiler").unwrap();
        compiler.versions.insert(1, "21.2.3".into());
        compiler.peers.insert("21.2.3".into(), vec![("@angular/core".into(), "21.2.3".into())]);
        for (name, peers) in [
            ("@angular/platform-browser-dynamic", vec![("@angular/core", "21.2.3"), ("@angular/compiler", "21.2.3")]),
            ("@angular/common", vec![("@angular/core", "21.2.3")]),
            ("@angular/cdk", vec![("@angular/core", "^21.0.0"), ("@angular/common", "^21.0.0")]),
        ] {
            let p = pkgs.get_mut(name).unwrap();
            p.versions.insert(1, "21.2.3".into());
            p.peers.insert("21.2.3".into(), peers.into_iter().map(|(n, r)| (n.to_string(), r.to_string())).collect());
        }
        // Newer compilers stopped naming the core they need; they still belong with 21.
        pkgs.get_mut("@angular/compiler").unwrap().peers.remove("22.2.0");
        let moved = settle("@angular/core", "21.2.3", &pkgs, &|_, _| true).unwrap();
        for name in ["@angular/compiler", "@angular/common", "@angular/platform-browser-dynamic", "@angular/cdk"] {
            assert_eq!(moved.get(name).map(String::as_str), Some("21.2.3"), "{name} goes to 21 with core, not the newest 22");
        }
        assert_eq!(settle("rxjs", "7.8.2", &pkgs, &|_, _| true).unwrap().len(), 1, "a package that moves alone is just itself");
        let no_core_21 = |name: &str, v: &str| !(name == "@angular/core" && v == "21.2.3");
        assert_eq!(settle("@angular/core", "21.2.3", &pkgs, &no_core_21), None, "a held or unusable choice is never offered");
    }

    #[test]
    fn candidates_are_stable_and_newest_first() {
        let versions: Vec<String> = ["16.2.12", "22.2.0", "22.3.0-rc.1", "17.0.0", "15.0.0"].iter().map(|v| v.to_string()).collect();
        assert_eq!(candidates("16.2.12", &versions), ["22.2.0", "17.0.0", "16.2.12"]);
    }
}
