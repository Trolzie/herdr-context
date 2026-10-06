//! `wherewasi setup`: install (or `--remove`) the agent integrations, the way
//! herdr's own integrations do. Runs automatically when the plugin is built
//! and whenever herdr starts, so installs stay current. Only agents whose
//! config folder exists are touched.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The Claude Code skill is the shared protocol behind skill frontmatter.
const SKILL_FRONTMATTER: &str = "---\nname: wherewasi\ndescription: Keep the repo's \
wherewasi file (the user's live overview in the herdr sidebar) current, recording what you are \
doing now, what you just did, tasks and decisions. Use when starting, finishing or changing \
direction on work in a repo, or when the user asks where things stand.\n---\n\n";
const PI_EXTENSION: &str = include_str!("../agents/pi/wherewasi.ts");

/// A hook entry: agent event, optional tool matcher, `wherewasi hook` event.
type Hook = (&'static str, Option<&'static str>, &'static str);

const CLAUDE_HOOKS: &[Hook] = &[
    ("SessionStart", None, "session-start"),
    ("UserPromptSubmit", None, "prompt"),
    ("Stop", None, "stop"),
    // Mirror the agent's own planning into `## Plan`.
    ("PostToolUse", Some("ExitPlanMode"), "plan"),
    ("TaskCreated", None, "task-created"),
    ("TaskCompleted", None, "task-completed"),
];

const CODEX_HOOKS: &[Hook] = &[
    ("SessionStart", None, "session-start"),
    ("UserPromptSubmit", None, "prompt"),
    ("Stop", None, "stop"),
    ("PostToolUse", Some("^update_plan$"), "plan"),
];

pub fn run(args: &[String]) -> Result<()> {
    let quiet = args.iter().any(|a| a == "--quiet");
    let remove = args.iter().any(|a| a == "--remove");
    if quiet && !remove && opted_out() {
        return Ok(());
    }
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    let exe = std::env::current_exe()?.canonicalize()?;
    let mut report = Vec::new();
    // Problems with one agent's config never stop the others, and never fail
    // the run: this is a herdr build step, and a failure would abort install.
    let mut warnings = Vec::new();

    let (linked, line) = link_binary(&home, &exe, remove);
    report.push(line);
    // Hooks call the stable link, so their commands (and Codex's trust
    // hashes) survive plugin moves and reinstalls.
    let bin = linked.unwrap_or_else(|| exe.clone());

    let claude = home.join(".claude");
    if claude.is_dir() {
        let skill = claude.join("skills/wherewasi/SKILL.md");
        let skill_result = if remove {
            let _ = std::fs::remove_file(&skill);
            let _ = skill.parent().map(std::fs::remove_dir);
            Ok(())
        } else {
            let contents = format!("{SKILL_FRONTMATTER}{}", crate::hook::PROTOCOL);
            write_if_changed(&skill, &contents)
        };
        let hooks_result = merge_hooks(
            &claude.join("settings.json"),
            "claude",
            CLAUDE_HOOKS,
            &bin,
            remove,
        );
        match skill_result.and(hooks_result) {
            Ok(()) => report.push(format!("claude: {}", state(remove, "skill + hooks"))),
            Err(err) => warnings.push(format!("claude: skipped: {err:#}")),
        }
    }

    let codex = home.join(".codex");
    if codex.is_dir() {
        match merge_hooks(
            &codex.join("hooks.json"),
            "codex",
            CODEX_HOOKS,
            &bin,
            remove,
        ) {
            Ok(()) => {
                report.push(format!("codex: {}", state(remove, "hooks")));
                if !remove {
                    report.push(
                        "  approve them once in Codex with /hooks; Codex skips new hooks until then"
                            .to_owned(),
                    );
                }
            }
            Err(err) => warnings.push(format!("codex: skipped: {err:#}")),
        }
    }

    let pi = home.join(".pi/agent");
    if pi.is_dir() {
        let extension = pi.join("extensions/wherewasi.ts");
        let result = if remove {
            let _ = std::fs::remove_file(&extension);
            Ok(())
        } else {
            let source = PI_EXTENSION
                .replace("__WHEREWASI_BIN__", &bin.to_string_lossy())
                .replace("__VERSION__", VERSION);
            write_if_changed(&extension, &source)
        };
        match result {
            Ok(()) => report.push(format!("pi: {}", state(remove, "extension"))),
            Err(err) => warnings.push(format!("pi: skipped: {err:#}")),
        }
    }

    if !quiet {
        for line in report {
            println!("{line}");
        }
        if !remove {
            println!("Agents pick this up in their next session.");
        }
    }
    for warning in warnings {
        eprintln!("wherewasi setup: {warning}");
    }
    Ok(())
}

fn state(remove: bool, what: &str) -> String {
    if remove {
        format!("{what} removed")
    } else {
        format!("{what} installed")
    }
}

/// `auto_setup = false` in the plugin config turns off automatic installs.
fn opted_out() -> bool {
    crate::resolve::plugin_config()
        .and_then(|text| crate::resolve::config_value(&text, "auto_setup"))
        .is_some_and(|value| value == "false")
}

/// Put `wherewasi` on PATH via `~/.local/bin`, without clobbering real files.
/// Returns the link when it points at this binary.
fn link_binary(home: &Path, exe: &Path, remove: bool) -> (Option<PathBuf>, String) {
    let link = home.join(".local/bin/wherewasi");
    let ours = std::fs::symlink_metadata(&link)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if remove {
        if ours {
            let _ = std::fs::remove_file(&link);
        }
        return (
            None,
            "wherewasi command removed from ~/.local/bin".to_owned(),
        );
    }
    if link.exists() && !ours {
        let note = format!("skipped {}: a real file is already there", link.display());
        return (None, note);
    }
    if !std::fs::read_link(&link).is_ok_and(|target| target == exe) {
        let _ = std::fs::remove_file(&link);
        let _ = link.parent().map(std::fs::create_dir_all);
        if let Err(err) = std::os::unix::fs::symlink(exe, &link) {
            return (None, format!("could not link {}: {err}", link.display()));
        }
    }
    let note = format!("wherewasi command: {}", link.display());
    (Some(link), note)
}

/// Add (or remove) our entries in a Claude Code / Codex style hooks file,
/// leaving every other hook untouched. Existing entries are updated in
/// place so their position (which Codex's trust keys use) stays the same.
fn merge_hooks(path: &Path, agent: &str, wanted: &[Hook], exe: &Path, remove: bool) -> Result<()> {
    let original = std::fs::read_to_string(path).unwrap_or_default();
    let before: Value = if original.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&original)
            .with_context(|| format!("{} is not valid JSON", path.display()))?
    };
    let mut settings = before.clone();
    let root = settings
        .as_object_mut()
        .with_context(|| format!("{} is not a JSON object", path.display()))?;
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks
        .as_object_mut()
        .with_context(|| format!("\"hooks\" in {} is not an object", path.display()))?;

    let command_for = |action: &str| {
        let exe = shell_quote(&exe.to_string_lossy());
        format!("[ -x {exe} ] && {exe} hook {agent} {action} || true")
    };
    let mut placed = vec![false; wanted.len()];

    for (event, entries) in hooks.iter_mut() {
        let Some(entries) = entries.as_array_mut() else {
            continue;
        };
        let mut emptied = Vec::new();
        for (index, entry) in entries.iter_mut().enumerate() {
            // Leave anything that isn't a well-formed entry alone.
            let Some(entry) = entry.as_object_mut() else {
                continue;
            };
            let matcher = entry
                .get("matcher")
                .and_then(Value::as_str)
                .filter(|m| !m.is_empty() && *m != "*")
                .map(str::to_owned);
            let Some(inner) = entry.get_mut("hooks").and_then(Value::as_array_mut) else {
                continue;
            };
            let count = inner.len();
            inner.retain_mut(|hook| {
                let Some(action) = our_action(hook) else {
                    return true;
                };
                let slot = wanted.iter().position(|(e, m, a)| {
                    e == event && m.map(str::to_owned) == matcher && *a == action
                });
                match slot {
                    Some(slot) if !remove && !placed[slot] => {
                        placed[slot] = true;
                        hook["command"] = json!(command_for(&action));
                        true
                    }
                    _ => false,
                }
            });
            if inner.is_empty() && count > 0 {
                emptied.push(index);
            }
        }
        for index in emptied.into_iter().rev() {
            entries.remove(index);
        }
    }

    if !remove {
        for (slot, (event, matcher, action)) in wanted.iter().enumerate() {
            if placed[slot] {
                continue;
            }
            let mut entry = json!({
                "hooks": [{ "type": "command", "command": command_for(action), "timeout": 10 }]
            });
            if let Some(matcher) = matcher {
                entry["matcher"] = json!(matcher);
            }
            let Some(entries) = hooks
                .entry(*event)
                .or_insert_with(|| json!([]))
                .as_array_mut()
            else {
                anyhow::bail!("\"{event}\" in {} is not a list", path.display());
            };
            entries.push(entry);
        }
    }
    // Drop event lists we emptied; keep any the user had left empty.
    let before_hooks = before["hooks"].as_object();
    hooks.retain(|event, value| {
        let user_empty = before_hooks
            .and_then(|h| h.get(event))
            .is_some_and(|v| v.as_array().is_some_and(Vec::is_empty));
        !value.as_array().is_some_and(Vec::is_empty) || user_empty
    });
    if hooks.is_empty() && before.get("hooks").is_none() {
        root.remove("hooks");
    }

    if settings == before {
        return Ok(());
    }
    Ok(crate::resolve::write_atomically(
        path,
        &to_json(&settings, &original)?,
    )?)
}

