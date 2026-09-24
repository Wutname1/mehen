//! Latest-version lookups, one per package no matter how many projects use it.

use std::time::Duration;

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};

use crate::compat::Requirement;
use crate::model::Ecosystem;
use crate::version::{Version, max_version};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackageInfo {
    pub latest: Option<String>,
    /// Every published version (tags for Actions), for picking safe targets.
    #[serde(default)]
    pub versions: Vec<String>,
    /// GitHub Actions only: (tag, commit SHA) pairs, so SHA pins can be named.
    pub tags: Vec<(String, String)>,
    /// What individual versions need from a project (frameworks, Rust, Node).
    #[serde(default)]
    pub requirements: Vec<(String, Requirement)>,
}

impl PackageInfo {
    /// Most specific version tag pointing at a commit: `v4.2.2` over `v4`.
    pub fn tag_for_commit(&self, sha: &str) -> Option<String> {
        self.tags
            .iter()
            .filter(|(_, commit)| commit.eq_ignore_ascii_case(sha))
            .filter_map(|(tag, _)| Version::parse(tag).map(|v| (v, tag)))
            .max_by(|a, b| a.0.parts.len().cmp(&b.0.parts.len()).then(a.0.cmp(&b.0)))
            .map(|(_, tag)| tag.clone())
    }
}

const MAX_RETRY_WAIT_SECS: u64 = 15;

/// GET with short backoff on 429 and 5xx. A Retry-After longer than
/// `MAX_RETRY_WAIT_SECS` fails straight away: waiting minutes per request
/// would stall the whole check.
async fn get(http: &reqwest::Client, url: &str, accept: Option<&str>) -> anyhow::Result<reqwest::Response> {
    let mut attempt = 0;
    loop {
        let mut request = http.get(url);
        if let Some(accept) = accept {
            request = request.header(reqwest::header::ACCEPT, accept);
        }
        let response = request.send().await?;
        let status = response.status();
        let retryable = status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
        let retry_after = response.headers().get(reqwest::header::RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
        let wait = retry_after.unwrap_or(1 << attempt);
        if !retryable || attempt >= 3 || wait > MAX_RETRY_WAIT_SECS {
            return Ok(response.error_for_status()?);
        }
        tokio::time::sleep(Duration::from_secs(wait)).await;
        attempt += 1;
    }
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("Mehen/", env!("CARGO_PKG_VERSION"), " (dependency manager)"))
        .timeout(Duration::from_secs(20))
        .build()
        .expect("http client")
}

pub async fn lookup(http: &reqwest::Client, ecosystem: Ecosystem, name: &str) -> anyhow::Result<PackageInfo> {
    match ecosystem {
        Ecosystem::Npm => npm(http, name).await,
        Ecosystem::Cargo => crates(http, name).await,
        Ecosystem::Nuget => nuget(http, name).await,
        Ecosystem::GithubActions => github_tags(http, name).await,
        Ecosystem::Go => go_module(http, name).await,
    }
}

/// Uses the abbreviated package document (what the npm CLI fetches). It is
/// served from npm's CDN cache, while `/{name}/latest` goes to the origin and
/// is rate limited much sooner.
async fn npm(http: &reqwest::Client, name: &str) -> anyhow::Result<PackageInfo> {
    #[derive(Deserialize)]
    struct Packument {
        #[serde(rename = "dist-tags", default)]
        dist_tags: std::collections::HashMap<String, String>,
        #[serde(default)]
        versions: std::collections::HashMap<String, PackumentVersion>,
    }
    #[derive(Deserialize)]
    struct PackumentVersion {
        // Old packages sometimes publish `engines` as an array; keep it loose.
        #[serde(default)]
        engines: Option<serde_json::Value>,
        #[serde(rename = "peerDependencies", default)]
        peer_dependencies: Option<std::collections::HashMap<String, String>>,
    }
    let url = format!("https://registry.npmjs.org/{}", name.replace('/', "%2f"));
    let doc: Packument = get(http, &url, Some("application/vnd.npm.install-v1+json")).await?.json().await?;
    let mut requirements = Vec::new();
    for (v, meta) in &doc.versions {
        if let Some(range) = meta.engines.as_ref().and_then(|e| e.get("node")).and_then(|n| n.as_str()).map(str::trim) {
            if !range.is_empty() && range != "*" {
                requirements.push((v.clone(), Requirement::Node { range: range.to_string() }));
            }
        }
        if let Some(peers) = meta.peer_dependencies.as_ref().filter(|p| !p.is_empty()) {
            let mut peers: Vec<(String, String)> = peers.iter().map(|(k, r)| (k.clone(), r.clone())).collect();
            peers.sort();
            requirements.push((v.clone(), Requirement::Peers { peers }));
        }
    }
    Ok(PackageInfo { latest: doc.dist_tags.get("latest").cloned(), versions: doc.versions.into_keys().collect(), requirements, ..Default::default() })
}

