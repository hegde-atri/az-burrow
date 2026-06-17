use crate::model::TunnelStatus;
use crate::tui::app::{App, Overlay};
use crate::tui::overlays;
use crate::tui::theme;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(5),
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
        Overlay::Logs(id) => overlays::draw_logs(f, area, app, *id),
        Overlay::Help => overlays::draw_help(f, area),
    }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    // ASCII badger on the left, title + summary on the right.
    let cols = Layout::horizontal([Constraint::Length(8), Constraint::Min(0)]).split(area);

    let ascii = Paragraph::new(vec![
        Line::from("  ___"),
        Line::from(" (o o)"),
        Line::from(" (. .)"),
        Line::from("  \\-/ "),
    ])
    .style(theme::accent());
    f.render_widget(ascii, cols[0]);

    let title = Line::from(Span::styled(
        format!("Burrow v{} · cosy Azure tunnels", app.version),
        theme::title(),
    ));

    let active = app.tunnels.iter().filter(|t| t.status.is_running()).count();
    let visible = app.visible_indices().len();
    let summary = match &app.filter {
        Some(q) => {
            let unit = if visible == 1 { "match" } else { "matches" };
            Line::from(Span::styled(
                format!("Filter: {q} ({visible} {unit}) — Esc to clear"),
                theme::subtitle(),
            ))
        }
        None => Line::from(Span::styled(
            format!("{} tunnels · {} active", app.tunnels.len(), active),
            theme::subtitle(),
        )),
    };

    // Leading blank nudges the title to sit beside the middle of the badger.
    f.render_widget(
        Paragraph::new(vec![Line::from(""), title, summary]),
        cols[1],
    );
}

fn status_span(status: &TunnelStatus) -> Span<'static> {
    let color = match status {
        TunnelStatus::Active => Color::Green,
        TunnelStatus::Connecting | TunnelStatus::Starting => theme::SECONDARY,
        TunnelStatus::Error(_) => Color::Red,
        TunnelStatus::Inactive => theme::MUTED,
    };
    Span::styled(status.label(), Style::default().fg(color))
}

fn draw_table(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(Span::styled(" Tunnels ", theme::title()));

    if app.tunnels.is_empty() {
        let inner = block.inner(area);
        f.render_widget(block, area);
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled("No tunnels yet", theme::accent())),
            Line::from(Span::styled("press c to create one", theme::muted())),
        ])
        .alignment(Alignment::Center);
        f.render_widget(msg, inner);
        return;
    }

    let header = Row::new(["Name", "Ports", "Status", "Cert"]).style(theme::title());

    let visible = app.visible_indices();
    let rows: Vec<Row> = visible
        .iter()
        .map(|&i| {
            let t = &app.tunnels[i];
            let ports = format!("{}→{}", t.local_port, t.remote_port);
            let cert = match (t.cert_status, &t.cert_expires_in) {
                (Some(c), Some(exp)) => format!("{} {}", c.label(), exp),
                (Some(c), None) => c.label().to_string(),
                (None, _) => "N/A".into(),
            };
            Row::new(vec![
                Cell::from(t.machine.name.clone()),
                Cell::from(ports),
                Cell::from(Line::from(status_span(&t.status))),
                Cell::from(cert),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(30),
        Constraint::Length(14),
        Constraint::Length(16),
        Constraint::Min(14),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected_row())
        .highlight_symbol("● ")
        .block(block);

    app.table_state
        .select((!visible.is_empty()).then(|| app.cursor.min(visible.len() - 1)));
    f.render_stateful_widget(table, area, &mut app.table_state);
}

fn draw_notification(f: &mut Frame, area: Rect, app: &App) {
    if let Some(n) = &app.notification {
        let p = Paragraph::new(n.as_str())
            .style(theme::selected_row())
            .alignment(Alignment::Center);
        f.render_widget(p, area);
    }
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let text = if app.tunnels.is_empty() {
        "c: create • q: quit • ?: help"
    } else {
        "↵ start/stop • ␣ logs • c new • a all • / filter • d del • ? help"
    };
    let p = Paragraph::new(text)
        .style(theme::muted())
        .alignment(Alignment::Center);
    f.render_widget(p, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn renders_without_panicking_and_shows_title() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            "9.9".into(),
            Vec::new(),
            crate::azure::tunnel::TunnelManager::new(tx.clone()),
            crate::azure::cert::CertManager::new(tx),
        );
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("Burrow v9.9"));
        assert!(content.contains("No tunnels yet"));
    }

    #[test]
    fn populated_table_shows_ports_and_summary() {
        use crate::model::Machine;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            "1.0".into(),
            Vec::new(),
            crate::azure::tunnel::TunnelManager::new(tx.clone()),
            crate::azure::cert::CertManager::new(tx),
        );
        let machine = Machine {
            name: "vm-web".into(),
            resource_group: "rg".into(),
            target_resource_id: "rid".into(),
            bastion_name: "b".into(),
            bastion_resource_group: "brg".into(),
            bastion_subscription: String::new(),
            ssh_config_path: None,
        };
        app.add_tunnel_for_test(machine, "2022", "22");

        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();

        assert!(content.contains("Ports")); // merged column header
        assert!(content.contains("2022→22")); // merged port cell
        assert!(content.contains("1 tunnels · 0 active")); // summary line
        assert!(content.contains("● ")); // selection highlight symbol
    }
}
