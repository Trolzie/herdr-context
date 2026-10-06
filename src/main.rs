mod app;
mod edit;
mod herdr;
mod hook;
mod layout;
mod markdown;
mod resolve;
mod setup;
mod state;
mod theme;
mod update;

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

const PLUGIN_ID: &str = "trolz.wherewasi";
const USAGE: &str = "\
wherewasi: keep a live \"where was I?\" file for the repo you are in

Agent commands (run inside the repo):
  wherewasi now TEXT       set what is happening right now
  wherewasi did TEXT       log something just finished (newest first)
  wherewasi todo TEXT      add an open task
  wherewasi check TEXT     tick the first open task containing TEXT
  wherewasi decide TEXT    record a decision
  wherewasi note TEXT      add a note
  wherewasi show           print the file
  wherewasi path           print the file's path

Setup:
  wherewasi setup [--remove] install or remove the agent hooks (runs automatically)

Plugin commands:
  wherewasi view | toggle | edit | render FILE [WIDTH] | hook AGENT EVENT";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("view") => view(),
        Some(command @ ("now" | "did" | "todo" | "check" | "decide" | "note")) => {
            update(command, &args[1..].join(" "))
        }
        Some("show") => show(),
        Some("setup") => setup::run(&args[1..]),
        Some("hook") => match args.get(1..4) {
            // pi: `hook pi EVENT SESSION CWD`
            Some([agent, event, session]) if agent == "pi" => {
                let cwd = args
                    .get(4)
                    .map_or_else(|| PathBuf::from("."), PathBuf::from);
                hook::run_plain(event, session, &cwd);
                Ok(())
            }
            _ => match args.get(1..3) {
                Some([agent, event]) => hook::run(agent, event),
                _ => bail!("usage: wherewasi hook AGENT EVENT"),
            },
        },
        Some("path") => {
            println!("{}", here().file_or_default().display());
            Ok(())
        }
        Some("toggle") => toggle(),
        Some("edit") => edit(),
        Some("render") => render(&args[1..]),
        None | Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => bail!("unknown command `{other}`\n{USAGE}"),
    }
}

/// The target for the directory the command runs in.
fn here() -> resolve::Target {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    resolve::Target::resolve(&cwd)
}

/// Agent command: apply one section edit, creating the file if needed.
fn update(command: &str, text: &str) -> Result<()> {
    let text = text.trim();
    if text.is_empty() {
        bail!("usage: wherewasi {command} TEXT");
    }
    let mut target = here();
    let path = target.file_or_create().context("create wherewasi file")?;
    let mut file = edit::File::parse(&std::fs::read_to_string(&path)?);
    match command {
        "now" => file.now(text),
        "did" => file.did(text),
        "todo" => file.todo(text),
        "decide" => file.decide(text),
        "note" => file.note(text),
        "check" => match file.check(text) {
            Ok(task) => println!("checked: {task}"),
            Err(open) if open.is_empty() => bail!("no open tasks"),
            Err(open) => bail!(
                "no open task matches `{text}`; open tasks:\n- {}",
                open.join("\n- ")
            ),
        },
        _ => unreachable!(),
    }
    resolve::write_atomically(&path, &file.render())?;
    Ok(())
}

fn show() -> Result<()> {
    let target = here();
    match &target.file {
        Some(path) => print!("{}", std::fs::read_to_string(path)?),
        None => println!(
            "No wherewasi file for {} yet. `wherewasi now TEXT` creates {}.",
            target.repo,
            target.default_file.display()
        ),
    }
    Ok(())
}

fn view() -> Result<()> {
    let mut terminal = ratatui::init();
    let result = app::App::new().run(&mut terminal);
    ratatui::restore();
    result
}

/// Plugin action: close the sidebar in the current tab, or open it.
fn toggle() -> Result<()> {
    let focused = match std::env::var("HERDR_PANE_ID") {
        Ok(id) if !id.is_empty() => herdr::pane_get(&id)?,
        _ => herdr::pane_current()?,
    };
    let tab = focused.tab_id.clone();

    if let Some(pane) = state::lookup(&tab) {
        state::forget(&tab);
        if herdr::pane_get(&pane).is_ok_and(|p| p.tab_id == tab) {
            herdr::request("plugin.pane.close", json!({ "pane_id": pane }))?;
            return Ok(());
        }
    }

    let plugin = std::env::var("HERDR_PLUGIN_ID").unwrap_or_else(|_| PLUGIN_ID.to_owned());
    let target = rightmost_pane(&focused.pane_id).unwrap_or(focused.pane_id.clone());
    let opened = herdr::request(
        "plugin.pane.open",
        json!({
            "plugin_id": plugin,
            "entrypoint": "sidebar",
            "placement": "split",
            "direction": "right",
            "target_pane_id": target,
            "focus": false,
            "cwd": focused.cwd,
        }),
    )?;
    // Splits open at 50%; narrow the sidebar to a sidebar-like width.
    if let Some(sidebar) = find_pane_id(&opened) {
        let _ = size_sidebar(&tab, &sidebar, &target);
    }
    Ok(())
}

