//! Kanagawa palette, matching herdr's `kanagawa` theme.

use ratatui::style::{Color, Modifier, Style};

pub const FUJI_WHITE: Color = Color::Rgb(0xDC, 0xD7, 0xBA);
pub const OLD_WHITE: Color = Color::Rgb(0xC8, 0xC0, 0x93);
pub const FUJI_GRAY: Color = Color::Rgb(0x72, 0x71, 0x69);
pub const SUMI_INK_4: Color = Color::Rgb(0x2A, 0x2A, 0x37);
pub const SUMI_INK_5: Color = Color::Rgb(0x36, 0x36, 0x46);
pub const SUMI_INK_6: Color = Color::Rgb(0x54, 0x54, 0x6D);
pub const CRYSTAL_BLUE: Color = Color::Rgb(0x7E, 0x9C, 0xD8);
pub const ONI_VIOLET: Color = Color::Rgb(0x95, 0x7F, 0xB8);
pub const CARP_YELLOW: Color = Color::Rgb(0xE6, 0xC3, 0x84);
pub const SPRING_GREEN: Color = Color::Rgb(0x98, 0xBB, 0x6C);
pub const WAVE_AQUA: Color = Color::Rgb(0x7A, 0xA8, 0x9F);
pub const SAMURAI_RED: Color = Color::Rgb(0xE8, 0x24, 0x24);

pub fn text() -> Style {
    Style::new().fg(FUJI_WHITE)
}

pub fn dim() -> Style {
    Style::new().fg(FUJI_GRAY)
}

pub fn rule() -> Style {
    Style::new().fg(SUMI_INK_6)
}

pub fn heading(level: u8) -> Style {
    match level {
        1 => Style::new().fg(CRYSTAL_BLUE).add_modifier(Modifier::BOLD),
        2 => Style::new().fg(ONI_VIOLET).add_modifier(Modifier::BOLD),
        _ => Style::new().fg(OLD_WHITE).add_modifier(Modifier::BOLD),
    }
}

pub fn pinned() -> Style {
    Style::new().fg(CARP_YELLOW).add_modifier(Modifier::BOLD)
}

pub fn inline_code() -> Style {
    Style::new().fg(WAVE_AQUA).bg(SUMI_INK_4)
}

pub fn code_block() -> Style {
    Style::new().fg(WAVE_AQUA)
}

pub fn link() -> Style {
    Style::new()
        .fg(FUJI_GRAY)
        .add_modifier(Modifier::UNDERLINED)
}

pub fn quote() -> Style {
    Style::new().fg(OLD_WHITE).add_modifier(Modifier::ITALIC)
}

pub fn task_done() -> Style {
    Style::new()
        .fg(FUJI_GRAY)
        .add_modifier(Modifier::CROSSED_OUT)
}

pub fn cursor_bg() -> Color {
    SUMI_INK_5
}

/// Symbol, label and colour for a herdr agent status.
pub fn agent_status(status: &str) -> (&'static str, Color) {
    match status {
        "working" => ("●", CARP_YELLOW),
        "blocked" => ("◆", SAMURAI_RED),
        "done" => ("✓", SPRING_GREEN),
        "idle" => ("○", FUJI_GRAY),
        _ => ("·", FUJI_GRAY),
    }
}
