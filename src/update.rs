//! Daily check for a newer release, shown as a hint in the sidebar footer.
//! It never updates anything itself.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const RELEASES: &str = "https://api.github.com/repos/Trolzie/herdr-wherewasi/releases/latest";
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// The newer version available, if any. Uses a cached answer for a day;
/// otherwise asks GitHub (call this off the UI thread).
pub fn newer_version() -> Option<String> {
    if disabled() {
        return None;
    }
    let cache = cache_file()?;
    let fresh = std::fs::metadata(&cache)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|m| m.elapsed().ok())
        .is_some_and(|age| age < CHECK_EVERY);
    let latest = if fresh {
        std::fs::read_to_string(&cache).ok()?
    } else {
        let latest = fetch_latest().unwrap_or_default();
        let _ = cache.parent().map(std::fs::create_dir_all);
        let _ = std::fs::write(&cache, &latest);
        latest
    };
    let latest = latest.trim().trim_start_matches('v').to_owned();
    (parse(&latest)? > parse(env!("CARGO_PKG_VERSION"))?).then_some(latest)
}

fn fetch_latest() -> Option<String> {
    let output = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "5",
            "-H",
            "Accept: application/vnd.github+json",
            RELEASES,
        ])
        .output()
        .ok()?;
    let release: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    release["tag_name"].as_str().map(str::to_owned)
}

/// `update_check = false` in the plugin config turns the check off.
fn disabled() -> bool {
    crate::resolve::plugin_config()
        .and_then(|text| crate::resolve::config_value(&text, "update_check"))
        .is_some_and(|value| value == "false")
}

fn cache_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".local/state/wherewasi/latest-release"))
}

fn parse(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.').map(|p| p.parse::<u64>().ok());
    Some((
        parts.next()??,
        parts.next()??,
        parts.next().flatten().unwrap_or(0),
    ))
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn compares_versions_numerically() {
        assert!(parse("0.10.0") > parse("0.9.3"));
        assert_eq!(parse("1.2"), Some((1, 2, 0)));
        assert_eq!(parse("main"), None);
    }
}
