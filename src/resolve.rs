//! Map a working directory to the wherewasi file that belongs to it.

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
        let default_file = root.join(".herdr").join("wherewasi.md");
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

    /// Create the default file from the template. Returns whether `.herdr/`
    /// is kept out of Git via `.git/info/exclude`.
    pub fn create(&mut self) -> std::io::Result<bool> {
        let path = self.default_file.clone();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, template(&self.repo))?;
        self.file = Some(path);
        Ok(exclude_herdr_dir(&self.root))
    }

    /// The existing file, or a freshly created one.
    pub fn file_or_create(&mut self) -> std::io::Result<PathBuf> {
        if self.file.is_none() {
            self.create()?;
        }
        Ok(self.file_or_default())
    }

    pub fn file_or_default(&self) -> PathBuf {
        self.file
            .clone()
            .unwrap_or_else(|| self.default_file.clone())
    }

    /// Re-check candidates so files created outside the sidebar are picked up.
    pub fn find_file(&self) -> Option<PathBuf> {
        let mut candidates = vec![self.default_file.clone(), self.root.join("WHEREWASI.md")];
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
    let value = config_value(&plugin_config()?, "notes_dir")?;
    Some(expand_home(&value))
}

/// The plugin's `config.toml`. Herdr passes its folder to plugin commands;
/// agents and hooks run outside the plugin, so fall back to its usual place.
pub fn plugin_config() -> Option<String> {
    let dir = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            let base = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
            Some(base.join("herdr/plugins/config/trolz.wherewasi"))
        })?;
    std::fs::read_to_string(dir.join("config.toml")).ok()
}

/// Read a `key = "value"` line; enough for this plugin's single setting.
pub fn config_value(text: &str, key: &str) -> Option<String> {
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

/// Write via a temp file and rename, so a crash never leaves a truncated
/// file. Renaming over the resolved target keeps symlinks (e.g. into a
/// dotfiles repo) intact, and the original permissions are kept.
pub fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = target.with_extension(format!("wherewasi-{}.tmp", std::process::id()));
    std::fs::write(&temp, contents)?;
    if let Ok(meta) = std::fs::metadata(&target) {
        let _ = std::fs::set_permissions(&temp, meta.permissions());
    }
    std::fs::rename(&temp, &target).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

/// Keep `.herdr/` out of Git without touching the shared `.gitignore`.
fn exclude_herdr_dir(root: &Path) -> bool {
    let Some(path) = git(
        root,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "info/exclude",
        ],
    ) else {
        return false;
    };
    let path = PathBuf::from(path);
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing
        .lines()
        .any(|l| l.trim() == ".herdr/" || l.trim() == ".herdr")
    {
        return true;
    }
    let mut contents = existing;
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str(".herdr/\n");
    path.parent()
        .is_some_and(|dir| std::fs::create_dir_all(dir).is_ok())
        && std::fs::write(&path, contents).is_ok()
}

/// Starter file; kept markdownlint-clean so editors don't flag it.
pub fn template(repo: &str) -> String {
    format!(
        "# {repo}\n\n## Now\n\nNothing in progress yet.\n\n## Goal\n\n## Plan\n\n## Tasks\n\n## Log\n\n## Decisions\n\n## Notes\n"
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
