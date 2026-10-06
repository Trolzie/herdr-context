//! Section-level edits used by the agent commands (`now`, `did`, `todo`, …).
//! They keep the file markdownlint-clean: one blank line around headings
//! and lists.

use std::collections::HashSet;

use chrono::Local;

use crate::resolve::Target;

/// Most recent log entries kept; older ones are dropped.
const LOG_LIMIT: usize = 40;

/// Where each section goes if it is missing, by template order.
const ORDER: [&str; 7] = ["Now", "Goal", "Plan", "Tasks", "Log", "Decisions", "Notes"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pending,
    InProgress,
    Completed,
}

/// One step of an agent's own plan (Codex `update_plan`, Claude tasks).
#[derive(Clone, Debug)]
pub struct Step {
    pub text: String,
    pub status: Status,
}

pub struct File {
    lines: Vec<String>,
}

/// Markers around the mirrored agent plan, so it can be replaced without
/// touching anything the user wrote in `## Plan`. Hidden by the sidebar.
const PLAN_START: &str = "<!-- wherewasi:plan -->";
const PLAN_END: &str = "<!-- /wherewasi:plan -->";

impl File {
    pub fn parse(source: &str) -> Self {
        Self {
            lines: source.lines().map(str::to_owned).collect(),
        }
    }

    pub fn render(&self) -> String {
        let mut out = self.lines.join("\n").trim_end().to_owned();
        out.push('\n');
        out
    }

    /// Replace the first paragraph of `## Now`; anything after it stays.
    pub fn now(&mut self, text: &str) {
        let text = one_line(text);
        let (start, end) = self.section("Now");
        let Some(first) = (start + 1..end).find(|i| !self.lines[*i].trim().is_empty()) else {
            self.insert_body(start, end, vec![text]);
            return;
        };
        // A code block or heading first: put the text in front of it.
        if is_fence(&self.lines[first]) || is_heading(&self.lines[first]) {
            self.lines.splice(first..first, [text, String::new()]);
            return;
        }
        let last = (first..end)
            .find(|i| {
                let line = &self.lines[*i];
                line.trim().is_empty() || is_heading(line) || is_fence(line)
            })
            .unwrap_or(end);
        let mut replacement = vec![text];
        if last < end && !self.lines[last].trim().is_empty() {
            replacement.push(String::new());
        }
        self.lines.splice(first..last, replacement);
    }

    /// Add a timestamped entry to the top of `## Log`, keeping the newest
    /// `LOG_LIMIT` entries.
    pub fn did(&mut self, text: &str) {
        let stamp = Local::now().format("%b %-d %H:%M");
        let entry = format!("- `{stamp}` {}", one_line(text));
        let (start, end) = self.section("Log");
        match self.items(start, end).first() {
            Some(&first) => self.lines.insert(first, entry),
            None => self.append_line(start, end, entry),
        }
        let (start, end) = self.section("Log");
        let items = self.items(start, end);
        if items.len() > LOG_LIMIT {
            let last = *items.last().unwrap_or(&end);
            let stop = (last + 1..end)
                .find(|i| {
                    !self.lines[*i].starts_with(char::is_whitespace) || self.lines[*i].is_empty()
                })
                .unwrap_or(end);
            self.lines.drain(items[LOG_LIMIT]..stop);
            // Don't leave two blank lines where the dropped entries were.
            let at = items[LOG_LIMIT];
            while at > 0
                && at < self.lines.len()
                && self.lines[at].trim().is_empty()
                && self.lines[at - 1].trim().is_empty()
            {
                self.lines.remove(at);
            }
        }
    }

    pub fn todo(&mut self, text: &str) {
        self.append("Tasks", format!("- [ ] {}", one_line(text)));
    }

    pub fn decide(&mut self, text: &str) {
        self.append("Decisions", format!("- {}", one_line(text)));
    }

    pub fn note(&mut self, text: &str) {
        self.append("Notes", format!("- {}", one_line(text)));
    }

    pub fn contains(&self, text: &str) -> bool {
        self.lines.iter().any(|l| l.contains(text))
    }

