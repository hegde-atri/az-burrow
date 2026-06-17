//! Shared "cosy" palette and style helpers for the TUI.

use ratatui::style::{Color, Modifier, Style};

pub const PRIMARY: Color = Color::Rgb(0x7D, 0x56, 0xF4); // cosy purple
pub const SECONDARY: Color = Color::Rgb(0xFF, 0x8C, 0x00); // warm orange
pub const MUTED: Color = Color::Rgb(0x6C, 0x6C, 0x6C); // dim grey
pub const DANGER: Color = Color::Rgb(0xFF, 0x6B, 0x6B); // soft red

pub fn title() -> Style {
    Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)
}
pub fn subtitle() -> Style {
    Style::default().fg(PRIMARY).add_modifier(Modifier::ITALIC)
}
pub fn accent() -> Style {
    Style::default().fg(SECONDARY).add_modifier(Modifier::BOLD)
}
pub fn muted() -> Style {
    Style::default().fg(MUTED)
}
pub fn selected_row() -> Style {
    Style::default()
        .bg(PRIMARY)
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}
pub fn border() -> Style {
    Style::default().fg(PRIMARY)
}
