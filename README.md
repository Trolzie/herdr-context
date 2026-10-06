# wherewasi

*Where was I?* A [Herdr](https://herdr.dev) plugin that docks a live, rendered Markdown file on the right of your tab: the plan, what was just done, and what's next. Jump between tabs, repos and worktrees and the answer is already on screen.

```text
 prime · fix/login-redirect                ● working
─────────────────────────────────────────────────────
 ▌NOW
   Fix redirect loop after SSO callback

 ▾ Goal
   Users land on /dashboard after login.

 ▾ Tasks                               3/5 ━━━──
   ✔ reproduce locally
   ✔ find where next is set
 › ✔ sanitize next param
   ☐ add regression test
   ☐ ask pi for review

 ▸ Decisions (3)
─────────────────────────────────────────────────────
 .herdr/wherewasi.md · 2m ago           e edit  ? keys
```

## Features

- Renders headings, lists, task checkboxes, code blocks, quotes, links and tables in the Kanagawa palette.
- Follows focus: click a pane in another repo or worktree and the sidebar switches to that repo's file.
- Pins a `## Now` section to the top.
- Shows task progress per section and lets you fold sections.
- `space` ticks a checkbox and writes `[x]` back to the file.
- Reloads live when the file changes, including edits made by agents.
- The header shows repo, branch and the focused pane's agent status, plus the active herdr-ledger run when that tool is installed.

## Agents keep it current

You don't have to write the file yourself. The plugin briefs your coding agents and has them log their work as they go, so the sidebar fills itself in:

- **Session start:** the agent gets the current file and a short protocol (see [`agents/protocol.md`](agents/protocol.md)), so it knows the plan before it starts.
- **While working:** the agent records progress with one-line commands:

  ```bash
  wherewasi now "Fixing the login redirect loop"
  wherewasi did "Allow-listed redirect paths"
  wherewasi todo "Add a regression test"
  wherewasi check "regression test"
  wherewasi decide "Allow-list paths instead of stripping next"
  ```

- **Its own plan, mirrored:** when the agent plans with its built-in tools, that plan appears in `## Plan` automatically. This covers Claude Code's approved plans and task list, and Codex's `update_plan`. The step in progress becomes `## Now`, and finished steps are logged. Agents without a checklist tool (newer Claude models, pi) are asked to write their steps with `wherewasi todo` instead.
- **Commits, logged for free:** commits made during a turn are logged by their subject line.
- **End of a turn:** if the agent changed the repo but nothing was logged, it is asked once to add a line before it finishes.

The hooks only act inside Herdr panes, and the file stays yours to edit: press `e`, or edit it however you like. Agents are told to build on your wording.

Supported agents: Claude Code (hooks + skill), Codex (hooks) and pi (extension).

## Install

Requires Herdr 0.9.3 or newer and a Rust toolchain (`cargo`).

```bash
herdr plugin install Trolzie/herdr-wherewasi
```

Bind the toggle to a key in `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+m"
type = "plugin_action"
command = "trolz.wherewasi.toggle"
description = "toggle wherewasi sidebar"
```

Then run `herdr server reload-config`. Without a binding, use `herdr plugin action invoke trolz.wherewasi.toggle`.

### What install sets up

Installing runs `wherewasi setup`, and Herdr runs it again on every start so paths stay current after updates. It only touches agents whose config folder exists, and only adds its own entries:

| Where | What |
| --- | --- |
| `~/.local/bin/wherewasi` | symlink to the plugin binary, so agents can run `wherewasi` |
| `~/.claude/settings.json` | `SessionStart`, `UserPromptSubmit`, `Stop`, `PostToolUse` (plan mode), `TaskCreated` and `TaskCompleted` hooks |
| `~/.claude/skills/wherewasi/` | a skill describing the commands |
| `~/.codex/hooks.json` | `SessionStart`, `UserPromptSubmit`, `Stop` and `PostToolUse` (`update_plan`) hooks |
| `~/.pi/agent/extensions/wherewasi.ts` | a pi extension doing the same |

Codex asks you to approve new hooks once: open Codex, run `/hooks` and trust the wherewasi hooks. Until then Codex skips them. The hook commands point at `~/.local/bin/wherewasi`, so they stay approved across plugin updates.

To turn this off, add `auto_setup = false` to the plugin's `config.toml` (see below) and run `wherewasi setup --remove`, which removes exactly what it added.

## Updating

Once a day the sidebar checks GitHub for a newer release and, if there is one, shows `↑ vX.Y.Z` in its footer. It never updates itself. To update:

```bash
herdr plugin install Trolzie/herdr-wherewasi --yes
```

The agent hooks are refreshed automatically the next time Herdr starts.

## Which file is shown

For the focused pane's directory, the first file that exists wins:

1. `<repo>/.herdr/wherewasi.md` (private by default)
2. `<repo>/WHEREWASI.md` (commit this one to share it with your team)
3. `<notes_dir>/<repo>.md`, if you configure a notes folder (see below)

The repo name comes from the `origin` remote, so Herdr's generated worktree names do not change it.

To keep notes outside your repos, set a notes folder in the plugin's config file. `herdr plugin config-dir trolz.wherewasi` prints its folder; create `config.toml` there:

```toml
notes_dir = "~/notes"
# auto_setup = false     # don't install agent hooks
# update_check = false   # don't check GitHub for new releases
```

The `WHEREWASI_NOTES_DIR` environment variable overrides it.

If no file exists, press `n` to create `.herdr/wherewasi.md` from a template. The sidebar adds `.herdr/` to the repo's `.git/info/exclude` so the file stays private. Remove that line if you want to commit the file and share it with your team.

## Keys

| Key | Action |
| --- | --- |
| `j` / `k`, arrows | move |
| `ctrl+d` / `ctrl+u` | half page |
| `g` / `G` | top / bottom |
| `space` / `enter` | toggle checkbox |
| `z` / `tab` | fold section |
| `Z` | fold / unfold all |
| `e` | edit in `$EDITOR` (Herdr popup) |
| `n` | create wherewasi file |
| `r` | reload |
| `?` | help |
| `q` | close sidebar |

## Tips

- `wherewasi render FILE [WIDTH]` prints the rendered file as plain text, which is handy for checking layout.

## Development

```bash
cargo build --release
herdr plugin link "$PWD"
cargo fmt --check && cargo test && cargo clippy --all-targets -- -D warnings
```

## License

MIT