    /// Tick the first open task containing `query` (case-insensitive).
    /// Returns the task text, or the open tasks when nothing matched.
    pub fn check(&mut self, query: &str) -> Result<String, Vec<String>> {
        let query = query.to_lowercase();
        let Some((start, end)) = self.find("Tasks") else {
            return Err(Vec::new());
        };
        let open: Vec<usize> = self
            .items(start, end)
            .into_iter()
            .filter(|i| is_open(&self.lines[*i]))
            .collect();
        let Some(&hit) = open
            .iter()
            .find(|i| item_text(&self.lines[**i]).to_lowercase().contains(&query))
        else {
            return Err(open.iter().map(|i| item_text(&self.lines[*i])).collect());
        };
        self.lines[hit] = self.lines[hit].replacen("- [ ]", "- [x]", 1);
        Ok(item_text(&self.lines[hit]))
    }

    /// Mirror an agent's own plan into `## Plan` (between markers), mark
    /// the step in progress as `## Now`, and log newly completed steps.
    pub fn set_plan(&mut self, steps: &[Step]) {
        let items: Vec<String> = steps
            .iter()
            .map(|step| {
                let text = one_line(&step.text);
                match step.status {
                    Status::Completed => format!("- [x] {text}"),
                    Status::InProgress => format!("- [ ] **{text}**"),
                    Status::Pending => format!("- [ ] {text}"),
                }
            })
            .collect();
        if items.is_empty() {
            return;
        }
        let before = self.plan_done();
        self.replace_plan_block(items);
        if let Some(step) = steps.iter().find(|s| s.status == Status::InProgress) {
            self.now(&step.text);
        }
        for step in steps {
            let text = one_line(&step.text);
            if step.status == Status::Completed && !before.contains(&text) {
                self.did(&format!("done: {text}"));
            }
        }
    }

    /// Record a freshly approved plan: title as the goal (unless one is
    /// set), its steps as the mirrored plan.
    pub fn approved_plan(&mut self, plan: &str) {
        let (title, steps) = plan_outline(plan);
        if let Some(title) = &title
            && self.body("Goal").is_empty()
        {
            let (start, end) = self.section("Goal");
            self.insert_body(start, end, vec![title.clone()]);
        }
        let steps: Vec<Step> = steps
            .into_iter()
            .map(|text| Step {
                text,
                status: Status::Pending,
            })
            .collect();
        self.set_plan(&steps);
        self.did(&format!(
            "plan approved: {}",
            title.as_deref().unwrap_or("new plan")
        ));
    }

    /// Add an open step to the mirrored plan (Claude `TaskCreated`).
    pub fn plan_add(&mut self, text: &str) {
        let text = one_line(text);
        if self.plan_item(&text).is_some() {
            return;
        }
        let mut items = self.plan_block_items();
        items.push(format!("- [ ] {text}"));
        self.replace_plan_block(items);
    }

    /// Tick a plan step (Claude `TaskCompleted`) and log it once.
    pub fn plan_complete(&mut self, text: &str) {
        let text = one_line(text);
        match self.plan_item(&text) {
            Some(i) if !is_open(&self.lines[i]) => return,
            Some(i) => self.lines[i] = format!("- [x] {text}"),
            None => {
                let mut items = self.plan_block_items();
                items.push(format!("- [x] {text}"));
                self.replace_plan_block(items);
            }
        }
        self.did(&format!("done: {text}"));
    }

    /// Texts of completed plan steps.
    fn plan_done(&self) -> HashSet<String> {
        let Some((start, end)) = self.find("Plan") else {
            return HashSet::new();
        };
        self.items(start, end)
            .into_iter()
            .filter(|i| !is_open(&self.lines[*i]) && self.lines[*i].starts_with("- ["))
            .map(|i| item_text(&self.lines[i]))
            .collect()
    }

    fn plan_item(&self, text: &str) -> Option<usize> {
        let (start, end) = self.find("Plan")?;
        self.items(start, end)
            .into_iter()
            .find(|i| item_text(&self.lines[*i]) == text)
    }

    /// Lines between the plan markers, if both are there.
    fn plan_block(&self) -> Option<(usize, usize)> {
        let (start, end) = self.find("Plan")?;
        let open = (start..end).find(|i| self.lines[*i].trim() == PLAN_START)?;
        let close = (open..end).find(|i| self.lines[*i].trim() == PLAN_END)?;
        Some((open, close))
    }

    /// Remove marker lines left without their partner, so a new block
    /// can't pair with them and swallow the user's text in between.
    fn drop_stray_markers(&mut self) {
        let Some((start, end)) = self.find("Plan") else {
            return;
        };
        let stray: Vec<usize> = (start..end)
            .filter(|i| matches!(self.lines[*i].trim(), PLAN_START | PLAN_END))
            .collect();
        for i in stray.into_iter().rev() {
            self.lines.remove(i);
        }
    }

