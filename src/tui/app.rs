use crate::azure::cert::CertManager;
use crate::azure::tunnel::TunnelManager;
use crate::model::{Machine, Tunnel, TunnelId, TunnelStatus};
use crate::model::format_duration;
use crate::tui::action::{Action, BgEvent};
use crate::tui::view;
use color_eyre::eyre::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind};
use futures::StreamExt;
use ratatui::backend::Backend;
use ratatui::Terminal;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// Which overlay (if any) is currently shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    None,
    Create,
    ConfirmDelete(usize),
    ConfirmQuit,
    Logs(TunnelId),
}

/// Step in the create-tunnel wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateStep {
    Machine,
    LocalPort,
    RemotePort,
}

pub struct App {
    pub version: String,
    pub machines: Vec<Machine>,
    pub tunnels: Vec<Tunnel>,
    pub cursor: usize,
    pub overlay: Overlay,
    pub create_step: CreateStep,
    pub selected_machine: usize,
    pub create_local: String,
    pub create_remote: String,
    pub notification: Option<String>,
    pub shown_logs: Vec<String>,
    pub tunnel_mgr: TunnelManager,
    pub cert_mgr: CertManager,
    next_id: u64,
    should_quit: bool,
}

impl App {
    pub fn new(
        version: String,
        machines: Vec<Machine>,
        tunnel_mgr: TunnelManager,
        cert_mgr: CertManager,
    ) -> Self {
        Self {
            version, machines, tunnels: Vec::new(), cursor: 0, overlay: Overlay::None,
            create_step: CreateStep::Machine, selected_machine: 0,
            create_local: String::new(), create_remote: String::new(),
            notification: None, shown_logs: Vec::new(),
            tunnel_mgr, cert_mgr, next_id: 1, should_quit: false,
        }
    }

    #[cfg(test)]
    pub fn new_for_test(tx: UnboundedSender<BgEvent>) -> Self {
        Self::new(
            "test".into(), Vec::new(),
            TunnelManager::new(tx.clone()), CertManager::new(tx),
        )
    }

    #[cfg(test)]
    pub fn add_tunnel_for_test(&mut self, machine: Machine, local: &str, remote: &str) {
        let id = TunnelId(self.next_id);
        self.next_id += 1;
        self.tunnels.push(Tunnel {
            id, machine, local_port: local.into(), remote_port: remote.into(),
            status: TunnelStatus::Inactive, cert_status: None, cert_expires_in: None,
        });
    }

    pub fn id_at_cursor(&self) -> Option<TunnelId> {
        self.tunnels.get(self.cursor).map(|t| t.id)
    }

    pub fn remove_tunnel(&mut self, idx: usize) {
        if idx >= self.tunnels.len() {
            return;
        }
        let id = self.tunnels[idx].id;
        self.tunnel_mgr.stop(id);
        self.tunnels.remove(idx);
        if self.cursor >= self.tunnels.len() && self.cursor > 0 {
            self.cursor = self.tunnels.len().saturating_sub(1);
        }
    }

    /// Apply a background event. Late events for unknown ids are dropped.
    pub fn apply_bg(&mut self, ev: BgEvent) {
        match ev {
            BgEvent::TunnelStatus { id, status } => {
                if let Some(t) = self.tunnels.iter_mut().find(|t| t.id == id) {
                    t.status = status;
                }
            }
            BgEvent::TunnelLog { id, .. } => {
                if let Overlay::Logs(open) = self.overlay {
                    if open == id {
                        self.shown_logs = self.tunnel_mgr.logs(id);
                    }
                }
            }
            BgEvent::TunnelExited { id, error } => {
                if let Some(t) = self.tunnels.iter_mut().find(|t| t.id == id) {
                    t.status = match error {
                        Some(e) => TunnelStatus::Error(e),
                        None => TunnelStatus::Inactive,
                    };
                }
                self.tunnel_mgr.stop(id);
            }
            BgEvent::Cert { vm_name, status, expires_in } => {
                for t in self.tunnels.iter_mut().filter(|t| t.machine.name == vm_name) {
                    t.cert_status = Some(status);
                    t.cert_expires_in = expires_in.map(format_duration).or(Some("expired".into()));
                }
            }
            BgEvent::CertRegenResult { vm_name, ok, message } => {
                self.notification = Some(if ok {
                    format!("✅ {message} for {vm_name}")
                } else {
                    format!("❌ {message}")
                });
            }
        }
    }

