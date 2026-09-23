//! Latest-version lookups, one per package no matter how many projects use it.

use std::time::Duration;

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};

use crate::model::Ecosystem;
use crate::version::{Version, max_version};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackageInfo {
    pub latest: Option<String>,
    /// GitHub Actions only: (tag, commit SHA) pairs, so SHA pins can be named.
    pub tags: Vec<(String, String)>,
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
        Ecosystem::GithubActions => github_tags(name).await,
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
    }
    let url = format!("https://registry.npmjs.org/{}", name.replace('/', "%2f"));
    let doc: Packument = get(http, &url, Some("application/vnd.npm.install-v1+json")).await?.json().await?;
    Ok(PackageInfo { latest: doc.dist_tags.get("latest").cloned(), ..Default::default() })
}

/// Uses the sparse index (static files behind a CDN) rather than the crates.io
/// API, which asks tools to stay under one request per second.
async fn crates(http: &reqwest::Client, name: &str) -> anyhow::Result<PackageInfo> {
    #[derive(Deserialize)]
    struct Entry {
        vers: String,
        #[serde(default)]
        yanked: bool,
    }
    let lower = name.to_ascii_lowercase();
    let prefix = match lower.len() {
        1 => "1".to_string(),
        2 => "2".to_string(),
        3 => format!("3/{}", &lower[..1]),
        _ => format!("{}/{}", &lower[..2], &lower[2..4]),
    };
    let body = get(http, &format!("https://index.crates.io/{prefix}/{lower}"), None).await?.text().await?;
    let versions: Vec<String> = body
        .lines()
        .filter_map(|l| serde_json::from_str::<Entry>(l).ok())
        .filter(|e| !e.yanked)
        .map(|e| e.vers)
        .collect();
    Ok(PackageInfo { latest: max_version(versions.iter().map(String::as_str)), ..Default::default() })
}

async fn nuget(http: &reqwest::Client, name: &str) -> anyhow::Result<PackageInfo> {
    #[derive(Deserialize)]
    struct Index {
        versions: Vec<String>,
    }
    let url = format!("https://api.nuget.org/v3-flatcontainer/{}/index.json", name.to_ascii_lowercase());
    let index: Index = get(http, &url, None).await?.json().await?;
    Ok(PackageInfo { latest: max_version(index.versions.iter().map(String::as_str)), ..Default::default() })
}

/// `git ls-remote` does not count against GitHub's 60-requests-an-hour API
/// limit and returns the commit behind every tag in one call.
async fn github_tags(name: &str) -> anyhow::Result<PackageInfo> {
    let url = format!("https://github.com/{name}.git");
    let mut cmd = tokio::process::Command::new("git");
    cmd.args(["ls-remote", "--tags", &url]).env("GIT_TERMINAL_PROMPT", "0").kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash from the GUI app
    let output = tokio::time::timeout(Duration::from_secs(30), cmd.output()).await.context("git ls-remote timed out")??;
    if !output.status.success() {
        return Err(anyhow!("git ls-remote failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }

    // Annotated tags list twice: the tag object, then the commit as `tag^{}`.
    // The peeled line wins because pins point at commits.
    let mut tags: Vec<(String, String)> = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
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
    Ok(PackageInfo { latest, tags })
}
