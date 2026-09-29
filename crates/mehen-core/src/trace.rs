//! A shared, append-only record of what every Mehen process does with the
//! database, kept to find out what damages it. The app and each background
//! run write to the same `db-trace.log`, one line per event:
//!
//! `2026-09-28T20:41:12.345Z pid=1234 app  opened: journal=truncate ...`
//!
//! Besides Mehen's own events it carries SQLite's error log (corruption
//! reports with their source line, WAL recovery on open, locking trouble).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// The log moves to `db-trace.1.log` past this size, so two files at most.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

struct Trace {
    path: PathBuf,
    role: String,
    lock: Mutex<()>,
}

static TRACE: OnceLock<Trace> = OnceLock::new();

/// Starts tracing into `dir/db-trace.log` as `role` ("app", "background").
/// Call once, before the first database is opened: SQLite only accepts its
/// log callback before it starts up.
pub fn init(dir: &Path, role: &str) {
    if TRACE.set(Trace { path: dir.join("db-trace.log"), role: role.to_string(), lock: Mutex::new(()) }).is_err() {
        return;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    line(format!("start: version={} sqlite={} exe={exe} args={args:?}", env!("CARGO_PKG_VERSION"), rusqlite::version()));
    // SAFETY: called before any connection exists (see above); SQLite refuses
    // with SQLITE_MISUSE otherwise, which is only logged.
    if let Err(e) = unsafe { rusqlite::trace::config_log(Some(sqlite_log)) } {
        line(format!("sqlite log not attached: {e}"));
    }
}

fn sqlite_log(code: std::ffi::c_int, message: &str) {
    line(format!("sqlite[{code}]: {message}"));
}

/// Appends one event. Does nothing before `init`; never fails the caller.
pub fn line(event: impl AsRef<str>) {
    let Some(trace) = TRACE.get() else { return };
    let _guard = trace.lock.lock().unwrap_or_else(|e| e.into_inner());
    if std::fs::metadata(&trace.path).is_ok_and(|m| m.len() > MAX_BYTES) {
        let _ = std::fs::rename(&trace.path, trace.path.with_file_name("db-trace.1.log"));
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&trace.path) {
        let _ = writeln!(file, "{} pid={} {:<10} {}", now_utc(), std::process::id(), trace.role, event.as_ref().replace('\n', " | "));
    }
}

/// The last `lines` lines of the trace, oldest first, for attaching to a report.
pub fn tail(lines: usize) -> String {
    let Some(trace) = TRACE.get() else { return String::new() };
    let text = std::fs::read_to_string(&trace.path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// `2026-09-28T20:41:12.345Z`.
fn now_utc() -> String {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let (date, time) = civil(since.as_secs() as i64);
    format!("{date}T{time}.{:03}Z", since.subsec_millis())
}

/// `("2026-09-28", "20:41:12")` in UTC for seconds since 1970.
pub fn civil(secs: i64) -> (String, String) {
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (format!("{year:04}-{month:02}-{day:02}"), format!("{:02}:{:02}:{:02}", rest / 3600, rest % 3600 / 60, rest % 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), ("1970-01-01".into(), "00:00:00".into()));
        assert_eq!(civil(951_782_400), ("2000-02-29".into(), "00:00:00".into()));
        assert_eq!(civil(1_709_251_199), ("2024-02-29".into(), "23:59:59".into()));
    }
}
