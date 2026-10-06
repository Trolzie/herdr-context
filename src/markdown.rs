//! Parse Markdown into width-independent document lines. Wrapping, folding
//! and the cursor are applied later in `layout`.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use crate::theme;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Blank,
    Text,
    Heading {
        level: u8,
        title: String,
    },
    Task {
        checked: bool,
        src_line: usize,
    },
    CodeTop {
        lang: String,
        width: usize,
    },
    Code {
        text: String,
        width: usize,
    },
    CodeBottom {
        width: usize,
    },
    Rule,
    /// Pre-aligned row that must not be wrapped.
    Fixed,
}

#[derive(Clone, Debug)]
pub struct DocLine {
    pub kind: Kind,
    /// Prefix on the first visual row (indent, bullet, quote bar).
    pub prefix: Vec<Span<'static>>,
    /// Prefix repeated on wrapped continuation rows.
    pub cont: Vec<Span<'static>>,
    pub spans: Vec<Span<'static>>,
    /// Starts a list item (used for folded-section counts).
    pub item: bool,
}

impl DocLine {
    fn blank() -> Self {
        Self {
            kind: Kind::Blank,
            prefix: Vec::new(),
            cont: Vec::new(),
            spans: Vec::new(),
            item: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Section {
    pub level: u8,
    pub title: String,
    /// Index of the heading line.
    pub line: usize,
    /// One past the last line of the section body.
    pub end: usize,
    pub tasks_done: usize,
    pub tasks_total: usize,
    pub items: usize,
    pub pinned: bool,
}

impl Section {
    /// Key used to remember folds across reloads.
    pub fn key(&self) -> String {
        format!("{}:{}", self.level, self.title)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Doc {
    pub lines: Vec<DocLine>,
    pub sections: Vec<Section>,
}

impl Doc {
    pub fn parse(source: &str) -> Self {
        let mut builder = Builder::new(source);
        builder.run();
        let mut lines = builder.lines;
        while lines.last().is_some_and(|l| l.kind == Kind::Blank) {
            lines.pop();
        }
        pin_now_section(&mut lines);
        let sections = index_sections(&lines);
        Self { lines, sections }
    }

    /// Innermost section containing `line` (a heading belongs to its own).
    pub fn section_at(&self, line: usize) -> Option<usize> {
        self.sections
            .iter()
            .enumerate()
            .filter(|(_, s)| s.line <= line && line < s.end)
            .max_by_key(|(_, s)| s.line)
            .map(|(i, _)| i)
    }
}

fn is_pinned(title: &str) -> bool {
    title.trim().eq_ignore_ascii_case("now")
}

/// Move a `## Now` section to the top so it is always the first thing seen.
fn pin_now_section(lines: &mut Vec<DocLine>) {
    let sections = index_sections(lines);
    let Some(now) = sections.iter().find(|s| s.pinned) else {
        return;
    };
    if now.line == 0 {
        return;
    }
    let mut moved: Vec<DocLine> = lines.drain(now.line..now.end).collect();
    while moved.last().is_some_and(|l| l.kind == Kind::Blank) {
        moved.pop();
    }
    moved.push(DocLine::blank());
    while lines.last().is_some_and(|l| l.kind == Kind::Blank) {
        lines.pop();
    }
    lines.splice(0..0, moved);
}

fn index_sections(lines: &[DocLine]) -> Vec<Section> {
    let mut sections = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Kind::Heading { level, title } = &line.kind else {
            continue;
        };
        let end = lines[i + 1..]
            .iter()
            .position(|l| matches!(&l.kind, Kind::Heading { level: other, .. } if other <= level))
            .map_or(lines.len(), |offset| i + 1 + offset);
        let body = &lines[i + 1..end];
        let tasks = body.iter().filter_map(|l| match l.kind {
            Kind::Task { checked, .. } => Some(checked),
            _ => None,
        });
        let (done, total) = tasks.fold((0, 0), |(d, t), checked| (d + usize::from(checked), t + 1));
        sections.push(Section {
            level: *level,
            title: title.clone(),
            line: i,
            end,
            tasks_done: done,
            tasks_total: total,
            items: body.iter().filter(|l| l.item).count(),
            pinned: is_pinned(title),
        });
    }
    sections
}

const INDENT: &str = "  ";

struct ListState {
    next: Option<u64>,
}

struct Builder<'a> {
    source: &'a str,
    lines: Vec<DocLine>,
    spans: Vec<Span<'static>>,
    bold: usize,
    italic: usize,
    strike: usize,
    link: usize,
    heading: Option<u8>,
    quote: usize,
    lists: Vec<ListState>,
    /// Marker waiting for the first line of the current list item.
    pending_marker: Option<String>,
    task: Option<(bool, usize)>,
    code: Option<(String, Vec<String>)>,
    table: Option<Vec<Vec<String>>>,
    cell: String,
}

impl<'a> Builder<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            lines: Vec::new(),
            spans: Vec::new(),
            bold: 0,
            italic: 0,
            strike: 0,
            link: 0,
            heading: None,
            quote: 0,
            lists: Vec::new(),
            pending_marker: None,
            task: None,
            code: None,
            table: None,
            cell: String::new(),
        }
    }

