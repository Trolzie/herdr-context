//! Remember which pane hosts the sidebar in each tab, so the toggle action
//! can close it again.

use std::path::PathBuf;

fn dir() -> Option<PathBuf> {
    let base = std::env::var_os("HERDR_PLUGIN_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state/herdr-context"))
        })?;
    Some(base.join("sidebars"))
}

fn file(tab: &str) -> Option<PathBuf> {
    Some(dir()?.join(tab.replace([':', '/'], "_")))
}

pub fn remember(tab: &str, pane: &str) {
    if let Some(path) = file(tab) {
        let _ = path.parent().map(std::fs::create_dir_all);
        let _ = std::fs::write(path, pane);
    }
}

pub fn forget(tab: &str) {
    if let Some(path) = file(tab) {
        let _ = std::fs::remove_file(path);
    }
}

pub fn lookup(tab: &str) -> Option<String> {
    let pane = std::fs::read_to_string(file(tab)?).ok()?;
    Some(pane.trim().to_owned()).filter(|p| !p.is_empty())
}
