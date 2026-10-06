//! The sidebar: follows the focused pane, renders its wherewasi file, and
//! handles keys.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use serde_json::{Value, json};
use unicode_width::UnicodeWidthStr;

use crate::herdr::{self, FocusEvent, PaneInfo};
use crate::layout::{self, Row};
use crate::markdown::{self, Doc, Kind};
use crate::resolve::Target;
use crate::{state, theme};

const SWITCH_FLASH: Duration = Duration::from_millis(700);
const MESSAGE_TTL: Duration = Duration::from_secs(3);

pub struct App {
    own_pane: Option<String>,
    own_tab: Option<String>,
    tracked: Option<PaneInfo>,
    target: Option<Target>,
    source: String,
    doc: Doc,
    mtime: Option<SystemTime>,
    folds: HashMap<PathBuf, HashSet<String>>,
    positions: HashMap<PathBuf, (usize, usize)>,
    cursor: usize,
    scroll: usize,
    switched_at: Option<Instant>,
    ledger: Option<String>,
    /// Newer release available, from the daily update check.
    update: Option<String>,
    help: bool,
    message: Option<(String, Instant)>,
    body_height: usize,
    quit: bool,
}

impl App {
    pub fn new() -> Self {
        let own_pane = std::env::var("HERDR_PANE_ID")
            .ok()
            .filter(|id| !id.is_empty());
        Self {
            own_tab: own_pane
                .as_deref()
                .and_then(|id| herdr::pane_get(id).ok())
                .map(|p| p.tab_id),
            own_pane,
            tracked: None,
            target: None,
            source: String::new(),
            doc: Doc::default(),
            mtime: None,
            folds: HashMap::new(),
            positions: HashMap::new(),
            cursor: 0,
            scroll: 0,
            switched_at: None,
            ledger: None,
            update: None,
            help: false,
            message: None,
            body_height: 0,
            quit: false,
        }
    }

    pub fn run(mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        if let (Some(pane), Some(tab)) = (&self.own_pane, &self.own_tab) {
            state::remember(tab, pane);
        }
        let (tx, rx) = channel();
        herdr::subscribe_focus(tx);
        let (update_tx, update_rx) = channel();
        std::thread::spawn(move || {
            let _ = update_tx.send(crate::update::newer_version());
        });

        // Start from the pane that was focused when the sidebar opened, or
        // fall back to the directory the sidebar was launched in.
        self.on_focus(None);
        if self.target.is_none()
            && let Ok(cwd) = std::env::current_dir()
        {
            self.switch_to(&cwd);
        }

        let result = self.event_loop(terminal, &rx, &update_rx);
        if let Some(tab) = &self.own_tab {
            state::forget(tab);
        }
        result
    }

