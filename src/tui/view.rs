use crate::model::TunnelStatus;
use crate::tui::app::{App, Overlay};
use crate::tui::overlays;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

const PRIMARY: Color = Color::Rgb(0x7D, 0x56, 0xF4);
const SECONDARY: Color = Color::Rgb(0xFF, 0x8C, 0x00);
const MUTED: Color = Color::Rgb(0x62, 0x62, 0x62);

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);

    draw_header(f, chunks[0], app);
    draw_table(f, chunks[1], app);
    draw_notification(f, chunks[2], app);
    draw_footer(f, chunks[3], app);

    match &app.overlay {
        Overlay::None => {}
        Overlay::Create => overlays::draw_create(f, area, app),
        Overlay::ConfirmDelete(idx) => overlays::draw_confirm_delete(f, area, app, *idx),
        Overlay::ConfirmQuit => overlays::draw_confirm_quit(f, area),
        Overlay::Logs(_) => overlays::draw_logs(f, area, app),
    }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let title = Line::from(vec![Span::styled(
        format!("Burrow v{} ~ hegde-atri", app.version),
        Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD),
    )]);
    let subtitle = Line::from(Span::styled(
        "Your cosy tunnel to Azure VMs",
        Style::default().fg(PRIMARY).add_modifier(Modifier::ITALIC),
    ));
    f.render_widget(Paragraph::new(vec![title, subtitle]), area);
}

fn status_span(status: &TunnelStatus) -> Span<'static> {
    let color = match status {
        TunnelStatus::Active => Color::Green,
        TunnelStatus::Connecting | TunnelStatus::Starting => SECONDARY,
        TunnelStatus::Error(_) => Color::Red,
        TunnelStatus::Inactive => MUTED,
    };
    Span::styled(status.label(), Style::default().fg(color))
}

fn draw_table(f: &mut Frame, area: Rect, app: &App) {
    let header = Row::new(["Name", "Local", "Remote", "Status", "Cert", "Expires"])
        .style(Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD));

    let rows: Vec<Row> = app.tunnels.iter().enumerate().map(|(i, t)| {
        let cert = t.cert_status.map(|c| c.label().to_string()).unwrap_or_else(|| "N/A".into());
        let expires = t.cert_expires_in.clone().unwrap_or_else(|| "-".into());
        let style = if i == app.cursor {
            Style::default().bg(PRIMARY).fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        Row::new(vec![
            Cell::from(t.machine.name.clone()),
            Cell::from(t.local_port.clone()),
            Cell::from(t.remote_port.clone()),
            Cell::from(Line::from(status_span(&t.status))),
            Cell::from(cert),
            Cell::from(expires),
        ]).style(style)
    }).collect();

    let widths = [
        Constraint::Percentage(30), Constraint::Length(7), Constraint::Length(7),
        Constraint::Length(16), Constraint::Length(14), Constraint::Length(10),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(PRIMARY)));
    f.render_widget(table, area);
}

fn draw_notification(f: &mut Frame, area: Rect, app: &App) {
    if let Some(n) = &app.notification {
        let p = Paragraph::new(n.as_str())
            .style(Style::default().bg(PRIMARY).fg(Color::White).add_modifier(Modifier::BOLD))
            .alignment(Alignment::Center);
        f.render_widget(p, area);
    }
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let text = if app.tunnels.is_empty() {
        "c: create • ↑/↓: navigate • q: quit"
    } else {
        "c: create • Enter: start/stop • Space: logs • r: regen cert • d: delete • ↑/↓: navigate • q: quit"
    };
    let p = Paragraph::new(text).style(Style::default().fg(MUTED)).alignment(Alignment::Center);
    f.render_widget(p, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn renders_without_panicking_and_shows_title() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let app = App::new("9.9".into(), Vec::new(),
            crate::azure::tunnel::TunnelManager::new(tx.clone()),
            crate::azure::cert::CertManager::new(tx));
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("Burrow v9.9"));
    }
}