/// Pretty-print with the file's existing indentation (two spaces if new).
fn to_json(value: &Value, original: &str) -> Result<String> {
    let indent: String = original
        .lines()
        .nth(1)
        .map(|line| line.chars().take_while(|c| c.is_whitespace()).collect())
        .filter(|indent: &String| !indent.is_empty())
        .unwrap_or_else(|| "  ".to_owned());
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    serde::Serialize::serialize(value, &mut serializer)?;
    let mut text = String::from_utf8(out)?;
    text.push('\n');
    Ok(text)
}

/// The `wherewasi hook <agent> <action>` action of one of our commands.
fn our_action(hook: &Value) -> Option<String> {
    let command = hook["command"].as_str()?;
    let (_, rest) = command
        .split_once("/wherewasi' hook ")
        .or_else(|| command.split_once("/wherewasi hook "))?;
    let mut words = rest.split_whitespace();
    let _agent = words.next()?;
    words.next().map(str::to_owned)
}

/// Single-quote for `sh`, escaping embedded quotes.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn write_if_changed(path: &Path, contents: &str) -> Result<()> {
    if std::fs::read_to_string(path).is_ok_and(|c| c == contents) {
        return Ok(());
    }
    Ok(crate::resolve::write_atomically(path, contents)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_other_hooks_and_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("wherewasi-setup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"model":"x","hooks":{"Stop":[{"hooks":[{"type":"command","command":"other"}]}]}}"#,
        )
        .unwrap();
        let exe = Path::new("/opt/wherewasi");
        merge_hooks(&path, "claude", CLAUDE_HOOKS, exe, false).unwrap();
        let once = std::fs::read_to_string(&path).unwrap();
        merge_hooks(&path, "claude", CLAUDE_HOOKS, exe, false).unwrap();
        assert_eq!(once, std::fs::read_to_string(&path).unwrap());

        let value: Value = serde_json::from_str(&once).unwrap();
        assert_eq!(value["model"], "x");
        assert_eq!(value["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert!(once.contains("hook claude session-start"));
        assert_eq!(value["hooks"]["PostToolUse"][0]["matcher"], "ExitPlanMode");

        merge_hooks(&path, "claude", CLAUDE_HOOKS, exe, true).unwrap();
        let removed: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            removed,
            json!({"model":"x","hooks":{"Stop":[{"hooks":[{"type":"command","command":"other"}]}]}})
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wherewasi-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("settings.json")
    }

    #[test]
    fn malformed_settings_are_reported_not_rewritten() {
        let path = scratch("malformed");
        let broken = "{ \"hooks\": { \"Stop\": [ }";
        std::fs::write(&path, broken).unwrap();
        assert!(
            merge_hooks(
                &path,
                "claude",
                CLAUDE_HOOKS,
                Path::new("/x/wherewasi"),
                false
            )
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
    }

    #[test]
    fn our_entries_keep_their_position_and_remove_is_byte_exact() {
        let path = scratch("position");
        let original = "{\n    \"hooks\": {\n        \"Stop\": [\n            {\n                \"hooks\": [\n                    {\n                        \"type\": \"command\",\n                        \"command\": \"other\"\n                    }\n                ]\n            }\n        ]\n    }\n}\n";
        std::fs::write(&path, original).unwrap();
        let exe = Path::new("/opt/wherewasi");
        merge_hooks(&path, "codex", CODEX_HOOKS, exe, false).unwrap();
        // The user adds a hook after ours; a re-run must not move ours.
        let mut value: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        value["hooks"]["Stop"]
            .as_array_mut()
            .unwrap()
            .push(json!({"hooks": [{"type": "command", "command": "later"}]}));
        std::fs::write(&path, to_json(&value, original).unwrap()).unwrap();
        merge_hooks(
            &path,
            "codex",
            CODEX_HOOKS,
            Path::new("/new/place/wherewasi"),
            false,
        )
        .unwrap();
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let stop = value["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop[0]["hooks"][0]["command"], "other");
        assert!(
            stop[1]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("/new/place/wherewasi")
        );
        assert_eq!(stop[2]["hooks"][0]["command"], "later");

        merge_hooks(&path, "codex", CODEX_HOOKS, exe, true).unwrap();
        value_eq_after_removing_later(&path, original);
    }

    fn value_eq_after_removing_later(path: &Path, original: &str) {
        let mut value: Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        value["hooks"]["Stop"].as_array_mut().unwrap().pop();
        assert_eq!(to_json(&value, original).unwrap(), original);
    }

    #[test]
    fn quotes_paths_with_apostrophes() {
        assert_eq!(
            shell_quote("/Users/o'neil/wherewasi"),
            "'/Users/o'\\''neil/wherewasi'"
        );
        let hook =
            json!({"command": "[ -x '/a/wherewasi' ] && '/a/wherewasi' hook claude stop || true"});
        assert_eq!(our_action(&hook).as_deref(), Some("stop"));
        assert_eq!(
            our_action(&json!({"command": "my-wherewasi-wrapper hook x"})),
            None
        );
    }

    #[test]
    fn odd_entries_never_panic_or_get_rewritten() {
        for (name, odd) in [
            ("string", r#"{"hooks":{"SessionStart":["oops"]}}"#),
            ("null", r#"{"hooks":{"Stop":[null]}}"#),
        ] {
            let path = scratch(name);
            std::fs::write(&path, odd).unwrap();
            merge_hooks(
                &path,
                "claude",
                CLAUDE_HOOKS,
                Path::new("/x/wherewasi"),
                false,
            )
            .unwrap();
            let value: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let original: Value = serde_json::from_str(odd).unwrap();
            for (event, entries) in original["hooks"].as_object().unwrap() {
                assert_eq!(value["hooks"][event][0], entries[0], "{name}: kept as-is");
            }
        }
        let path = scratch("not-a-list");
        std::fs::write(&path, r#"{"hooks":{"Stop":"nope"}}"#).unwrap();
        assert!(
            merge_hooks(
                &path,
                "claude",
                CLAUDE_HOOKS,
                Path::new("/x/wherewasi"),
                false
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"hooks":{"Stop":"nope"}}"#
        );
    }

    #[test]
    fn keeps_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let path = scratch("mode");
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        merge_hooks(
            &path,
            "codex",
            CODEX_HOOKS,
            Path::new("/x/wherewasi"),
            false,
        )
        .unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