    fn event_loop(
        &mut self,
        terminal: &mut DefaultTerminal,
        rx: &Receiver<FocusEvent>,
        update_rx: &Receiver<Option<String>>,
    ) -> Result<()> {
        let mut last_file_check = Instant::now();
        let mut last_status = Instant::now();
        let mut last_ledger = Instant::now();
        while !self.quit {
            terminal.draw(|frame| self.draw(frame))?;
            if let Ok(update) = update_rx.try_recv() {
                self.update = update;
            }

            if event::poll(Duration::from_millis(200))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        self.on_key(key, terminal)?
                    }
                    _ => {}
                }
            }
            while let Ok(message) = rx.try_recv() {
                match message {
                    FocusEvent::Focused(pane) => self.on_focus(pane),
                    FocusEvent::PaneUpdated(pane) => {
                        if self.tracked.as_ref().is_some_and(|t| t.pane_id == pane) {
                            self.refresh_tracked();
                        }
                    }
                }
            }
            if last_file_check.elapsed() >= Duration::from_millis(750) {
                last_file_check = Instant::now();
                self.check_file();
            }
            if last_status.elapsed() >= Duration::from_secs(2) {
                last_status = Instant::now();
                self.refresh_tracked();
                if let Some(own) = &self.own_pane
                    && let Ok(info) = herdr::pane_get(own)
                {
                    self.own_tab = Some(info.tab_id);
                }
            }
            if last_ledger.elapsed() >= Duration::from_secs(15) {
                last_ledger = Instant::now();
                self.refresh_ledger();
            }
        }
        Ok(())
    }

    // ----- following herdr focus -----

    fn on_focus(&mut self, pane_id: Option<String>) {
        let info = match pane_id {
            Some(id) if Some(&id) == self.own_pane.as_ref() => return,
            Some(id) => herdr::pane_get(&id),
            None => herdr::pane_current(),
        };
        let Ok(info) = info else { return };
        if Some(&info.pane_id) == self.own_pane.as_ref() {
            return;
        }
        // Only follow panes that share the sidebar's tab.
        if let Some(tab) = &self.own_tab
            && &info.tab_id != tab
        {
            return;
        }
        let cwd = info.cwd.clone();
        self.tracked = Some(info);
        if let Some(cwd) = cwd {
            self.switch_to(&cwd);
        }
    }

    fn refresh_tracked(&mut self) {
        let Some(id) = self.tracked.as_ref().map(|t| t.pane_id.clone()) else {
            return;
        };
        let Ok(info) = herdr::pane_get(&id) else {
            return;
        };
        let moved = info.cwd != self.tracked.as_ref().and_then(|t| t.cwd.clone());
        let cwd = info.cwd.clone();
        self.tracked = Some(info);
        if moved && let Some(cwd) = cwd {
            self.switch_to(&cwd);
        }
    }

    fn switch_to(&mut self, cwd: &Path) {
        let next = Target::resolve(cwd);
        if let Some(current) = &self.target {
            if current.root == next.root && current.file == next.file {
                self.target = Some(next);
                return;
            }
            let key = file_key(current);
            self.positions.insert(key, (self.cursor, self.scroll));
            self.switched_at = Some(Instant::now());
        }
        let key = file_key(&next);
        self.target = Some(next);
        self.load();
        let (cursor, scroll) = self.positions.get(&key).copied().unwrap_or((0, 0));
        self.cursor = cursor;
        self.scroll = scroll;
        self.clamp_cursor();
        self.refresh_ledger();
    }

    // ----- file state -----

    fn load(&mut self) {
        let file = self.target.as_ref().and_then(|t| t.file.clone());
        match file.as_deref().map(std::fs::read_to_string) {
            Some(Ok(source)) => {
                self.doc = Doc::parse(&source);
                self.source = source;
                self.mtime = file.as_deref().and_then(modified);
            }
            _ => {
                self.doc = Doc::default();
                self.source.clear();
                self.mtime = None;
            }
        }
        self.clamp_cursor();
    }

    fn check_file(&mut self) {
        let Some(target) = &mut self.target else {
            return;
        };
        let found = target.find_file();
        if found != target.file {
            target.file = found;
            self.load();
            return;
        }
        let mtime = target.file.as_deref().and_then(modified);
        if mtime != self.mtime {
            self.load();
            self.flash("● updated");
        }
    }

    fn write(&mut self, source: &str) {
        let Some(path) = self.target.as_ref().and_then(|t| t.file.clone()) else {
            return;
        };
        match crate::resolve::write_atomically(&path, source) {
            Ok(()) => self.load(),
            Err(err) => self.flash(format!("write failed: {err}")),
        }
    }

    fn create_file(&mut self) {
        let Some(target) = &mut self.target else {
            return;
        };
        if target.file.is_some() {
            return;
        }
        match target.create() {
            Ok(excluded) => {
                self.load();
                self.flash(if excluded {
                    "created · .herdr/ kept private via .git/info/exclude"
                } else {
                    "created .herdr/wherewasi.md"
                });
            }
            Err(err) => self.flash(format!("create failed: {err}")),
        }
    }

    fn refresh_ledger(&mut self) {
        self.ledger = self.target.as_ref().and_then(|t| ledger_line(&t.root));
    }

    fn flash(&mut self, message: impl Into<String>) {
        self.message = Some((message.into(), Instant::now()));
    }

    // ----- cursor and folding -----

    fn folds(&self) -> HashSet<String> {
        self.target
            .as_ref()
            .and_then(|t| self.folds.get(&file_key(t)))
            .cloned()
            .unwrap_or_default()
    }

    fn selectable(&self) -> Vec<usize> {
        layout::visible_lines(&self.doc, &self.folds())
            .into_iter()
            .filter(|i| self.doc.lines[*i].kind != Kind::Blank)
            .collect()
    }

    fn clamp_cursor(&mut self) {
        let selectable = self.selectable();
        if selectable.is_empty() {
            self.cursor = 0;
            return;
        }
        if !selectable.contains(&self.cursor) {
            self.cursor = selectable
                .iter()
                .copied()
                .rfind(|i| *i <= self.cursor)
                .unwrap_or(selectable[0]);
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let selectable = self.selectable();
        if selectable.is_empty() {
            return;
        }
        let at = selectable
            .iter()
            .position(|i| *i == self.cursor)
            .unwrap_or(0);
        let next = (at as isize + delta).clamp(0, selectable.len() as isize - 1) as usize;
        self.cursor = selectable[next];
    }

    fn toggle_fold(&mut self) {
        let Some(index) = self.doc.section_at(self.cursor) else {
            return;
        };
        let section = &self.doc.sections[index];
        if section.pinned {
            return;
        }
        let (key, heading) = (section.key(), section.line);
        let Some(target) = &self.target else { return };
        let folds = self.folds.entry(file_key(target)).or_default();
        if !folds.remove(&key) {
            folds.insert(key);
            self.cursor = heading;
        }
    }

    fn toggle_all_folds(&mut self) {
        let Some(target) = &self.target else { return };
        let keys: HashSet<String> = self
            .doc
            .sections
            .iter()
            .filter(|s| !s.pinned)
            .map(|s| s.key())
            .collect();
        let folds = self.folds.entry(file_key(target)).or_default();
        if folds.is_empty() {
            *folds = keys;
        } else {
            folds.clear();
        }
        self.clamp_cursor();
    }

    fn toggle_task(&mut self) {
        let Kind::Task { src_line, .. } = self
            .doc
            .lines
            .get(self.cursor)
            .map(|l| &l.kind)
            .cloned()
            .unwrap_or(Kind::Blank)
        else {
            return;
        };
        if let Some(source) = markdown::toggle_task(&self.source, src_line) {
            self.write(&source);
        }
    }

    // ----- keys -----

    fn on_key(&mut self, key: KeyEvent, terminal: &mut DefaultTerminal) -> Result<()> {
        if self.help {
            self.help = false;
            return Ok(());
        }
        let half = (self.body_height / 2).max(1) as isize;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('q') => self.close(),
            KeyCode::Char('c') if ctrl => self.close(),
            KeyCode::Char('j') | KeyCode::Down => self.move_cursor(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_cursor(-1),
            KeyCode::Char('d') if ctrl => self.move_cursor(half),
            KeyCode::Char('u') if ctrl => self.move_cursor(-half),
            KeyCode::PageDown => self.move_cursor(half * 2),
            KeyCode::PageUp => self.move_cursor(-half * 2),
            KeyCode::Char('g') | KeyCode::Home => self.move_cursor(isize::MIN / 2),
            KeyCode::Char('G') | KeyCode::End => self.move_cursor(isize::MAX / 2),
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle_task(),
            KeyCode::Char('z') | KeyCode::Tab => self.toggle_fold(),
            KeyCode::Char('Z') => self.toggle_all_folds(),
            KeyCode::Char('e') => self.edit(terminal)?,
            KeyCode::Char('n') => self.create_file(),
            KeyCode::Char('r') => {
                self.load();
                self.refresh_ledger();
                self.flash("reloaded");
            }
            KeyCode::Char('?') => self.help = true,
            _ => {}
        }
        Ok(())
    }

    fn close(&mut self) {
        self.quit = true;
        // Closing the pane kills this process, so clean up first.
        if let Some(tab) = &self.own_tab {
            state::forget(tab);
        }
        if let Some(pane) = &self.own_pane {
            let _ = herdr::request("plugin.pane.close", json!({ "pane_id": pane }));
        }
    }

    /// Open the file in `$EDITOR` in a herdr popup, or inline as a fallback.
    fn edit(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        if self.target.as_ref().is_some_and(|t| t.file.is_none()) {
            self.create_file();
        }
        let Some(target) = &self.target else {
            return Ok(());
        };
        let Some(file) = target.file.clone() else {
            return Ok(());
        };
        let root = target.root.clone();

        if let Ok(plugin) = std::env::var("HERDR_PLUGIN_ID") {
            let opened = herdr::request(
                "plugin.pane.open",
                json!({
                    "plugin_id": plugin,
                    "entrypoint": "editor",
                    "placement": "popup",
                    "width": "90%",
                    "height": "90%",
                    "cwd": root,
                    "env": { "WHEREWASI_FILE": file },
                }),
            );
            match opened {
                Ok(_) => return Ok(()),
                Err(err) => self.flash(format!("popup unavailable ({err}); editing inline")),
            }
        }

        ratatui::restore();
        let status = editor_command(&file).current_dir(&root).status();
        *terminal = ratatui::init();
        if let Err(err) = status {
            self.flash(format!("editor failed: {err}"));
        }
        self.load();
        Ok(())
    }

    // ----- drawing -----

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let narrow = area.width < 30;
        let show_ledger = self.ledger.is_some() && !narrow;
        let [header, ledger, top_rule, body, bottom_rule, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(u16::from(show_ledger)),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);

        frame.render_widget(self.header_line(header.width as usize, narrow), header);
        if show_ledger && let Some(text) = &self.ledger {
            frame.render_widget(Line::styled(format!(" {text}"), theme::dim()), ledger);
        }
        let rule = Line::styled("─".repeat(area.width as usize), theme::rule());
        frame.render_widget(rule.clone(), top_rule);
        frame.render_widget(rule, bottom_rule);
        frame.render_widget(self.footer_line(footer.width as usize), footer);

        self.body_height = body.height as usize;
        let content = Rect {
            x: body.x + 1,
            width: body.width.saturating_sub(1),
            ..body
        };
        if self.target.as_ref().is_some_and(|t| t.file.is_none()) {
            self.draw_empty(frame, content);
        } else {
            self.draw_body(frame, content);
        }
        if self.help {
            draw_help(frame, area);
        }
    }

    fn header_line(&self, width: usize, narrow: bool) -> Line<'static> {
        let mut left = vec![Span::raw(" ")];
        if self.switched_at.is_some_and(|t| t.elapsed() < SWITCH_FLASH) {
            left.push(Span::styled("↻ ", theme::pinned()));
        }
        match &self.target {
            Some(target) => {
                left.push(Span::styled(
                    target.repo.clone(),
                    Style::new()
                        .fg(theme::CRYSTAL_BLUE)
                        .add_modifier(Modifier::BOLD),
                ));
                if let Some(branch) = &target.branch {
                    left.push(Span::styled(format!(" · {branch}"), theme::dim()));
                }
            }
            None => left.push(Span::styled("wherewasi", theme::dim())),
        }

        let mut right = Vec::new();
        if let Some(status) = self
            .tracked
            .as_ref()
            .and_then(|t| t.agent_status.as_deref())
            && self.tracked.as_ref().is_some_and(|t| t.agent.is_some())
        {
            let (symbol, colour) = theme::agent_status(status);
            let label = if narrow {
                symbol.to_owned()
            } else {
                format!("{symbol} {status}")
            };
            right.push(Span::styled(label, Style::new().fg(colour)));
            right.push(Span::raw(" "));
        }
        justify(left, right, width)
    }

    fn footer_line(&self, width: usize) -> Line<'static> {
        let left = match &self.message {
            Some((text, at)) if at.elapsed() < MESSAGE_TTL => {
                vec![
                    Span::raw(" "),
                    Span::styled(text.clone(), Style::new().fg(theme::CARP_YELLOW)),
                ]
            }
            _ => {
                let mut spans = vec![Span::raw(" ")];
                if let Some(target) = &self.target
                    && let Some(file) = &target.file
                {
                    spans.push(Span::styled(display_path(file, &target.root), theme::dim()));
                    if let Some(age) = self.mtime.and_then(|m| m.elapsed().ok()) {
                        spans.push(Span::styled(format!(" · {}", ago(age)), theme::dim()));
                    }
                }
                spans
            }
        };
        let key = Style::new().fg(theme::OLD_WHITE);
        let mut right = Vec::new();
        if let Some(version) = &self.update
            && width >= 46
        {
            let hint = format!("↑ v{version}  ");
            right.push(Span::styled(hint, Style::new().fg(theme::CARP_YELLOW)));
        }
        right.extend(if width >= 34 {
            vec![
                Span::styled("e", key),
                Span::styled(" edit  ", theme::dim()),
                Span::styled("?", key),
                Span::styled(" keys ", theme::dim()),
            ]
        } else {
            vec![Span::styled("?", key), Span::raw(" ")]
        });
        justify(left, right, width)
    }

    fn draw_body(&mut self, frame: &mut Frame, area: Rect) {
        let width = area.width as usize;
        let rows: Vec<Row> = layout::rows(&self.doc, &self.folds(), width);
        let height = area.height as usize;

        // Keep every row of the cursor line on screen.
        let first = rows.iter().position(|r| r.line == self.cursor);
        let last = rows.iter().rposition(|r| r.line == self.cursor);
        if let (Some(first), Some(last)) = (first, last) {
            if first < self.scroll {
                self.scroll = first;
            } else if last >= self.scroll + height {
                self.scroll = (last + 1).saturating_sub(height).min(first);
            }
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(height));

        let lines: Vec<Line> = rows
            .into_iter()
            .skip(self.scroll)
            .take(height)
            .map(|row| {
                if row.line != self.cursor {
                    return row.content;
                }
                let mut line = row.content;
                if let Some(first) = line.spans.first_mut()
                    && first.content.starts_with(' ')
                {
                    let rest = first.content[1..].to_owned();
                    let style = first.style;
                    *first = Span::styled(rest, style);
                    line.spans
                        .insert(0, Span::styled("›", Style::new().fg(theme::CARP_YELLOW)));
                }
                line.style(Style::new().bg(theme::cursor_bg()))
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), area);
    }

    fn draw_empty(&self, frame: &mut Frame, area: Rect) {
        let repo = self
            .target
            .as_ref()
            .map_or("this folder".to_owned(), |t| t.repo.clone());
        let key = Style::new()
            .fg(theme::CARP_YELLOW)
            .add_modifier(Modifier::BOLD);
        let lines = vec![
            Line::from(vec![
                Span::styled("no notes yet for ", theme::dim()),
                Span::styled(repo, Style::new().fg(theme::CRYSTAL_BLUE)),
            ]),
            Line::default(),
            Line::from(vec![
                Span::styled("n", key),
                Span::styled("  create .herdr/wherewasi.md", theme::dim()),
            ]),
            Line::from(vec![
                Span::styled("e", key),
                // Padded to the line above so the centred keys line up.
                Span::styled("  create and open in editor ", theme::dim()),
            ]),
        ];
        let top = area.height.saturating_sub(lines.len() as u16) / 3;
        let area = Rect {
            y: area.y + top,
            height: area.height - top,
            ..area
        };
        frame.render_widget(Paragraph::new(lines).centered(), area);
    }
}