    fn start_create(&mut self) {
        if self.overlay == Overlay::None && !self.machines.is_empty() {
            self.overlay = Overlay::Create;
            self.create_step = CreateStep::Machine;
            self.selected_machine = 0;
            self.create_local.clear();
            self.create_remote.clear();
        }
    }

    fn finish_create(&mut self) {
        let id = TunnelId(self.next_id);
        self.next_id += 1;
        let machine = self.machines[self.selected_machine].clone();
        self.tunnels.push(Tunnel {
            id, machine, local_port: self.create_local.clone(),
            remote_port: self.create_remote.clone(), status: TunnelStatus::Inactive,
            cert_status: None, cert_expires_in: None,
        });
        self.overlay = Overlay::None;
    }

    fn toggle_selected(&mut self) {
        let Some(idx) = (self.cursor < self.tunnels.len()).then_some(self.cursor) else { return };
        let status = self.tunnels[idx].status.clone();
        match status {
            TunnelStatus::Inactive | TunnelStatus::Error(_) => {
                self.tunnels[idx].status = TunnelStatus::Starting;
                let tunnel = self.tunnels[idx].clone();
                if let Err(e) = self.tunnel_mgr.start(&tunnel) {
                    self.tunnels[idx].status = TunnelStatus::Error(e.to_string());
                }
            }
            TunnelStatus::Active => {
                let id = self.tunnels[idx].id;
                self.tunnel_mgr.stop(id);
                self.tunnels[idx].status = TunnelStatus::Inactive;
            }
            _ => {}
        }
    }

