//! Which tests or compiler errors a failed check reports, so a run can tell
//! failures an update added from ones the project already had.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;

fn rx(pattern: &str) -> Regex {
    Regex::new(pattern).expect("failure patterns are valid")
}

/// One line per failing test or error, worded the same however often or in
/// whatever order the tool prints it: timings and line numbers dropped. Empty
/// when the output names nothing it recognises, which compares as unknown.
pub fn signatures(output: &str) -> BTreeSet<String> {
    static ANSI: LazyLock<Regex> = LazyLock::new(|| rx(r"\x1b\[[0-9;]*[A-Za-z]"));
    static TIMING: LazyLock<Regex> = LazyLock::new(|| rx(r"\s*\(\d+(?:\.\d+)?\s*m?s\)\s*$"));
    // node --test, vitest, jest and mocha marks; TAP's `not ok`.
    static MARKED: LazyLock<Regex> = LazyLock::new(|| rx(r"^(?:[✖×✕✗]|not ok \d+ -)\s+(.+)$"));
    static JEST_FILE: LazyLock<Regex> = LazyLock::new(|| rx(r"^FAIL\s+(\S+)"));
    static JEST_TEST: LazyLock<Regex> = LazyLock::new(|| rx(r"^●\s+(.+)$"));
    static CARGO_TEST: LazyLock<Regex> = LazyLock::new(|| rx(r"^test (\S+) \.\.\. FAILED$"));
    static PYTEST: LazyLock<Regex> = LazyLock::new(|| rx(r"^FAILED\s+(\S+)"));
    static TSC: LazyLock<Regex> = LazyLock::new(|| rx(r"^(\S.*?)(?:\(\d+,\d+\)|:\d+:\d+)\s*[:-]\s*error (TS\d+: .+)$"));
    static DOTNET: LazyLock<Regex> = LazyLock::new(|| rx(r"^(\S.*?)\(\d+,\d+\): error (CS\d+: .+?)(?: \[.*\])?$"));
    static RUSTC: LazyLock<Regex> = LazyLock::new(|| rx(r"^(error(?:\[E\d+\])?: .+)$"));

    let mut found = BTreeSet::new();
    for raw in output.lines() {
        let line = ANSI.replace_all(raw, "");
        let line = line.trim();
        let hit = if let Some(c) = MARKED.captures(line) {
            let name = TIMING.replace(&c[1], "").trim().to_string();
            // node --test heads its summary with "✖ failing tests:".
            (!name.ends_with("failing tests:")).then(|| format!("test: {name}"))
        } else if let Some(c) = CARGO_TEST.captures(line) {
            Some(format!("test: {}", &c[1]))
        } else if let Some(c) = JEST_FILE.captures(line).or_else(|| PYTEST.captures(line)) {
            Some(format!("test: {}", &c[1]))
        } else if let Some(c) = JEST_TEST.captures(line) {
            Some(format!("test: {}", c[1].trim()))
        } else if let Some(c) = TSC.captures(line).or_else(|| DOTNET.captures(line)) {
            Some(format!("{}: {}", c[1].replace('\\', "/"), &c[2]))
        } else {
            RUSTC.captures(line).filter(|c| !c[1].starts_with("error: could not compile") && !c[1].starts_with("error: aborting")).map(|c| c[1].to_string())
        };
        found.extend(hit);
    }
    found
}

/// A few of `failures` for a sentence: "a, b and 3 more".
pub fn list(failures: &BTreeSet<String>) -> String {
    let names: Vec<&str> = failures.iter().map(|f| f.strip_prefix("test: ").unwrap_or(f)).collect();
    match names.len() {
        0 => String::new(),
        1..=3 => names.join("; "),
        n => format!("{}; and {} more", names[..2].join("; "), n - 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_test_failures_read_the_same_before_and_after() {
        let before = "✔ parseRange handles the common forms (0.3017ms)\n✖ resolves the Copilot CLI bundled by the official SDK (1.5675ms)\nℹ fail 1\n\n✖ failing tests:\n\ntest at tests\\copilotAuth.test.mjs:58:1\n✖ resolves the Copilot CLI bundled by the official SDK (1.5675ms)\n";
        let after = "✖ resolves the Copilot CLI bundled by the official SDK (2.1ms)\n✖ failing tests:\n";
        assert_eq!(signatures(before), signatures(after));
        assert_eq!(signatures(before).into_iter().collect::<Vec<_>>(), ["test: resolves the Copilot CLI bundled by the official SDK"]);
        let worse = format!("{after}✖ serves 206 for a range request (3ms)\n");
        assert!(!signatures(&worse).is_subset(&signatures(before)), "a new failing test is new");
    }

    #[test]
    fn compiler_and_runner_failures() {
        let out = "src/lib/sentry.ts(51,5): error TS2353: Object literal may only specify known properties.\n\
                   test util::parses ... FAILED\n\
                   error[E0308]: mismatched types\n\
                   error: could not compile `x` (lib) due to 1 previous error\n\
                   FAILED tests/test_api.py::test_login - AssertionError\n\
                   \x1b[31mFAIL\x1b[39m src/app.test.ts\n";
        let s: Vec<String> = signatures(out).into_iter().collect();
        assert_eq!(
            s,
            [
                "error[E0308]: mismatched types",
                "src/lib/sentry.ts: TS2353: Object literal may only specify known properties.",
                "test: src/app.test.ts",
                "test: tests/test_api.py::test_login",
                "test: util::parses",
            ]
        );
        assert!(signatures("npm ERR! something went wrong").is_empty());
    }
}
