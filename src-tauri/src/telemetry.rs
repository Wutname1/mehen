//! Crash and error reports from the app's own code, sent to GlitchTip.
//!
//! Release builds only. The client starts before the window so a panic during
//! startup is still caught, but nothing is sent until the saved choice has
//! been read and says yes: until then, and whenever it is off, events are
//! dropped before they leave.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::scrub;

const DSN: &str = "https://a637017bd857436a9bb9840d939e3351@errors.nyxservices.com/12";

/// Setting key: "false" turns error reports off. On when missing.
pub const ERROR_REPORTS: &str = "error_reports";

static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Keep the guard alive for the life of the app: dropping it sends what is
/// still queued, which is what gets a panic out before the process ends.
pub fn init() -> Option<sentry::ClientInitGuard> {
    if cfg!(debug_assertions) {
        return None;
    }
    let options = sentry::ClientOptions::new()
        .release(concat!("mehen@", env!("CARGO_PKG_VERSION")))
        .environment("production")
        // Paths and project names travel in panic messages: no extra
        // identifiers, and the text itself is scrubbed below.
        .send_default_pii(false)
        .attach_stacktrace(true)
        .before_send(|mut event: sentry::protocol::Event| {
            if !ENABLED.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(message) = event.message.take() {
                event.message = Some(scrub::scrub_text(&message));
            }
            for exception in event.exception.iter_mut() {
                if let Some(value) = exception.value.take() {
                    exception.value = Some(scrub::scrub_text(&value));
                }
            }
            for entry in event.logentry.iter_mut() {
                entry.message = scrub::scrub_text(&entry.message);
            }
            Some(event)
        })
        .before_breadcrumb(|mut crumb: sentry::protocol::Breadcrumb| {
            if !ENABLED.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(message) = crumb.message.take() {
                crumb.message = Some(scrub::scrub_text(&message));
            }
            Some(crumb)
        });
    Some(sentry::init((DSN, options)))
}
