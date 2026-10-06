//! Turn a parsed document into visual rows for a given width and fold state.

use std::collections::HashSet;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::markdown::{Doc, DocLine, Kind};
use crate::theme;

pub struct Row {
    /// Document line this row was produced from.
    pub line: usize,
    pub content: Line<'static>,
}

/// Document lines that are visible given the folded section keys.
pub fn visible_lines(doc: &Doc, folds: &HashSet<String>) -> Vec<usize> {
    let mut hidden = vec![false; doc.lines.len()];
    for section in &doc.sections {
        if !section.pinned && folds.contains(&section.key()) {
            for flag in &mut hidden[section.line + 1..section.end] {
                *flag = true;
            }
        }
    }
    // A folded section swallows the blank line before the next heading;
    // keep it so sections stay visually separated.
    for section in &doc.sections {
        if hidden[section.line] {
            continue;
        }
        if section.end > section.line + 1
            && section.end <= doc.lines.len()
            && folds.contains(&section.key())
            && doc.lines[section.end - 1].kind == Kind::Blank
        {
            hidden[section.end - 1] = false;
        }
    }
    (0..doc.lines.len()).filter(|i| !hidden[*i]).collect()
}

pub fn rows(doc: &Doc, folds: &HashSet<String>, width: usize) -> Vec<Row> {
    let mut rows = Vec::new();
    for index in visible_lines(doc, folds) {
        let line = &doc.lines[index];
        let produced = match &line.kind {
            Kind::Blank => vec![Line::default()],
            Kind::Heading { .. } => vec![heading(doc, index, folds, width)],
            Kind::Rule => vec![Line::from(vec![
                Span::raw("  "),
                Span::styled("─".repeat(width.saturating_sub(4)), theme::rule()),
            ])],
            Kind::CodeTop { lang, width: w } => vec![code_frame(line, lang, *w, width, true)],
            Kind::CodeBottom { width: w } => vec![code_frame(line, "", *w, width, false)],
            Kind::Code { text, width: w } => vec![code_line(line, text, *w, width)],
            Kind::Fixed => vec![truncate_line(
                line.prefix
                    .iter()
                    .cloned()
                    .chain(line.spans.iter().cloned())
                    .collect(),
                width,
            )],
            Kind::Text | Kind::Task { .. } => wrap(line, width),
        };
        rows.extend(produced.into_iter().map(|content| Row {
            line: index,
            content,
        }));
    }
    rows
}

fn heading(doc: &Doc, index: usize, folds: &HashSet<String>, width: usize) -> Line<'static> {
    let line = &doc.lines[index];
    let Kind::Heading { title, .. } = &line.kind else {
        unreachable!()
    };
    let Some(section) = doc.sections.iter().find(|s| s.line == index) else {
        return Line::from(line.spans.clone());
    };
    let folded = folds.contains(&section.key());

    let mut left: Vec<Span<'static>> = Vec::new();
    if section.pinned {
        left.push(Span::styled("▌", theme::pinned()));
        left.push(Span::styled(title.to_uppercase(), theme::pinned()));
    } else {
        let arrow = if folded { "▸ " } else { "▾ " };
        left.push(Span::styled(arrow, theme::dim()));
        left.extend(line.spans.iter().cloned());
        if folded && section.items > 0 {
            left.push(Span::styled(format!(" ({})", section.items), theme::dim()));
        }
    }

    let mut right: Vec<Span<'static>> = Vec::new();
    // A top-level title would only repeat its subsections' progress.
    if section.tasks_total > 0 && section.level > 1 {
        let (done, total) = (section.tasks_done, section.tasks_total);
        let fraction = format!("{done}/{total}");
        let colour = if done == total {
            theme::SPRING_GREEN
        } else {
            theme::FUJI_GRAY
        };
        right.push(Span::styled(fraction, Style::new().fg(colour)));
        let left_width: usize = left.iter().map(|s| s.content.width()).sum();
        if width >= left_width + 6 + 8 {
            let filled = (done * 5).div_ceil(total.max(1)).min(5);
            let filled = if done == 0 { 0 } else { filled.max(1) };
            right.push(Span::raw(" "));
            right.push(Span::styled(
                "━".repeat(filled),
                Style::new().fg(theme::SPRING_GREEN),
            ));
            right.push(Span::styled("─".repeat(5 - filled), theme::rule()));
        }
    }

    let left_width: usize = left.iter().map(|s| s.content.width()).sum();
    let right_width: usize = right.iter().map(|s| s.content.width()).sum();
    if right_width > 0 && left_width + right_width < width {
        left.push(Span::raw(" ".repeat(width - left_width - right_width)));
        left.extend(right);
        Line::from(left)
    } else {
        truncate_line(left, width)
    }
}

fn code_frame(
    line: &DocLine,
    lang: &str,
    code_width: usize,
    width: usize,
    top: bool,
) -> Line<'static> {
    let prefix_width: usize = line.prefix.iter().map(|s| s.content.width()).sum();
    let inner = (code_width + 2).min(width.saturating_sub(prefix_width + 2));
    let mut spans = line.prefix.clone();
    if top {
        let label = if lang.is_empty() {
            String::new()
        } else {
            format!(" {lang} ")
        };
        let label: String = label.chars().take(inner).collect();
        spans.push(Span::styled("╭", theme::rule()));
        spans.push(Span::styled(label.clone(), theme::dim()));
        spans.push(Span::styled(
            format!("{}╮", "─".repeat(inner.saturating_sub(label.width()))),
            theme::rule(),
        ));
    } else {
        spans.push(Span::styled(
            format!("╰{}╯", "─".repeat(inner)),
            theme::rule(),
        ));
    }
    Line::from(spans)
}

