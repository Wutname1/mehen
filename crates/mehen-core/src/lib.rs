//! Mehen's engine: finds projects under a folder, reads their dependencies,
//! and checks them for updates and known vulnerabilities. Front ends (the
//! desktop app today, a VS Code extension later) only call `scan` and `check`.

pub mod batch;
pub mod check;
pub mod compat;
pub mod diagnose;
pub mod icons;
pub mod ignore;
pub mod lockfiles;
pub mod model;
pub mod osv;
pub mod registry;
pub mod scan;
pub mod store;
pub mod update;
pub mod version;

pub use check::{CheckOptions, check};
pub use model::*;
pub use ignore::{IgnoreKind, IgnoreRule, IgnoreSet};
pub use scan::{discover, scan};
pub use store::Store;