/// The Go module proxy's list of tagged versions. A module with only
/// commit-based pseudo-versions lists nothing, so `@latest` fills in. The
/// proxy answers 410 as well as 404 for a module it cannot serve (a private
/// one, for example).
async fn go_module(http: &reqwest::Client, name: &str) -> anyhow::Result<PackageInfo> {
    let base = format!("https://proxy.golang.org/{}", crate::golang::proxy_escape(name));
    let not_found = |e: anyhow::Error| if e.to_string().contains("410") { anyhow!("404: {name} is not on the Go module proxy") } else { e };
    let list = get(http, &format!("{base}/@v/list"), None).await.map_err(not_found)?.text().await?;
    let mut versions: Vec<String> = list.lines().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string).collect();
    if versions.is_empty() {
        #[derive(Deserialize)]
        struct Latest {
            #[serde(rename = "Version")]
            version: String,
        }
        let latest: Latest = get(http, &format!("{base}/@latest"), None).await.map_err(not_found)?.json().await?;
        versions.push(latest.version);
    }
    Ok(PackageInfo { latest: max_version(versions.iter().map(String::as_str)), versions, ..Default::default() })
}

/// Uses the sparse index (static files behind a CDN) rather than the crates.io
/// API, which asks tools to stay under one request per second.
async fn crates(http: &reqwest::Client, name: &str) -> anyhow::Result<PackageInfo> {
    #[derive(Deserialize)]
    struct Entry {
        vers: String,
        #[serde(default)]
        yanked: bool,
        #[serde(default)]
        rust_version: Option<String>,
    }
    let lower = name.to_ascii_lowercase();
    let prefix = match lower.len() {
        1 => "1".to_string(),
        2 => "2".to_string(),
        3 => format!("3/{}", &lower[..1]),
        _ => format!("{}/{}", &lower[..2], &lower[2..4]),
    };
    let body = get(http, &format!("https://index.crates.io/{prefix}/{lower}"), None).await?.text().await?;
    let entries: Vec<Entry> = body.lines().filter_map(|l| serde_json::from_str::<Entry>(l).ok()).filter(|e| !e.yanked).collect();
    let requirements = entries
        .iter()
        .filter_map(|e| e.rust_version.as_ref().map(|r| (e.vers.clone(), Requirement::Rust { version: r.clone() })))
        .collect();
    let versions: Vec<String> = entries.into_iter().map(|e| e.vers).collect();
    Ok(PackageInfo { latest: max_version(versions.iter().map(String::as_str)), versions, requirements, ..Default::default() })
}

/// Uses the registration API rather than the flat version list because it
/// also says which target frameworks each version ships for. Small packages
/// inline every version in the index; large ones split into pages.
async fn nuget(http: &reqwest::Client, name: &str) -> anyhow::Result<PackageInfo> {
    #[derive(Deserialize)]
    struct Index {
        items: Vec<Page>,
    }
    #[derive(Deserialize)]
    struct Page {
        #[serde(rename = "@id")]
        id: String,
        #[serde(default)]
        items: Option<Vec<Leaf>>,
    }
    #[derive(Deserialize)]
    struct PageBody {
        items: Vec<Leaf>,
    }
    #[derive(Deserialize)]
    struct Leaf {
        #[serde(rename = "catalogEntry")]
        entry: Entry,
    }
    #[derive(Deserialize)]
    struct Entry {
        version: String,
        #[serde(default = "listed_default")]
        listed: bool,
        #[serde(rename = "dependencyGroups", default)]
        groups: Vec<Group>,
    }
    #[derive(Deserialize)]
    struct Group {
        #[serde(rename = "targetFramework", default)]
        framework: Option<String>,
    }
    fn listed_default() -> bool {
        true
    }

    let url = format!("https://api.nuget.org/v3/registration5-gz-semver2/{}/index.json", name.to_ascii_lowercase());
    let index: Index = get(http, &url, None).await?.json().await?;
    // Long-lived packages have dozens of pages; they are fetched together.
    let pages = futures::future::try_join_all(index.items.into_iter().map(|page| async move {
        match page.items {
            Some(items) => anyhow::Ok(items),
            None => Ok(get(http, &page.id, None).await?.json::<PageBody>().await?.items),
        }
    }))
    .await?;
    let leaves: Vec<Leaf> = pages.into_iter().flatten().collect();

    let mut versions = Vec::new();
    let mut requirements = Vec::new();
    for Leaf { entry } in leaves.into_iter().filter(|l| l.entry.listed) {
        // Build metadata ("+abc") is not part of the version NuGet restores.
        let version = entry.version.split('+').next().unwrap_or(&entry.version).to_string();
        let frameworks: Vec<String> = entry.groups.into_iter().filter_map(|g| g.framework).filter(|f| !f.is_empty()).collect();
        if !frameworks.is_empty() {
            requirements.push((version.clone(), Requirement::Frameworks { frameworks }));
        }
        versions.push(version);
    }
    Ok(PackageInfo { latest: max_version(versions.iter().map(String::as_str)), versions, requirements, ..Default::default() })
}