fn draw_help(frame: &mut Frame, area: Rect) {
    let entries = [
        ("j / k", "move"),
        ("^d / ^u", "half page"),
        ("g / G", "top / bottom"),
        ("space", "toggle checkbox"),
        ("z / tab", "fold section"),
        ("Z", "fold / unfold all"),
        ("e", "edit in $EDITOR"),
        ("n", "create wherewasi file"),
        ("r", "reload"),
        ("q", "close sidebar"),
    ];
    let key = Style::new().fg(theme::CARP_YELLOW);
    let lines: Vec<Line> = entries
        .iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(format!(" {k:<8}"), key),
                Span::styled(*v, theme::text()),
            ])
        })
        .collect();
    let width = 32.min(area.width);
    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::rule())
        .title(Span::styled(" keys ", theme::dim()));
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn justify(mut left: Vec<Span<'static>>, right: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let left_width: usize = left.iter().map(|s| s.content.width()).sum();
    let right_width: usize = right.iter().map(|s| s.content.width()).sum();
    if left_width + right_width < width {
        left.push(Span::raw(" ".repeat(width - left_width - right_width)));
        left.extend(right);
    }
    Line::from(left)
}

fn file_key(target: &Target) -> PathBuf {
    target
        .file
        .clone()
        .unwrap_or_else(|| target.default_file.clone())
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn display_path(file: &Path, root: &Path) -> String {
    if let Ok(relative) = file.strip_prefix(root) {
        return relative.display().to_string();
    }
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if file.starts_with(&home) => {
            format!("~/{}", file.strip_prefix(&home).unwrap_or(file).display())
        }
        _ => file.display().to_string(),
    }
}

