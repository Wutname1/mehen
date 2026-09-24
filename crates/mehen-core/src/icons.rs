//! Project logos: finds a favicon, app icon, or logo image inside a project
//! folder so the project list can show it. Discovery walks the folder, so its
//! answer (including "nothing found") is remembered in the store and the walk
//! runs once per project, not once per launch.

use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_DEPTH: usize = 5;

fn mime(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_string_lossy().to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "ico" => Some("image/x-icon"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

fn skipped(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git" | ".next" | ".nuxt" | ".svelte-kit" | "build" | "coverage" | "dist" | "node_modules" | "out" | "target" | "vendor" | "bin" | "obj"
    )
}

/// Lower is better: favicons first, then app icons, logos, and icons, with
/// shallow files and public/assets folders ahead of deeper ones.
fn score(root: &Path, path: &Path) -> Option<u32> {
    mime(path)?;
    let stem = path.file_stem()?.to_string_lossy().to_ascii_lowercase();
    let name = if stem == "favicon" {
        0
    } else if stem.contains("favicon") {
        10
    } else if matches!(stem.as_str(), "app-icon" | "app_icon" | "appicon") {
        20
    } else if stem == "logo" {
        30
    } else if stem.ends_with("-logo") || stem.ends_with("_logo") {
        38
    } else if stem == "icon" {
        45
    } else if stem.ends_with("-icon") || stem.ends_with("_icon") {
        52
    } else {
        return None;
    };
    let relative = path.strip_prefix(root).unwrap_or(path);
    let depth = relative.components().count().saturating_sub(1) as u32;
    let parent = relative.parent().map(|p| p.to_string_lossy().replace('\\', "/").to_ascii_lowercase()).unwrap_or_default();
    let folder = if parent.is_empty() {
        0
    } else if matches!(parent.as_str(), "public" | "static" | "assets" | "icons" | "images") {
        2
    } else if parent.ends_with("/public") || parent.ends_with("/static") || parent.ends_with("/assets") {
        5
    } else {
        15
    };
    Some(name + folder + depth * 3)
}

/// The best logo-like image in `root`, if any.
pub fn find_icon(root: &Path) -> Option<PathBuf> {
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let mut best: Option<(u32, PathBuf)> = None;
    while let Some((dir, depth)) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                if depth < MAX_DEPTH && !skipped(&entry.file_name().to_string_lossy()) {
                    pending.push((path, depth + 1));
                }
                continue;
            }
            if !kind.is_file() || entry.metadata().map_or(true, |m| m.len() > MAX_BYTES) {
                continue;
            }
            if let Some(s) = score(root, &path) {
                let better = best.as_ref().is_none_or(|(b, bp)| s < *b || (s == *b && path.to_string_lossy() < bp.to_string_lossy()));
                if better {
                    best = Some((s, path));
                }
            }
        }
    }
    best.map(|(_, path)| path)
}

/// The image as a `data:` URL the webview can show without file access.
pub fn data_url(path: &Path) -> Option<String> {
    let mime = mime(path)?;
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favicons_beat_logos_and_dependency_folders_are_skipped() {
        let root = std::env::temp_dir().join("mehen-icons");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("public")).unwrap();
        fs::create_dir_all(root.join("node_modules").join("pkg")).unwrap();
        fs::write(root.join("logo.png"), b"x").unwrap();
        fs::write(root.join("public").join("favicon.svg"), b"<svg/>").unwrap();
        fs::write(root.join("node_modules").join("pkg").join("favicon.ico"), b"x").unwrap();
        assert_eq!(find_icon(&root), Some(root.join("public").join("favicon.svg")));
        assert!(data_url(&root.join("public").join("favicon.svg")).unwrap().starts_with("data:image/svg+xml;base64,"));
        assert!(score(&root, &root.join("hero.png")).is_none());
    }
}