/// Git's own ref listing does not count against GitHub's 60-requests-an-hour
/// API limit and returns the commit behind every tag in one call. It is read
/// over the shared HTTP client; `git ls-remote` is the fallback.
async fn github_tags(http: &reqwest::Client, name: &str) -> anyhow::Result<PackageInfo> {
    let refs = match advertised_refs(http, name).await {
        Ok(refs) => refs,
        Err(e) if e.to_string().contains("404") => return Err(e),
        Err(_) => ls_remote(name).await?,
    };
    Ok(tags_from_refs(&refs))
}

/// The ref advertisement from `info/refs`: pkt-lines of `<sha> <ref>`.
async fn advertised_refs(http: &reqwest::Client, name: &str) -> anyhow::Result<String> {
    let response = http.get(format!("https://github.com/{name}.git/info/refs?service=git-upload-pack")).send().await?;
    // GitHub answers 401 for a repository that does not exist.
    if matches!(response.status().as_u16(), 401 | 404) {
        return Err(anyhow!("404: repository not found"));
    }
    parse_advertisement(&response.error_for_status()?.bytes().await?)
}

fn parse_advertisement(body: &[u8]) -> anyhow::Result<String> {
    let mut refs = String::new();
    let mut rest: &[u8] = body;
    while rest.len() >= 4 {
        let len = usize::from_str_radix(std::str::from_utf8(&rest[..4])?, 16)?;
        if len == 0 {
            rest = &rest[4..];
            continue;
        }
        anyhow::ensure!(len >= 4 && len <= rest.len(), "malformed ref advertisement");
        let line = String::from_utf8_lossy(&rest[4..len]);
        // The first ref carries capabilities after a NUL.
        let line = line.split('\0').next().unwrap_or("").trim_end();
        if let Some((sha, reference)) = line.split_once(' ') {
            if reference.starts_with("refs/") {
                refs.push_str(&format!("{sha}\t{reference}\n"));
            }
        }
        rest = &rest[len..];
    }
    anyhow::ensure!(!refs.is_empty(), "no refs in the advertisement");
    Ok(refs)
}

async fn ls_remote(name: &str) -> anyhow::Result<String> {
    let url = format!("https://github.com/{name}.git");
    let mut cmd = tokio::process::Command::new("git");
    cmd.args(["ls-remote", "--tags", &url]).env("GIT_TERMINAL_PROMPT", "0").kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash from the GUI app
    let output = tokio::time::timeout(Duration::from_secs(30), cmd.output()).await.context("git ls-remote timed out")??;
    if !output.status.success() {
        return Err(anyhow!("git ls-remote failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `<sha>\t<ref>` lines to tags. Annotated tags list twice: the tag object,
/// then the commit as `tag^{}`; the peeled line wins because pins point at commits.
fn tags_from_refs(refs: &str) -> PackageInfo {
    let mut tags: Vec<(String, String)> = Vec::new();
    for line in refs.lines() {
        let Some((sha, reference)) = line.split_once('\t') else { continue };
        let Some(tag) = reference.strip_prefix("refs/tags/") else { continue };
        match tag.strip_suffix("^{}") {
            Some(peeled) => match tags.iter_mut().find(|(t, _)| t == peeled) {
                Some(entry) => entry.1 = sha.to_string(),
                None => tags.push((peeled.to_string(), sha.to_string())),
            },
            None => tags.push((tag.to_string(), sha.to_string())),
        }
    }
    let latest = max_version(tags.iter().map(|(t, _)| t.as_str()));
    let versions = tags.iter().map(|(t, _)| t.clone()).filter(|t| Version::parse(t).is_some()).collect();
    PackageInfo { latest, versions, tags, requirements: Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(line: &str) -> String {
        format!("{:04x}{line}", line.len() + 4)
    }

    #[test]
    fn reads_tags_from_a_ref_advertisement() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let c = "c".repeat(40);
        let body = [
            pkt("# service=git-upload-pack\n"),
            "0000".to_string(),
            pkt(&format!("{a} HEAD\0multi_ack side-band-64k\n")),
            pkt(&format!("{a} refs/heads/main\n")),
            pkt(&format!("{b} refs/tags/v4.1.0\n")),
            pkt(&format!("{c} refs/tags/v4.1.0^{{}}\n")),
            pkt(&format!("{a} refs/tags/v3\n")),
            "0000".to_string(),
        ]
        .concat();
        let info = tags_from_refs(&parse_advertisement(body.as_bytes()).unwrap());
        assert_eq!(info.tags, vec![("v4.1.0".to_string(), c.clone()), ("v3".to_string(), a.clone())]);
        assert_eq!(info.latest.as_deref(), Some("v4.1.0"));
    }

    #[test]
    fn a_truncated_advertisement_is_an_error() {
        assert!(parse_advertisement(b"00ffshort").is_err());
        assert!(parse_advertisement(b"0000").is_err(), "no refs at all");
    }
}