    fn handle_main_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char('q') => { self.overlay = Overlay::ConfirmQuit; }
            KeyCode::Char('c') => self.start_create(),
            KeyCode::Up | KeyCode::Char('k') => { self.cursor = self.cursor.saturating_sub(1); }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor + 1 < self.tunnels.len() { self.cursor += 1; }
            }
            KeyCode::Enter => self.toggle_selected(),
            KeyCode::Char(' ') => {
                if let Some(id) = self.id_at_cursor() {
                    self.shown_logs = self.tunnel_mgr.logs(id);
                    self.overlay = Overlay::Logs(id);
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if self.cursor < self.tunnels.len() {
                    self.overlay = Overlay::ConfirmDelete(self.cursor);
                }
            }
            KeyCode::Char('r') => return self.trigger_regen(),
            _ => {}
        }
        None
    }

    fn trigger_regen(&mut self) -> Option<Action> {
        let t = self.tunnels.get(self.cursor)?;
        match &t.machine.ssh_config_path {
            Some(p) if !p.is_empty() => {
                self.notification = Some(format!("🔄 Regenerating certificate for {}...", t.machine.name));
                let cert_mgr = self.cert_mgr.clone();
                let vm = t.machine.name.clone();
                let path = p.clone();
                tokio::spawn(async move { cert_mgr.generate(vm, path).await; });
            }
            _ => self.notification = Some("⚠️ No SSH config path set for this VM".into()),
        }
        None
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        match self.overlay.clone() {
            Overlay::None => return self.handle_main_key(key),
            Overlay::ConfirmQuit => match key.code {
                KeyCode::Char('y') => return Some(Action::Quit),
                KeyCode::Char('q') | KeyCode::Esc => self.overlay = Overlay::None,
                _ => {}
            },
            Overlay::ConfirmDelete(idx) => match key.code {
                KeyCode::Char('y') => { self.remove_tunnel(idx); self.overlay = Overlay::None; }
                KeyCode::Char('q') | KeyCode::Esc => self.overlay = Overlay::None,
                _ => {}
            },
            Overlay::Logs(_) => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                    self.overlay = Overlay::None;
                }
            }
            Overlay::Create => self.handle_create_key(key),
        }
        None
    }

    fn handle_create_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Esc {
            self.overlay = Overlay::None;
            return;
        }
        match self.create_step {
            CreateStep::Machine => match key.code {
                KeyCode::Up | KeyCode::Char('k') => { self.selected_machine = self.selected_machine.saturating_sub(1); }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.selected_machine + 1 < self.machines.len() { self.selected_machine += 1; }
                }
                KeyCode::Enter => self.create_step = CreateStep::LocalPort,
                _ => {}
            },
            CreateStep::LocalPort | CreateStep::RemotePort => match key.code {
                KeyCode::Char(c) if c.is_ascii_digit() => {
                    if self.create_step == CreateStep::LocalPort { self.create_local.push(c); }
                    else { self.create_remote.push(c); }
                }
                KeyCode::Backspace => {
                    if self.create_step == CreateStep::LocalPort { self.create_local.pop(); }
                    else { self.create_remote.pop(); }
                }
                KeyCode::Enter => {
                    if self.create_step == CreateStep::LocalPort && !self.create_local.is_empty() {
                        self.create_step = CreateStep::RemotePort;
                    } else if self.create_step == CreateStep::RemotePort && !self.create_remote.is_empty() {
                        self.finish_create();
                    }
                }
                _ => {}
            },
        }
    }

    /// The main async event loop.
    pub async fn run<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        mut rx: UnboundedReceiver<BgEvent>,
    ) -> Result<()> {
        let mut events = EventStream::new();
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut notif_clear_at: Option<Instant> = None;

        terminal.draw(|f| view::draw(f, self))?;

        loop {
            if self.notification.is_some() && notif_clear_at.is_none() {
                notif_clear_at = Some(Instant::now() + Duration::from_secs(3));
            }

            let action: Option<Action> = tokio::select! {
                maybe_ev = events.next() => {
                    match maybe_ev {
                        Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => self.handle_key(key),
                        _ => None,
                    }
                }
                Some(bg) = rx.recv() => { self.apply_bg(bg); None }
                _ = tick.tick() => Some(Action::Tick),
            };

            if let Some(Action::Quit) = action {
                self.should_quit = true;
            }
            if let Some(Action::Tick) = action {
                if let Overlay::Logs(id) = self.overlay {
                    self.shown_logs = self.tunnel_mgr.logs(id);
                }
            }
            if let Some(at) = notif_clear_at {
                if Instant::now() >= at {
                    self.notification = None;
                    notif_clear_at = None;
                }
            }

            terminal.draw(|f| view::draw(f, self))?;

            if self.should_quit {
                self.tunnel_mgr.stop_all();
                break;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn mk_machine(name: &str) -> Machine {
        Machine {
            name: name.into(), resource_group: "rg".into(),
            target_resource_id: "rid".into(), bastion_name: "b".into(),
            bastion_resource_group: "brg".into(), bastion_subscription: String::new(),
            ssh_config_path: None,
        }
    }

    fn app_with_two_tunnels() -> App {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new_for_test(tx);
        app.add_tunnel_for_test(mk_machine("a"), "1000", "22");
        app.add_tunnel_for_test(mk_machine("b"), "1001", "22");
        app
    }

    #[test]
    fn cursor_resolves_to_stable_id_after_delete() {
        let mut app = app_with_two_tunnels();
        let first_id = app.tunnels[0].id;
        app.remove_tunnel(0);
        assert_eq!(app.tunnels.len(), 1);
        assert_ne!(app.tunnels[0].id, first_id);
        assert_eq!(app.id_at_cursor(), Some(app.tunnels[0].id));
    }

    #[test]
    fn stale_bg_event_for_unknown_id_is_ignored() {
        let mut app = app_with_two_tunnels();
        let ghost = TunnelId(99999);
        app.apply_bg(crate::tui::action::BgEvent::TunnelStatus { id: ghost, status: TunnelStatus::Active });
        assert!(app.tunnels.iter().all(|t| t.status != TunnelStatus::Active));
    }
}
