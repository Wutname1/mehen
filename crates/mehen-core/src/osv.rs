//! Vulnerability lookups against the OSV database (osv.dev).

use futures::{StreamExt, stream};
use serde::Deserialize;
use serde_json::json;

use crate::model::{AffectedRange, Ecosystem, FixedIn, Vulnerability};
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
        #[serde(rename = "type", default)]
        kind: String,
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
        // Git ranges name commits, not versions a project can have.
        let ranges: Vec<&Range> = affected.ranges.iter().filter(|r| r.kind != "GIT").collect();
        let versions: Vec<String> = ranges.iter().flat_map(|r| r.events.iter()).filter_map(|e| e["fixed"].as_str().map(str::to_string)).collect();
        let spans: Vec<AffectedRange> = ranges.iter().flat_map(|r| affected_ranges(&r.events)).collect();
        match fixed.iter_mut().find(|f| f.ecosystem == ecosystem && f.name == pkg.name) {
            Some(f) => {
                f.versions.extend(versions);
                f.ranges.extend(spans);
            }
            None => fixed.push(FixedIn { ecosystem, name: pkg.name, versions, ranges: spans }),
        }
    }

    Ok(Vulnerability { url: format!("https://osv.dev/vulnerability/{}", osv.id), id: osv.id, aliases: osv.aliases, summary, severity, fixed })
}

/// OSV events, in order, as spans: each `introduced` opens one, and the next
/// `fixed` or `last_affected` closes it.
fn affected_ranges(events: &[serde_json::Value]) -> Vec<AffectedRange> {
    let mut spans = Vec::new();
    let mut open: Option<AffectedRange> = None;
    for event in events {
        if let Some(v) = event["introduced"].as_str() {
            if let Some(unclosed) = open.take() {
                spans.push(unclosed);
            }
            open = Some(AffectedRange { introduced: (v != "0").then(|| v.to_string()), ..Default::default() });
        } else if let Some(v) = event["fixed"].as_str() {
            let mut span = open.take().unwrap_or_default();
            span.fixed = Some(v.to_string());
            spans.push(span);
        } else if let Some(v) = event["last_affected"].as_str() {
            let mut span = open.take().unwrap_or_default();
            span.last_affected = Some(v.to_string());
            spans.push(span);
        }
    }
    spans.extend(open);
    spans
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
        "Go" => Some(Ecosystem::Go),
        "PyPI" => Some(Ecosystem::Pypi),
        "Pub" => Some(Ecosystem::Pub),
        "Packagist" => Some(Ecosystem::Packagist),
        "RubyGems" => Some(Ecosystem::RubyGems),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_become_spans() {
        let spans = affected_ranges(&[json!({"introduced": "0"}), json!({"fixed": "15.1.1"}), json!({"introduced": "16.0.0"}), json!({"fixed": "16.1.1"})]);
        assert_eq!(
            spans,
            vec![
                AffectedRange { introduced: None, fixed: Some("15.1.1".into()), last_affected: None },
                AffectedRange { introduced: Some("16.0.0".into()), fixed: Some("16.1.1".into()), last_affected: None },
            ]
        );
        let open = affected_ranges(&[json!({"introduced": "2.0.0"})]);
        assert_eq!(open, vec![AffectedRange { introduced: Some("2.0.0".into()), ..Default::default() }], "no fix yet");
        let last = affected_ranges(&[json!({"introduced": "0"}), json!({"last_affected": "3.4.0"})]);
        assert_eq!(last[0].last_affected.as_deref(), Some("3.4.0"));
    }
}
