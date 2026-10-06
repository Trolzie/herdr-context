//! Map a working directory to the context file that belongs to it.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// Git worktree root, or the plain directory when outside Git.
    pub root: PathBuf,
    /// Repository name, stable across Herdr's generated worktree names.
    pub repo: String,
    pub branch: Option<String>,
    /// First candidate that exists, if any.
    pub file: Option<PathBuf>,
    /// Where `n` creates a file when none exists.
    pub default_file: PathBuf,
}

impl Target {
    pub fn resolve(cwd: &Path) -> Self {
        let root = git(cwd, &["rev-parse", "--show-toplevel"])
            .map(PathBuf::from)
            .unwrap_or_else(|| cwd.to_path_buf());
        let repo = repo_name(&root);
        let branch = git(&root, &["branch", "--show-current"]).filter(|b| !b.is_empty());
        let default_file = root.join(".herdr").join("context.md");
        let mut target = Self {
            root,
            repo,
            branch,
            file: None,
            default_file,
        };
        target.file = target.find_file();
        target
    }

    /// Re-check candidates so files created outside the sidebar are picked up.
    pub fn find_file(&self) -> Option<PathBuf> {
        let mut candidates = vec![self.default_file.clone(), self.root.join("CONTEXT.md")];
        if let Some(dir) = notes_dir() {
            candidates.push(dir.join(format!("{}.md", self.repo)));
        }
        candidates.into_iter().find(|path| path.is_file())
    }
}

/// Optional folder of per-repo notes: `WHEREWASI_NOTES_DIR`, or
/// `notes_dir = "..."` in the plugin's `config.toml`.
fn notes_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("WHEREWASI_NOTES_DIR") {
        return Some(PathBuf::from(dir));
    }
    let config = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR")?;
    let text = std::fs::read_to_string(PathBuf::from(config).join("config.toml")).ok()?;
    let value = config_value(&text, "notes_dir")?;
    Some(expand_home(&value))
}

/// Read a `key = "value"` line; enough for this plugin's single setting.
fn config_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name.trim() == key).then(|| value.trim().trim_matches('"').to_owned())
    })
}

fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Prefer the `origin` remote, then the main worktree's directory, so names
/// like `worktree-brave-river-8940` do not leak into the header.
fn repo_name(root: &Path) -> String {
    if let Some(url) = git(root, &["remote", "get-url", "origin"]) {
        let name = url.trim_end_matches('/').trim_end_matches(".git");
        if let Some(last) = name.rsplit(['/', ':']).next().filter(|s| !s.is_empty()) {
            return last.to_owned();
        }
    }
    if let Some(common) = git(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ) {
        let common = PathBuf::from(common);
        if let Some(name) = common.parent().and_then(Path::file_name) {
            return name.to_string_lossy().into_owned();
        }
    }
    root.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "~".to_owned())
}

/// Starter file; kept markdownlint-clean so editors don't flag it.
pub fn template(repo: &str) -> String {
    format!(
        "# {repo}\n\n## Now\n\nWhat you are doing right now.\n\n## Goal\n\n## Tasks\n\n- [ ] first task\n\n## Decisions\n\n## Notes\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_notes_dir_setting() {
        let text = "# comment\nnotes_dir = \"~/notes\"\n";
        assert_eq!(config_value(text, "notes_dir").as_deref(), Some("~/notes"));
        assert_eq!(config_value(text, "other"), None);
    }
}