    fn run(&mut self) {
        let options =
            Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
        let events: Vec<_> = Parser::new_ext(self.source, options)
            .into_offset_iter()
            .collect();
        for (event, range) in events {
            self.event(event, range.start);
        }
        self.flush();
    }

    fn style(&self) -> Style {
        let mut style = match self.heading {
            Some(level) => theme::heading(level),
            None if self.quote > 0 => theme::quote(),
            None => theme::text(),
        };
        if self.bold > 0 {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.italic > 0 {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.strike > 0 {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        if self.link > 0 {
            style = theme::link();
        }
        style
    }

    fn push_text(&mut self, text: &str, style: Style) {
        if self.table.is_some() {
            self.cell.push_str(text);
            return;
        }
        self.spans.push(Span::styled(text.to_owned(), style));
    }

    fn line_of(&self, offset: usize) -> usize {
        self.source[..offset.min(self.source.len())]
            .matches('\n')
            .count()
    }

    fn blank(&mut self) {
        if self.lines.last().is_some_and(|l| l.kind != Kind::Blank) {
            self.lines.push(DocLine::blank());
        }
    }

    fn base_prefix(&self) -> Vec<Span<'static>> {
        let mut prefix = vec![Span::raw(INDENT)];
        for _ in 0..self.quote {
            prefix.push(Span::styled("┃ ", theme::rule()));
        }
        // Nested lists indent by two columns per level.
        for _ in 1..self.lists.len() {
            prefix.push(Span::raw("  "));
        }
        prefix
    }

    /// Turn buffered inline spans into a document line.
    fn flush(&mut self) {
        if self.spans.is_empty() && self.pending_marker.is_none() {
            return;
        }
        let mut spans = std::mem::take(&mut self.spans);
        // Drop leading whitespace left over from soft breaks.
        if let Some(first) = spans.first_mut() {
            first.content = first.content.trim_start().to_owned().into();
        }
        let mut prefix = self.base_prefix();
        let mut cont = prefix.clone();
        let mut kind = Kind::Text;
        let mut item = false;

        if let Some(marker) = self.pending_marker.take() {
            item = true;
            if let Some((checked, src_line)) = self.task.take() {
                kind = Kind::Task { checked, src_line };
                if checked {
                    prefix.push(Span::styled("✔ ", theme::dim().fg(theme::SPRING_GREEN)));
                    for span in &mut spans {
                        span.style = theme::task_done();
                    }
                } else {
                    prefix.push(Span::styled("☐ ", Style::new().fg(theme::OLD_WHITE)));
                }
                cont.push(Span::raw("  "));
            } else {
                let width = marker.width();
                prefix.push(Span::styled(marker, theme::dim()));
                cont.push(Span::raw(" ".repeat(width)));
            }
        } else if !self.lists.is_empty() {
            // Continuation paragraph inside a list item.
            prefix.push(Span::raw("  "));
            cont.push(Span::raw("  "));
        }

        if let Some(level) = self.heading {
            let title: String = spans.iter().map(|s| s.content.as_ref()).collect();
            kind = Kind::Heading { level, title };
            prefix.clear();
            cont = vec![Span::raw("  ")];
        }

        self.lines.push(DocLine {
            kind,
            prefix,
            cont,
            spans,
            item,
        });
    }

    fn event(&mut self, event: Event, offset: usize) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                if let Some((_, lines)) = &mut self.code {
                    lines.extend(text.lines().map(str::to_owned));
                } else {
                    let style = self.style();
                    self.push_text(&text, style);
                }
            }
            Event::Code(code) => self.push_text(&code, theme::inline_code()),
            Event::InlineMath(math) | Event::DisplayMath(math) => {
                self.push_text(&math, theme::inline_code());
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                let html = html.trim_end_matches('\n').to_owned();
                self.push_text(&html, theme::dim());
            }
            Event::SoftBreak => self.push_text(" ", theme::text()),
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.flush();
                self.blank();
                self.lines.push(DocLine {
                    kind: Kind::Rule,
                    ..DocLine::blank()
                });
                self.blank();
            }
            Event::TaskListMarker(checked) => {
                self.task = Some((checked, self.line_of(offset)));
            }
            Event::FootnoteReference(name) => {
                self.push_text(&format!("[^{name}]"), theme::dim());
            }
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush();
                self.blank();
                self.heading = Some(heading_level(level));
            }
            Tag::Paragraph => {}
            Tag::BlockQuote(_) => {
                self.flush();
                self.quote += 1;
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_owned()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, Vec::new()));
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push(ListState { next: start });
            }
            Tag::Item => {
                self.flush();
                let depth = self.lists.len();
                let marker = match self.lists.last_mut() {
                    Some(ListState { next: Some(n) }) => {
                        let marker = format!("{n}. ");
                        *n += 1;
                        marker
                    }
                    _ => match depth {
                        0 | 1 => "• ",
                        2 => "◦ ",
                        _ => "▪ ",
                    }
                    .to_owned(),
                };
                self.pending_marker = Some(marker);
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { .. } => self.link += 1,
            Tag::Image { .. } => self.push_text("🖼 ", theme::dim()),
            Tag::Table(_) => {
                self.flush();
                self.blank();
                self.table = Some(Vec::new());
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(table) = &mut self.table {
                    table.push(Vec::new());
                }
            }
            Tag::TableCell => self.cell.clear(),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.flush();
                self.heading = None;
            }
            TagEnd::Paragraph => {
                self.flush();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote = self.quote.saturating_sub(1);
                if self.quote == 0 {
                    self.blank();
                }
            }
            TagEnd::CodeBlock => self.code_block(),
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Item => self.flush(),
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                self.link = self.link.saturating_sub(1);
                self.push_text(" ↗", theme::dim());
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.cell).trim().to_owned();
                if let Some(row) = self.table.as_mut().and_then(|t| t.last_mut()) {
                    row.push(cell);
                }
            }
            TagEnd::Table => self.table_block(),
            _ => {}
        }
    }

    fn code_block(&mut self) {
        let Some((lang, code)) = self.code.take() else {
            return;
        };
        let prefix = self.base_prefix();
        let width = code
            .iter()
            .map(|l| l.width())
            .chain([lang.width() + 2])
            .max()
            .unwrap_or(0);
        let line = |kind| DocLine {
            kind,
            prefix: prefix.clone(),
            cont: prefix.clone(),
            spans: Vec::new(),
            item: false,
        };
        self.lines.push(line(Kind::CodeTop { lang, width }));
        for text in code {
            let text = text.replace('\t', "    ");
            self.lines.push(line(Kind::Code { text, width }));
        }
        self.lines.push(line(Kind::CodeBottom { width }));
        if self.lists.is_empty() {
            self.blank();
        }
    }

    fn table_block(&mut self) {
        let Some(rows) = self.table.take() else {
            return;
        };
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        let widths: Vec<usize> = (0..columns)
            .map(|c| {
                rows.iter()
                    .filter_map(|r| r.get(c))
                    .map(|s| s.width())
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let prefix = self.base_prefix();
        for (i, row) in rows.iter().enumerate() {
            let style = if i == 0 {
                theme::text().add_modifier(Modifier::BOLD)
            } else {
                theme::text()
            };
            let mut spans = Vec::new();
            for (c, width) in widths.iter().enumerate() {
                let cell = row.get(c).map_or("", String::as_str);
                let pad = width.saturating_sub(cell.width());
                spans.push(Span::styled(format!("{cell}{}", " ".repeat(pad)), style));
                if c + 1 < columns {
                    spans.push(Span::styled(" │ ", theme::rule()));
                }
            }
            self.lines.push(DocLine {
                kind: Kind::Fixed,
                prefix: prefix.clone(),
                cont: prefix.clone(),
                spans,
                item: false,
            });
            if i == 0 {
                let rule = widths
                    .iter()
                    .map(|w| "─".repeat(*w))
                    .collect::<Vec<_>>()
                    .join("─┼─");
                self.lines.push(DocLine {
                    kind: Kind::Fixed,
                    prefix: prefix.clone(),
                    cont: prefix.clone(),
                    spans: vec![Span::styled(rule, theme::rule())],
                    item: false,
                });
            }
        }
        self.blank();
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Flip the task checkbox on a source line. Returns the new source.
pub fn toggle_task(source: &str, src_line: usize) -> Option<String> {
    let mut out = String::with_capacity(source.len());
    let mut changed = false;
    for (i, line) in source.split_inclusive('\n').enumerate() {
        if i == src_line && !changed {
            let swapped = ["[ ]", "[x]", "[X]"]
                .iter()
                .filter_map(|m| line.find(m).map(|at| (at, *m)))
                .min_by_key(|(at, _)| *at)
                .map(|(at, marker)| {
                    let replacement = if marker == "[ ]" { "[x]" } else { "[ ]" };
                    format!("{}{replacement}{}", &line[..at], &line[at + 3..])
                });
            if let Some(swapped) = swapped {
                out.push_str(&swapped);
                changed = true;
                continue;
            }
        }
        out.push_str(line);
    }
    changed.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &DocLine) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn tasks_and_progress() {
        let doc = Doc::parse("## Tasks\n- [x] one\n- [ ] two\n- [ ] three\n");
        let section = &doc.sections[0];
        assert_eq!(
            (section.tasks_done, section.tasks_total, section.items),
            (1, 3, 3)
        );
        assert!(matches!(
            doc.lines[2].kind,
            Kind::Task {
                checked: false,
                src_line: 2
            }
        ));
        assert_eq!(text(&doc.lines[2]), "two");
    }

    #[test]
    fn now_section_is_pinned_first() {
        let doc = Doc::parse("## Goal\nship it\n\n## Now\nfix the bug\n\n## Notes\nmore\n");
        assert!(matches!(&doc.lines[0].kind, Kind::Heading { title, .. } if title == "Now"));
        assert!(doc.sections[0].pinned);
        assert_eq!(doc.sections.len(), 3);
    }

    #[test]
    fn nested_sections_end_at_same_or_higher_level() {
        let doc = Doc::parse("# A\n## B\ntext\n## C\nmore\n# D\n");
        let a = &doc.sections[0];
        let d = &doc.sections[3];
        assert_eq!(a.end, d.line);
    }

    #[test]
    fn toggle_task_flips_only_the_target_line() {
        let source = "- [ ] a\n- [x] b [ ]\n";
        assert_eq!(toggle_task(source, 0).unwrap(), "- [x] a\n- [x] b [ ]\n");
        assert_eq!(toggle_task(source, 1).unwrap(), "- [ ] a\n- [ ] b [ ]\n");
        assert!(toggle_task("plain\n", 0).is_none());
    }

    #[test]
    fn code_and_tables_are_fixed_blocks() {
        let doc = Doc::parse("```rust\nlet x = 1;\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n");
        assert!(matches!(&doc.lines[0].kind, Kind::CodeTop { lang, .. } if lang == "rust"));
        assert!(doc.lines.iter().any(|l| l.kind == Kind::Fixed));
    }
}
