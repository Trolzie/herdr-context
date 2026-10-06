# herdr-context

A [Herdr](https://herdr.dev) plugin that docks a live, rendered Markdown file on the right of your tab, so the context of what you are working on stays in view next to your agents.

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
 .herdr/context.md · 2m ago             e edit  ? keys
```

## Features

- Renders headings, lists, task checkboxes, code blocks, quotes, links and tables in the Kanagawa palette.
- Follows focus: click a pane in another repo or worktree and the sidebar switches to that repo's file.
- Pins a `## Now` section to the top.
- Shows task progress per section and lets you fold sections.
- `space` ticks a checkbox and writes `[x]` back to the file.
- Reloads live when the file changes, including edits made by agents.
- The header shows repo, branch and the focused pane's agent status, plus the active herdr-ledger run when that tool is installed.

## Install

Requires Herdr 0.9.3 or newer and a Rust toolchain (`cargo`).

```bash
herdr plugin install Trolzie/herdr-context
```

Bind the toggle to a key in `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+m"
type = "plugin_action"
command = "trolz.context.toggle"
description = "toggle context sidebar"
```

Then run `herdr server reload-config`. Without a binding, use `herdr plugin action invoke trolz.context.toggle`.

## Which file is shown

For the focused pane's directory, the first file that exists wins:

1. `<repo>/.herdr/context.md`
2. `<repo>/CONTEXT.md`
3. `<notes_dir>/<repo>.md`, if you configure a notes folder (see below)

The repo name comes from the `origin` remote, so Herdr's generated worktree names do not change it.

To keep notes outside your repos, set a notes folder in the plugin's config file. `herdr plugin config-dir trolz.context` prints its folder; create `config.toml` there:

```toml
notes_dir = "~/notes"
```

The `HERDR_CONTEXT_NOTES_DIR` environment variable overrides it.

If no file exists, press `n` to create `.herdr/context.md` from a template. The sidebar adds `.herdr/` to the repo's `.git/info/exclude` so the file stays private. Remove that line if you want to commit the file and share it with your team.

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
| `n` | create context file |
| `r` | reload |
| `?` | help |
| `q` | close sidebar |

## Tips

- Ask your agents to keep the file current, for example in `AGENTS.md`: "Record decisions and task progress in `.herdr/context.md`."
- `herdr-context render FILE [WIDTH]` prints the rendered file as plain text, which is handy for checking layout.

## Development

```bash
cargo build --release
herdr plugin link "$PWD"
cargo fmt --check && cargo test && cargo clippy --all-targets -- -D warnings
```

## License

MIT