fn find_pane_id(value: &Value) -> Option<String> {
    if let Some(id) = value["pane"]["pane_id"].as_str() {
        return Some(id.to_owned());
    }
    value.as_object()?.values().find_map(find_pane_id)
}

/// Target width: 30% of the tab, kept between 36 and 64 columns.
fn size_sidebar(tab: &str, sidebar: &str, sibling: &str) -> Option<()> {
    let layout = herdr::request("pane.layout", json!({ "pane_id": sidebar })).ok()?;
    let layout = &layout["layout"];
    let rect = |id: &str| {
        layout["panes"]
            .as_array()?
            .iter()
            .find(|p| p["pane_id"] == id)
            .map(|p| p["rect"].clone())
    };
    let (ours, theirs) = (rect(sidebar)?, rect(sibling)?);
    let x = |r: &Value, key: &str| r[key].as_f64().unwrap_or(0.0);
    let span = x(&ours, "x") + x(&ours, "width") - x(&theirs, "x");
    let wanted = (x(&layout["area"], "width") * 0.3)
        .clamp(36.0, 64.0)
        .min(span - 20.0);
    if span <= 0.0 || wanted <= 0.0 {
        return None;
    }

    let exported = herdr::request("layout.export", json!({ "tab_id": tab })).ok()?;
    let mut path = path_to(&exported["layout"]["root"], sidebar)?;
    // The sidebar must be the second child of the split that created it.
    if path.pop() != Some(true) {
        return None;
    }
    herdr::request(
        "layout.set_split_ratio",
        json!({ "tab_id": tab, "path": path, "ratio": 1.0 - wanted / span }),
    )
    .ok()?;
    Some(())
}

/// Branch choices (`false` = first, `true` = second) leading to a pane.
fn path_to(node: &Value, pane: &str) -> Option<Vec<bool>> {
    match node["type"].as_str()? {
        "pane" => (node["pane_id"] == pane).then(Vec::new),
        _ => [(false, "first"), (true, "second")]
            .into_iter()
            .find_map(|(branch, key)| {
                let mut path = path_to(&node[key], pane)?;
                path.insert(0, branch);
                Some(path)
            }),
    }
}

/// The tallest pane touching the right edge, so the sidebar spans as much
/// height as a split allows.
fn rightmost_pane(pane_id: &str) -> Option<String> {
    let result = herdr::request("pane.layout", json!({ "pane_id": pane_id })).ok()?;
    let layout = &result["layout"];
    let right_edge = layout["area"]["x"].as_u64()? + layout["area"]["width"].as_u64()?;
    layout["panes"]
        .as_array()?
        .iter()
        .filter(|p| {
            let rect = &p["rect"];
            rect["x"].as_u64().unwrap_or(0) + rect["width"].as_u64().unwrap_or(0) >= right_edge
        })
        .max_by_key(|p| p["rect"]["height"].as_u64().unwrap_or(0))
        .and_then(|p| p["pane_id"].as_str().map(str::to_owned))
}

/// Popup entrypoint: open the wherewasi file in the user's editor.
fn edit() -> Result<()> {
    let file = std::env::var_os("WHEREWASI_FILE").context("WHEREWASI_FILE is not set")?;
    let status = app::editor_command(&PathBuf::from(file)).status()?;
    if !status.success() {
        bail!("editor exited with {status}");
    }
    Ok(())
}

/// Print the rendered file as plain text, for previews and debugging.
fn render(args: &[String]) -> Result<()> {
    let file = args.first().context(USAGE)?;
    let width = args.get(1).and_then(|w| w.parse().ok()).unwrap_or(48);
    let source = std::fs::read_to_string(file)?;
    let doc = markdown::Doc::parse(&source);
    for row in layout::rows(&doc, &HashSet::new(), width) {
        let text: String = row
            .content
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        println!("{}", text.trim_end());
    }
    Ok(())
}