fn ago(age: Duration) -> String {
    match age.as_secs() {
        0..60 => "just now".to_owned(),
        s @ 60..3600 => format!("{}m ago", s / 60),
        s @ 3600..86400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86400),
    }
}

fn duration(ms: u64) -> String {
    let minutes = ms / 60_000;
    if minutes < 60 {
        format!("{minutes}m")
    } else {
        format!("{}h{:02}m", minutes / 60, minutes % 60)
    }
}

/// One-line summary of the active herdr-ledger run for this worktree.
fn ledger_line(root: &Path) -> Option<String> {
    let output = Command::new("herdr-ledger")
        .arg("show")
        .current_dir(root)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let summary: Value = serde_json::from_slice(&output.stdout).ok()?;
    let run = &summary["run"];
    if run["status"].as_str()? != "active" {
        return None;
    }
    let kind = run["kind"].as_str().unwrap_or("task");
    let mut parts = vec![format!("ledger: {kind}")];
    let rounds = summary["review_rounds"].as_u64().unwrap_or(0);
    if rounds > 0 {
        parts.push(format!("r{rounds}"));
    }
    if let Some(ms) = summary["elapsed_ms"].as_u64() {
        parts.push(duration(ms));
    }
    Some(parts.join(" · "))
}

/// `$EDITOR` may carry arguments (`code -w`), so run it through the shell.
pub fn editor_command(file: &Path) -> Command {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(format!("{} \"$1\"", editor()))
        .arg("sh")
        .arg(file);
    command
}

fn editor() -> String {
    for var in ["VISUAL", "EDITOR"] {
        if let Ok(value) = std::env::var(var)
            && !value.trim().is_empty()
        {
            return value;
        }
    }
    ["nvim", "vim", "vi"]
        .into_iter()
        .find(|bin| {
            Command::new("which")
                .arg(bin)
                .output()
                .is_ok_and(|o| o.status.success())
        })
        .unwrap_or("vi")
        .to_owned()
}
