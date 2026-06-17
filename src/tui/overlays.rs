use crate::tui::app::{App, CreateStep};
use crate::tui::theme;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

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
    let block = dialog_block("🚇 Create New SSH Tunnel", theme::PRIMARY);
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let step_no = match app.create_step { CreateStep::Machine => 1, CreateStep::LocalPort => 2, CreateStep::RemotePort => 3 };
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(format!("Step {step_no} of 3"), Style::default().fg(theme::PRIMARY).add_modifier(Modifier::BOLD))),
        Line::from(""),
    ];

    match app.create_step {
        CreateStep::Machine => {
            lines.push(Line::from(Span::styled("Select Virtual Machine:", Style::default().fg(theme::SECONDARY).add_modifier(Modifier::BOLD))));
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
            lines.push(Line::from(Span::styled("Local Port:", Style::default().fg(theme::SECONDARY).add_modifier(Modifier::BOLD))));
            lines.push(Line::from(format!("{}█", app.create_local)));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("The local port to bind (e.g., 2022, 8080)", Style::default().fg(Color::DarkGray))));
        }
        CreateStep::RemotePort => {
            lines.push(Line::from(format!("Machine: {} • Local: {}", app.machines[app.selected_machine].name, app.create_local)));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("Remote Port:", Style::default().fg(theme::SECONDARY).add_modifier(Modifier::BOLD))));
            lines.push(Line::from(format!("{}█", app.create_remote)));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("The remote port on the VM (e.g., 22, 80, 443) • Enter: create tunnel", Style::default().fg(Color::DarkGray))));
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub fn draw_confirm_delete(f: &mut Frame, area: Rect, app: &App, idx: usize) {
    let rect = centered(area, 60, 9);
    f.render_widget(Clear, rect);
    let block = dialog_block("🗑️  Confirm Delete", theme::SECONDARY);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let info = app.tunnels.get(idx)
        .map(|t| format!("{} (Local:{} → Remote:{})", t.machine.name, t.local_port, t.remote_port))
        .unwrap_or_default();
    let lines = vec![
        Line::from("Are you sure you want to delete this tunnel?"),
        Line::from(""),
        Line::from(Span::styled(info, Style::default().fg(theme::PRIMARY).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(Span::styled("Press 'y' to delete • 'q' or Esc to cancel", Style::default().fg(Color::DarkGray))),
    ];
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: false }), inner);
}

pub fn draw_confirm_quit(f: &mut Frame, area: Rect) {
    let rect = centered(area, 60, 9);
    f.render_widget(Clear, rect);
    let block = dialog_block("⚠️  Confirm Quit", theme::DANGER);
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

pub fn draw_logs(f: &mut Frame, area: Rect, app: &App, id: crate::model::TunnelId) {
    let rect = centered(area, 90, 28);
    f.render_widget(Clear, rect);
    // Identify which tunnel's logs these are (matches the Go log-viewer title).
    let info = app
        .tunnels
        .iter()
        .find(|t| t.id == id)
        .map(|t| format!("{}:{} → {} (Port {})", t.machine.name, t.remote_port, t.machine.name, t.local_port))
        .unwrap_or_else(|| "Unknown Tunnel".to_string());
    let block = dialog_block(&format!("📋 Tunnel Logs: {info}"), theme::PRIMARY);
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    // Reserve the last body row for the "Esc: close" hint.
    let body_rows = inner.height.saturating_sub(1) as usize;
    let mut lines: Vec<Line> = if app.shown_logs.is_empty() {
        vec![Line::from("No logs available yet...")]
    } else {
        let start = app.shown_logs.len().saturating_sub(body_rows);
        app.shown_logs[start..].iter().map(|l| Line::from(l.clone())).collect()
    };
    lines.push(Line::from(Span::styled("Esc: close", Style::default().fg(Color::DarkGray))));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