    fn plan_block_items(&self) -> Vec<String> {
        self.plan_block()
            .map(|(open, close)| {
                self.lines[open + 1..close]
                    .iter()
                    .filter(|l| !l.trim().is_empty())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn replace_plan_block(&mut self, items: Vec<String>) {
        let mut block = vec![PLAN_START.to_owned(), String::new()];
        block.extend(items);
        block.extend([String::new(), PLAN_END.to_owned()]);
        match self.plan_block() {
            Some((open, close)) => {
                self.lines.splice(open..=close, block);
            }
            None => {
                self.drop_stray_markers();
                let (start, end) = self.section("Plan");
                match (start + 1..end)
                    .rev()
                    .find(|i| !self.lines[*i].trim().is_empty())
                {
                    // After whatever the user already wrote in `## Plan`.
                    Some(last) => {
                        let mut insert = vec![String::new()];
                        insert.extend(block);
                        self.lines.splice(last + 1..last + 1, insert);
                    }
                    None => self.insert_body(start, end, block),
                }
            }
        }
    }

    /// Add a list item after a section's last content line.
    fn append(&mut self, section: &str, item: String) {
        let (start, end) = self.section(section);
        self.append_line(start, end, item);
    }

    fn append_line(&mut self, start: usize, end: usize, item: String) {
        match (start + 1..end)
            .rev()
            .find(|i| !self.lines[*i].trim().is_empty())
        {
            Some(last) => {
                let in_list = self.lines[last].starts_with("- ")
                    || self.lines[last].starts_with(char::is_whitespace);
                let mut insert = Vec::new();
                // Keep lists separated from a preceding paragraph.
                if !in_list {
                    insert.push(String::new());
                }
                insert.push(item);
                self.lines.splice(last + 1..last + 1, insert);
            }
            None => self.insert_body(start, end, vec![item]),
        }
    }

    /// Fill an empty section body.
    fn insert_body(&mut self, start: usize, end: usize, body: Vec<String>) {
        let mut replacement = vec![String::new()];
        replacement.extend(body);
        replacement.push(String::new());
        self.lines.splice(start + 1..end, replacement);
    }

    /// Top-level list items (`- `) in a range, skipping fenced code.
    fn items(&self, start: usize, end: usize) -> Vec<usize> {
        let mut fenced = false;
        (start + 1..end)
            .filter(|i| {
                let line = &self.lines[*i];
                if is_fence(line) {
                    fenced = !fenced;
                    return false;
                }
                !fenced && line.starts_with("- ")
            })
            .collect()
    }

    /// Top-level list items of a section (tests and read-only use).
    #[cfg(test)]
    fn list(&self, section: &str) -> Vec<String> {
        self.find(section)
            .map(|(start, end)| {
                self.items(start, end)
                    .into_iter()
                    .map(|i| self.lines[i].clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Body lines with surrounding blank lines trimmed (read-only).
    fn body(&self, section: &str) -> Vec<String> {
        let Some((start, end)) = self.find(section) else {
            return Vec::new();
        };
        let mut body: Vec<String> = self.lines[start + 1..end].to_vec();
        while body.first().is_some_and(|l| l.trim().is_empty()) {
            body.remove(0);
        }
        while body.last().is_some_and(|l| l.trim().is_empty()) {
            body.pop();
        }
        body
    }

    /// The section's range, creating the heading if it is missing.
    fn section(&mut self, section: &str) -> (usize, usize) {
        match self.find(section) {
            Some(range) => range,
            None => self.insert_heading(section),
        }
    }

    /// `(heading line, end of body)` for a `##` section, ignoring headings
    /// inside fenced code blocks.
    fn find(&self, section: &str) -> Option<(usize, usize)> {
        let mut fenced = false;
        let mut start = None;
        for (i, line) in self.lines.iter().enumerate() {
            if is_fence(line) {
                fenced = !fenced;
                continue;
            }
            if fenced {
                continue;
            }
            match start {
                None => {
                    let hit = line
                        .strip_prefix("## ")
                        .is_some_and(|t| t.trim().eq_ignore_ascii_case(section));
                    if hit {
                        start = Some(i);
                    }
                }
                Some(s) if line.starts_with("# ") || line.starts_with("## ") => {
                    return Some((s, i));
                }
                Some(_) => {}
            }
        }
        start.map(|s| (s, self.lines.len()))
    }

    /// Insert a missing heading before the next section in template order.
    fn insert_heading(&mut self, section: &str) -> (usize, usize) {
        let rank = ORDER.iter().position(|s| s.eq_ignore_ascii_case(section));
        let before = rank.and_then(|rank| {
            ORDER[rank + 1..]
                .iter()
                .find_map(|later| self.find(later).map(|(start, _)| start))
        });
        let at = before.unwrap_or(self.lines.len());
        let mut block = Vec::new();
        if at > 0 && !self.lines[at - 1].trim().is_empty() {
            block.push(String::new());
        }
        block.push(format!("## {section}"));
        block.push(String::new());
        let heading = at + block.len() - 2;
        self.lines.splice(at..at, block);
        (heading, heading + 2)
    }
}

fn is_fence(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("```") || line.starts_with("~~~")
}

/// A Markdown heading: `#`s followed by a space.
fn is_heading(line: &str) -> bool {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes) && line[hashes..].starts_with(' ')
}

fn is_open(line: &str) -> bool {
    line == "- [ ]" || line.starts_with("- [ ] ")
}

/// Apply edits to a target's file, creating it from the template if needed.
pub fn modify(target: &mut Target, apply: impl FnOnce(&mut File)) -> std::io::Result<()> {
    let path = target.file_or_create()?;
    let mut file = File::parse(&std::fs::read_to_string(&path)?);
    apply(&mut file);
    crate::resolve::write_atomically(&path, &file.render())
}

/// Turn an approved Markdown plan into a title and a list of steps: its
/// top-level numbered items, or else its section headings.
pub fn plan_outline(plan: &str) -> (Option<String>, Vec<String>) {
    // Sections that describe the plan rather than list its steps.
    const NOT_STEPS: [&str; 14] = [
        "context",
        "overview",
        "summary",
        "background",
        "goal",
        "goals",
        "verification",
        "testing",
        "tests",
        "files",
        "critical files",
        "notes",
        "risks",
        "open questions",
    ];
    let clean = |text: &str| one_line(&text.replace("**", "").replace('`', ""));
    let skip = |heading: &str| {
        let bare = heading.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ');
        NOT_STEPS.contains(&bare.to_lowercase().as_str())
    };
    let title = plan.lines().find_map(|l| l.strip_prefix("# ")).map(|t| {
        let t = clean(t);
        let stripped = t.strip_prefix("Plan:").or_else(|| t.strip_prefix("Plan -"));
        stripped.map_or(t.clone(), |rest| rest.trim().to_owned())
    });

    // Numbered top-level items, except under sections like "Verification".
    let mut numbered = Vec::new();
    let mut headings = Vec::new();
    let mut in_skipped = false;
    let mut fenced = false;
    for line in plan.lines() {
        if is_fence(line) {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if let Some(heading) = line
            .strip_prefix("## ")
            .or_else(|| line.strip_prefix("### "))
        {
            let heading = clean(heading);
            in_skipped = skip(&heading);
            if !in_skipped {
                headings.push(heading);
            }
            continue;
        }
        if in_skipped {
            continue;
        }
        if let Some((number, rest)) = line.split_once(". ")
            && !number.is_empty()
            && number.chars().all(|c| c.is_ascii_digit())
        {
            let rest = clean(rest);
            if !rest.is_empty() {
                numbered.push(rest);
            }
        }
    }
    let steps = if numbered.len() >= 2 {
        numbered
    } else {
        headings
    };
    (title, steps.into_iter().take(12).collect())
}

/// Collapse whitespace so multi-line steps stay one list item.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text of a `- [ ]` / `- [x]` item, without bold markers.
fn item_text(line: &str) -> String {
    let rest = ["- [ ]", "- [x]", "- [X]"]
        .iter()
        .find_map(|m| line.strip_prefix(m))
        .unwrap_or_else(|| line.strip_prefix("- ").unwrap_or(line));
    rest.trim()
        .trim_start_matches("**")
        .trim_end_matches("**")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::template;

    #[test]
    fn now_replaces_placeholder() {
        let mut file = File::parse(&template("demo"));
        file.now("Fixing the login redirect");
        let out = file.render();
        assert!(out.contains("## Now\n\nFixing the login redirect\n\n## Goal"));
        assert!(!out.contains("Nothing in progress"));
    }

    #[test]
    fn log_is_newest_first_and_capped() {
        let mut file = File::parse(&template("demo"));
        file.did("first");
        file.did("second");
        let out = file.render();
        let first = out.find("first").unwrap();
        let second = out.find("second").unwrap();
        assert!(second < first);
        for i in 0..LOG_LIMIT + 5 {
            file.did(&format!("entry {i}"));
        }
        assert_eq!(file.list("Log").len(), LOG_LIMIT);
    }

    #[test]
    fn todo_and_check() {
        let mut file = File::parse(&template("demo"));
        file.todo("write tests");
        file.todo("ship it");
        assert_eq!(file.check("SHIP").unwrap(), "ship it");
        assert!(file.render().contains("- [ ] write tests\n- [x] ship it\n"));
        assert_eq!(file.check("nope").unwrap_err(), vec!["write tests"]);
    }

    #[test]
    fn missing_sections_are_added_in_order() {
        let mut file = File::parse("# demo\n\n## Notes\n\nkeep me\n");
        file.decide("use Rust");
        let out = file.render();
        assert_eq!(
            out,
            "# demo\n\n## Decisions\n\n- use Rust\n\n## Notes\n\nkeep me\n"
        );
    }

    fn step(text: &str, status: Status) -> Step {
        Step {
            text: text.to_owned(),
            status,
        }
    }

    #[test]
    fn plan_mirror_sets_now_and_logs_new_completions() {
        let mut file = File::parse(&template("demo"));
        file.set_plan(&[
            step("Reproduce", Status::Completed),
            step("Fix", Status::InProgress),
            step("Test", Status::Pending),
        ]);
        let out = file.render();
        assert!(out.contains(
            "## Plan\n\n<!-- wherewasi:plan -->\n\n- [x] Reproduce\n- [ ] **Fix**\n- [ ] Test\n\n<!-- /wherewasi:plan -->\n\n## Tasks"
        ));
        assert!(out.contains("## Now\n\nFix\n"));
        assert_eq!(file.list("Log").len(), 1);

        // Re-sending the same plan logs nothing new; finishing Fix logs once.
        file.set_plan(&[
            step("Reproduce", Status::Completed),
            step("Fix", Status::Completed),
            step("Test", Status::InProgress),
        ]);
        let log = file.list("Log");
        assert_eq!(log.len(), 2);
        assert!(log[0].ends_with("done: Fix"));
    }

    #[test]
    fn approved_plan_uses_numbered_steps_or_headings() {
        let numbered = "# Fix login\n\n## Context\nwhy\n\n## Approach\n\n1. **Reproduce** it\n2. Patch `next`\n   1. nested\n";
        let (title, steps) = plan_outline(numbered);
        assert_eq!(title.as_deref(), Some("Fix login"));
        assert_eq!(steps, vec!["Reproduce it", "Patch next"]);

        let headings =
            "# Sidebar\n\n## Context\n\n## 1. Parser\n\n## 2. Renderer\n\n## Verification\n";
        assert_eq!(plan_outline(headings).1, vec!["1. Parser", "2. Renderer"]);

        let mixed = "# Plan: Fix flaky test\n\n## Steps\n\n1. Pin the seed\n2. Retry once\n\n## Verification\n\n1. Run cargo test\n2. Loop 100 times\n";
        let (title, steps) = plan_outline(mixed);
        assert_eq!(title.as_deref(), Some("Fix flaky test"));
        assert_eq!(steps, vec!["Pin the seed", "Retry once"]);

        let mut file = File::parse(&template("demo"));
        file.approved_plan(numbered);
        let out = file.render();
        assert!(out.contains("## Goal\n\nFix login\n"));
        assert!(out.contains("- [ ] Reproduce it\n- [ ] Patch next\n"));
    }

    #[test]
    fn claude_tasks_add_and_complete_once() {
        let mut file = File::parse(&template("demo"));
        file.plan_add("Write migration");
        file.plan_add("Write migration");
        file.plan_complete("Write migration");
        file.plan_complete("Write migration");
        assert_eq!(file.list("Plan"), vec!["- [x] Write migration"]);
        assert_eq!(file.list("Log").len(), 1);
    }

    #[test]
    fn edits_keep_hand_written_content() {
        let source = "## Now\n\nold focus\n\n### Context\n\nkeep me\n\n## Tasks\n\nIntro paragraph.\n\n- [ ] alpha\n  - [ ] nested\n- [ ] beta\n\n```md\n## Log\n- fake\n```\n\n* [ ] star item\n\n## Log\n\n- `x` first\n  continued here\n";
        let mut file = File::parse(source);
        assert_eq!(file.check("beta").unwrap(), "beta");
        file.did("second");
        file.now("new focus");
        let out = file.render();
        for kept in [
            "Intro paragraph.",
            "  - [ ] nested",
            "```md\n## Log\n- fake\n```",
            "* [ ] star item",
            "  continued here",
            "### Context\n\nkeep me",
        ] {
            assert!(out.contains(kept), "lost {kept:?} in\n{out}");
        }
        assert!(out.contains("- [x] beta"));
        assert!(out.contains("## Now\n\nnew focus\n"));
        let log = out.split("## Log\n").last().unwrap();
        assert!(log.find("second").unwrap() < log.find("first").unwrap());
    }

    #[test]
    fn check_handles_odd_task_lines() {
        let mut file = File::parse("## Tasks\n\n- [ ]\n- [ ]éa\n- [ ] real\n");
        assert_eq!(file.check("nothing").unwrap_err(), vec!["", "real"]);
        assert_eq!(file.check("real").unwrap(), "real");
    }

    #[test]
    fn mirrored_plan_leaves_user_plan_lines() {
        let mut file = File::parse("## Plan\n\nMy own notes.\n\n- [ ] user step\n");
        file.set_plan(&[step("Agent step", Status::Pending)]);
        file.set_plan(&[step("Agent step", Status::Completed)]);
        let out = file.render();
        assert!(out.contains("My own notes.") && out.contains("- [ ] user step"));
        assert_eq!(
            out.matches("Agent step").count(),
            2,
            "plan once + one log entry:\n{out}"
        );
    }

    #[test]
    fn appending_after_a_paragraph_keeps_a_blank_line() {
        let mut file = File::parse("## Notes\n\nsome prose\n");
        file.note("a bullet");
        assert_eq!(file.render(), "## Notes\n\nsome prose\n\n- a bullet\n");
    }

    #[test]
    fn now_respects_fences_and_headings() {
        let mut file = File::parse("## Now\n\n```sh\nmake\n```\n\n## Goal\n\nship\n");
        file.now("focus");
        file.decide("x");
        let out = file.render();
        assert!(
            out.starts_with("## Now\n\nfocus\n\n```sh\nmake\n```\n"),
            "{out}"
        );
        assert_eq!(out.matches("## Goal").count(), 1, "{out}");

        let mut file = File::parse("## Now\n\n#123 bug\nmore\n### Context\n\nkeep\n");
        file.now("focus");
        assert_eq!(file.render(), "## Now\n\nfocus\n\n### Context\n\nkeep\n");
    }

    #[test]
    fn dangling_plan_marker_does_not_eat_user_text() {
        let source = "## Plan\n\n<!-- wherewasi:plan -->\n\n- [ ] old mirrored\n\nMy notes after a lost end marker.\n";
        let mut file = File::parse(source);
        file.set_plan(&[step("A", Status::Pending)]);
        file.set_plan(&[step("B", Status::Pending)]);
        let out = file.render();
        assert!(out.contains("My notes after a lost end marker."), "{out}");
        assert!(out.contains("- [ ] old mirrored"), "{out}");
        assert_eq!(out.matches(PLAN_START).count(), 1, "{out}");
        assert!(out.contains("- [ ] B") && !out.contains("- [ ] A"), "{out}");
    }

    #[test]
    fn log_trim_in_loose_list_leaves_single_blank() {
        let mut source = String::from("## Log\n\n");
        for i in 0..LOG_LIMIT {
            source.push_str(&format!("- e{i}\n\n"));
        }
        source.push_str("trailing paragraph\n");
        let mut file = File::parse(&source);
        file.did("newest");
        let out = file.render();
        assert!(!out.contains("\n\n\n"), "{out}");
        assert!(out.contains("trailing paragraph"));
    }

    #[test]
    fn hand_ticked_capital_x_counts_as_done() {
        let mut file = File::parse(
            "## Plan\n\n<!-- wherewasi:plan -->\n\n- [X] A\n\n<!-- /wherewasi:plan -->\n",
        );
        file.set_plan(&[step("A", Status::Completed)]);
        assert!(!file.render().contains("done: A"));
    }
}
