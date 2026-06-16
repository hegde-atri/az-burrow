use crate::tui::app::{App, CreateStep};
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

const PRIMARY: Color = Color::Rgb(0x7D, 0x56, 0xF4);
const SECONDARY: Color = Color::Rgb(0xFF, 0x8C, 0x00);

/// Center a fixed-size rect within `area`.
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let [vert] = Layout::vertical([Constraint::Length(h)]).flex(Flex::Center).areas(area);
    let [rect] = Layout::horizontal([Constraint::Length(w)]).flex(Flex::Center).areas(vert);
    rect
}

fn dialog_block(title: &str, color: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .title(Span::styled(title.to_string(), Style::default().fg(color).add_modifier(Modifier::BOLD)))
}

pub fn draw_create(f: &mut Frame, area: Rect, app: &App) {
    let rect = centered(area, 72, 16);
    f.render_widget(Clear, rect);
    let block = dialog_block("🚇 Create New SSH Tunnel", PRIMARY);
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let step_no = match app.create_step { CreateStep::Machine => 1, CreateStep::LocalPort => 2, CreateStep::RemotePort => 3 };
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(format!("Step {step_no} of 3"), Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];

    match app.create_step {
        CreateStep::Machine => {
            lines.push(Line::from(Span::styled("Select Virtual Machine:", Style::default().fg(SECONDARY).add_modifier(Modifier::BOLD))));
            lines.push(Line::from(""));
            for (i, m) in app.machines.iter().enumerate() {
                let prefix = if i == app.selected_machine { "▶ " } else { "  " };
                lines.push(Line::from(format!("{prefix}{}", m.name)));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("↑/↓: navigate • Enter: select • Esc: cancel", Style::default().fg(Color::DarkGray))));
        }
        CreateStep::LocalPort => {
            lines.push(Line::from(format!("Machine: {}", app.machines[app.selected_machine].name)));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Local Port:", Style::default().fg(SECONDARY).add_modifier(Modifier::BOLD))));
            lines.push(Line::from(format!("{}█", app.create_local)));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("The local port to bind (e.g., 2022, 8080)", Style::default().fg(Color::DarkGray))));
        }
        CreateStep::RemotePort => {
            lines.push(Line::from(format!("Machine: {} • Local: {}", app.machines[app.selected_machine].name, app.create_local)));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Remote Port:", Style::default().fg(SECONDARY).add_modifier(Modifier::BOLD))));
            lines.push(Line::from(format!("{}█", app.create_remote)));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("The remote port on the VM (e.g., 22, 80) • Enter: create", Style::default().fg(Color::DarkGray))));
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub fn draw_confirm_delete(f: &mut Frame, area: Rect, app: &App, idx: usize) {
    let rect = centered(area, 60, 9);
    f.render_widget(Clear, rect);
    let block = dialog_block("🗑️  Confirm Delete", SECONDARY);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let info = app.tunnels.get(idx)
        .map(|t| format!("{} (Local:{} → Remote:{})", t.machine.name, t.local_port, t.remote_port))
        .unwrap_or_default();
    let lines = vec![
        Line::from("Are you sure you want to delete this tunnel?"),
        Line::from(""),
        Line::from(Span::styled(info, Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(Span::styled("Press 'y' to delete • 'q' or Esc to cancel", Style::default().fg(Color::DarkGray))),
    ];
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: false }), inner);
}

pub fn draw_confirm_quit(f: &mut Frame, area: Rect) {
    let rect = centered(area, 60, 9);
    f.render_widget(Clear, rect);
    let block = dialog_block("⚠️  Confirm Quit", Color::Rgb(0xFF, 0x6B, 0x6B));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let lines = vec![
        Line::from("All active SSH tunnels will be terminated."),
        Line::from("Are you sure you want to exit?"),
        Line::from(""),
        Line::from(Span::styled("Press 'y' to quit • 'q' or Esc to cancel", Style::default().fg(Color::DarkGray))),
    ];
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: false }), inner);
}

pub fn draw_logs(f: &mut Frame, area: Rect, app: &App) {
    let rect = centered(area, 90, 28);
    f.render_widget(Clear, rect);
    let block = dialog_block("📋 Tunnel Logs (Esc: close)", PRIMARY);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let lines: Vec<Line> = if app.shown_logs.is_empty() {
        vec![Line::from("No logs available yet...")]
    } else {
        let start = app.shown_logs.len().saturating_sub(inner.height as usize);
        app.shown_logs[start..].iter().map(|l| Line::from(l.clone())).collect()
    };
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
