//! Minimal client for the herdr socket API: one-shot requests and a
//! long-lived focus subscription.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

pub fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("HERDR_SOCKET_PATH") {
        return PathBuf::from(path);
    }
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/herdr/herdr.sock")
}

/// Send one request and return its `result` object.
pub fn request(method: &str, params: Value) -> Result<Value> {
    let mut stream = UnixStream::connect(socket_path()).context("connect to herdr socket")?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let body = json!({ "id": "wherewasi", "method": method, "params": params });
    writeln!(stream, "{body}")?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    let response: Value = serde_json::from_str(&line).context("parse herdr response")?;
    if let Some(error) = response.get("error") {
        bail!(
            "{method}: {}",
            error["message"].as_str().unwrap_or("unknown error")
        );
    }
    Ok(response["result"].clone())
}

#[derive(Clone, Debug, Default)]
pub struct PaneInfo {
    pub pane_id: String,
    pub tab_id: String,
    pub cwd: Option<PathBuf>,
    pub agent: Option<String>,
    pub agent_status: Option<String>,
}

impl PaneInfo {
    fn from_value(pane: &Value) -> Self {
        let text = |key: &str| pane[key].as_str().map(str::to_owned);
        Self {
            pane_id: text("pane_id").unwrap_or_default(),
            tab_id: text("tab_id").unwrap_or_default(),
            cwd: text("foreground_cwd")
                .or_else(|| text("cwd"))
                .map(PathBuf::from),
            agent: text("agent"),
            agent_status: text("agent_status"),
        }
    }
}

pub fn pane_get(pane_id: &str) -> Result<PaneInfo> {
    let result = request("pane.get", json!({ "pane_id": pane_id }))?;
    Ok(PaneInfo::from_value(&result["pane"]))
}

/// The pane that currently has focus in the server.
pub fn pane_current() -> Result<PaneInfo> {
    let result = request("pane.current", json!({}))?;
    Ok(PaneInfo::from_value(&result["pane"]))
}

pub enum FocusEvent {
    /// Some pane gained focus; `None` means "re-read the focused pane".
    Focused(Option<String>),
    /// Something about a pane changed (title, agent status, cwd).
    PaneUpdated(String),
}

/// Spawn a thread that streams focus changes, reconnecting when the server
/// restarts, hands off, or drops the subscription.
pub fn subscribe_focus(tx: Sender<FocusEvent>) {
    thread::spawn(move || {
        loop {
            if stream_focus(&tx).is_err() && tx.send(FocusEvent::Focused(None)).is_err() {
                return;
            }
            thread::sleep(Duration::from_secs(1));
        }
    });
}

fn stream_focus(tx: &Sender<FocusEvent>) -> Result<()> {
    let mut stream = UnixStream::connect(socket_path())?;
    let subscriptions: Vec<Value> = [
        "pane.focused",
        "tab.focused",
        "workspace.focused",
        "pane.updated",
    ]
    .iter()
    .map(|kind| json!({ "type": kind }))
    .collect();
    let body = json!({
        "id": "wherewasi-sub",
        "method": "events.subscribe",
        "params": { "subscriptions": subscriptions },
    });
    writeln!(stream, "{body}")?;

    for line in BufReader::new(stream).lines() {
        let event: Value = serde_json::from_str(&line?)?;
        if event.get("error").is_some() {
            bail!("subscription closed");
        }
        // Events arrive either flat or wrapped in `data`.
        let data = event.get("data").unwrap_or(&event);
        let kind = data["type"]
            .as_str()
            .or(event["event"].as_str())
            .unwrap_or("");
        let pane_id = data["pane_id"]
            .as_str()
            .or(data["pane"]["pane_id"].as_str())
            .map(str::to_owned);
        let message = match kind {
            "pane_focused" | "pane.focused" => FocusEvent::Focused(pane_id),
            "tab_focused" | "tab.focused" | "workspace_focused" | "workspace.focused" => {
                FocusEvent::Focused(None)
            }
            "pane_updated" | "pane.updated" => match pane_id {
                Some(id) => FocusEvent::PaneUpdated(id),
                None => continue,
            },
            _ => continue,
        };
        if tx.send(message).is_err() {
            return Ok(());
        }
    }
    bail!("subscription ended")
}
