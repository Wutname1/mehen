//! Vulnerability lookups against the OSV database (osv.dev).

use futures::{StreamExt, stream};
use serde::Deserialize;
use serde_json::json;

use crate::model::{Ecosystem, FixedIn, Vulnerability};
use crate::store::Store;

const BATCH: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Query {
    pub ecosystem: Ecosystem,
    pub name: String,
    pub version: String,
}

/// Returns the vulnerability IDs affecting each query, in query order.
pub async fn query_batch(http: &reqwest::Client, queries: &[Query]) -> anyhow::Result<Vec<Vec<String>>> {
    #[derive(Deserialize)]
    struct Response {
        results: Vec<ResultEntry>,
    }
    #[derive(Deserialize)]
    struct ResultEntry {
        #[serde(default)]
        vulns: Vec<IdOnly>,
    }
    #[derive(Deserialize)]
    struct IdOnly {
        id: String,
    }

    let mut out = Vec::with_capacity(queries.len());
    for chunk in queries.chunks(BATCH) {
        let body = json!({
            "queries": chunk.iter().map(|q| json!({
                "package": { "name": q.name, "ecosystem": q.ecosystem.osv_name() },
                "version": q.version,
            })).collect::<Vec<_>>()
        });
        let response: Response = http.post("https://api.osv.dev/v1/querybatch").json(&body).send().await?.error_for_status()?.json().await?;
        out.extend(response.results.into_iter().map(|r| r.vulns.into_iter().map(|v| v.id).collect()));
    }
    Ok(out)
}

/// Advisory details, from the store when cached. Returns the advisories and
/// how many had to be fetched.
pub async fn details(http: &reqwest::Client, store: &Store, ids: Vec<String>) -> (Vec<Vulnerability>, usize) {
    let (cached, missing): (Vec<_>, Vec<_>) = ids.into_iter().map(|id| (store.advisory(&id), id)).partition(|(hit, _)| hit.is_some());
    let fetched_count = missing.len();
    let fetched: Vec<Vulnerability> = stream::iter(missing)
        .map(|(_, id)| async move {
            match detail(http, &id).await {
                Ok(vuln) => {
                    store.put_advisory(&vuln);
                    vuln
                }
                Err(_) => placeholder(&id),
            }
        })
        .buffer_unordered(8)
        .collect()
        .await;
    (cached.into_iter().filter_map(|(hit, _)| hit).chain(fetched).collect(), fetched_count)
}

async fn detail(http: &reqwest::Client, id: &str) -> anyhow::Result<Vulnerability> {
    #[derive(Deserialize)]
    struct Osv {
        id: String,
        #[serde(default)]
        summary: Option<String>,
        #[serde(default)]
        details: Option<String>,
        #[serde(default)]
        aliases: Vec<String>,
        #[serde(default)]
        affected: Vec<Affected>,
        #[serde(default)]
        database_specific: Option<serde_json::Value>,
    }
    #[derive(Deserialize)]
    struct Affected {
        package: Option<Package>,
        #[serde(default)]
        ranges: Vec<Range>,
    }
    #[derive(Deserialize)]
    struct Package {
        name: String,
        ecosystem: String,
    }
    #[derive(Deserialize)]
    struct Range {
        #[serde(default)]
        events: Vec<serde_json::Value>,
    }

    let osv: Osv = http.get(format!("https://api.osv.dev/v1/vulns/{id}")).send().await?.error_for_status()?.json().await?;
    let summary = osv
        .summary
        .filter(|s| !s.is_empty())
        .or_else(|| osv.details.and_then(|d| d.lines().next().map(|l| l.chars().take(200).collect())))
        .unwrap_or_default();
    let severity = osv.database_specific.as_ref().and_then(|d| d["severity"].as_str()).map(str::to_uppercase);

    let mut fixed: Vec<FixedIn> = Vec::new();
    for affected in osv.affected {
        let Some(pkg) = affected.package else { continue };
        let Some(ecosystem) = from_osv_name(&pkg.ecosystem) else { continue };
        let versions: Vec<String> =
            affected.ranges.iter().flat_map(|r| r.events.iter()).filter_map(|e| e["fixed"].as_str().map(str::to_string)).collect();
        match fixed.iter_mut().find(|f| f.ecosystem == ecosystem && f.name == pkg.name) {
            Some(f) => f.versions.extend(versions),
            None => fixed.push(FixedIn { ecosystem, name: pkg.name, versions }),
        }
    }

    Ok(Vulnerability { url: format!("https://osv.dev/vulnerability/{}", osv.id), id: osv.id, aliases: osv.aliases, summary, severity, fixed })
}

fn placeholder(id: &str) -> Vulnerability {
    Vulnerability {
        id: id.to_string(),
        aliases: Vec::new(),
        summary: "Details could not be loaded".into(),
        severity: None,
        url: format!("https://osv.dev/vulnerability/{id}"),
        fixed: Vec::new(),
    }
}

fn from_osv_name(name: &str) -> Option<Ecosystem> {
    match name {
        "npm" => Some(Ecosystem::Npm),
        "crates.io" => Some(Ecosystem::Cargo),
        "NuGet" => Some(Ecosystem::Nuget),
        "GitHub Actions" => Some(Ecosystem::GithubActions),
        _ => None,
    }
}