fn code_line(line: &DocLine, text: &str, code_width: usize, width: usize) -> Line<'static> {
    let prefix_width: usize = line.prefix.iter().map(|s| s.content.width()).sum();
    let inner = (code_width + 2).min(width.saturating_sub(prefix_width + 2));
    let room = inner.saturating_sub(2);
    let mut shown = fit(text, room);
    let pad = room.saturating_sub(shown.width());
    shown.push_str(&" ".repeat(pad));
    let mut spans = line.prefix.clone();
    spans.push(Span::styled("│ ", theme::rule()));
    spans.push(Span::styled(shown, theme::code_block()));
    spans.push(Span::styled(" │", theme::rule()));
    Line::from(spans)
}

/// Cut `text` to `width` columns, ending with `…` when shortened.
fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

fn truncate_line(spans: Vec<Span<'static>>, width: usize) -> Line<'static> {
    let mut out = Vec::new();
    let mut used = 0;
    for span in spans {
        let w = span.content.width();
        if used + w <= width {
            used += w;
            out.push(span);
        } else {
            let room = width.saturating_sub(used);
            if room > 0 {
                out.push(Span::styled(fit(&span.content, room), span.style));
            }
            break;
        }
    }
    Line::from(out)
}

/// Greedy word wrap that keeps span styles and repeats the continuation prefix.
fn wrap(line: &DocLine, width: usize) -> Vec<Line<'static>> {
    let prefix_width: usize = line.prefix.iter().map(|s| s.content.width()).sum();
    let cont_width: usize = line.cont.iter().map(|s| s.content.width()).sum();

    let mut rows: Vec<Vec<Span<'static>>> = vec![line.prefix.clone()];
    let mut used = prefix_width;
    let mut avail = width.max(prefix_width + 1);

    for span in &line.spans {
        for token in split_keep_spaces(&span.content) {
            let token_width = token.width();
            let at_row_start = used
                == if rows.len() == 1 {
                    prefix_width
                } else {
                    cont_width
                };
            if token.trim().is_empty() {
                if !at_row_start && used + token_width <= avail {
                    rows.last_mut()
                        .unwrap()
                        .push(Span::styled(token.to_owned(), span.style));
                    used += token_width;
                }
                continue;
            }
            if used + token_width > avail && !at_row_start {
                rows.push(line.cont.clone());
                used = cont_width;
                avail = width.max(cont_width + 1);
            }
            // Hard-break words longer than a whole row.
            let mut rest = token;
            while used + rest.width() > avail {
                let room = avail - used;
                let cut = take_width(rest, room.max(1));
                rows.last_mut()
                    .unwrap()
                    .push(Span::styled(rest[..cut].to_owned(), span.style));
                rest = &rest[cut..];
                rows.push(line.cont.clone());
                used = cont_width;
                avail = width.max(cont_width + 1);
            }
            if !rest.is_empty() {
                rows.last_mut()
                    .unwrap()
                    .push(Span::styled(rest.to_owned(), span.style));
                used += rest.width();
            }
        }
    }
    rows.into_iter().map(Line::from).collect()
}

fn split_keep_spaces(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut in_space = None;
    for (i, ch) in text.char_indices() {
        let space = ch == ' ';
        if in_space.is_some_and(|s| s != space) {
            tokens.push(&text[start..i]);
            start = i;
        }
        in_space = Some(space);
    }
    if start < text.len() {
        tokens.push(&text[start..]);
    }
    tokens
}

/// Byte length of the longest prefix of `text` that fits in `width` columns.
fn take_width(text: &str, width: usize) -> usize {
    let mut used = 0;
    for (i, ch) in text.char_indices() {
        let w = ch.width().unwrap_or(0);
        if used + w > width && i > 0 {
            return i;
        }
        used += w;
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn wraps_with_hanging_indent() {
        let doc = Doc::parse("- alpha beta gamma delta epsilon\n");
        let rows = rows(&doc, &HashSet::new(), 16);
        let text: Vec<String> = rows.iter().map(|r| plain(&r.content)).collect();
        assert_eq!(text[0], "  • alpha beta ");
        assert!(text[1].starts_with("    gamma"));
        assert!(text.iter().all(|t| t.width() <= 16));
    }

    #[test]
    fn folding_hides_section_body() {
        let doc = Doc::parse("## A\n- one\n- two\n\n## B\ntext\n");
        let folds: HashSet<String> = [doc.sections[0].key()].into();
        let rows = rows(&doc, &folds, 40);
        let text: Vec<String> = rows.iter().map(|r| plain(&r.content)).collect();
        assert!(text[0].starts_with("▸ A (2)"));
        assert!(!text.iter().any(|t| t.contains("one")));
        assert!(text.iter().any(|t| t.contains("▾ B")));
    }

    #[test]
    fn progress_is_right_aligned() {
        let doc = Doc::parse("## Tasks\n- [x] a\n- [ ] b\n");
        let rows = rows(&doc, &HashSet::new(), 30);
        let heading = plain(&rows[0].content);
        assert_eq!(heading.width(), 30);
        assert!(heading.contains("1/2"));
    }
}
