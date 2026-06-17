use crate::azure::cleanup::kill_process_group;
use crate::model::{Tunnel, TunnelId, TunnelStatus};
use crate::tui::action::BgEvent;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

const MAX_LOG_LINES: usize = 100;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum StatusHint {
    Active,
    Connecting,
}

/// Append to a capped ring buffer (keep the last MAX_LOG_LINES).
fn push_log(logs: &mut Vec<String>, line: String) {
    logs.push(line);
    if logs.len() > MAX_LOG_LINES {
        let excess = logs.len() - MAX_LOG_LINES;
        logs.drain(0..excess);
    }
}

/// Matches Go: "Tunnel is ready"/"connect on port" -> Active, "Opening tunnel" -> Connecting.
fn classify_status(line: &str) -> Option<StatusHint> {
    if line.contains("Tunnel is ready") || line.contains("connect on port") {
        Some(StatusHint::Active)
    } else if line.contains("Opening tunnel") {
        Some(StatusHint::Connecting)
    } else {
        None
    }
}

/// Matches Go's stderr error scrape (case-insensitive "error"/"failed").
fn is_error_line(line: &str) -> bool {
    let l = line.to_lowercase();
    l.contains("error") || l.contains("failed")
}

struct Running {
    cancel: CancellationToken,
    pid: Option<u32>,
    logs: Arc<Mutex<Vec<String>>>,
}

/// Manages live `az network bastion tunnel` processes, keyed by stable TunnelId.
pub struct TunnelManager {
    tx: UnboundedSender<BgEvent>,
    running: HashMap<TunnelId, Running>,
}

impl TunnelManager {
    pub fn new(tx: UnboundedSender<BgEvent>) -> Self {
        Self { tx, running: HashMap::new() }
    }

    #[allow(dead_code)]
    pub fn is_running(&self, id: TunnelId) -> bool {
        self.running.contains_key(&id)
    }

    pub fn logs(&self, id: TunnelId) -> Vec<String> {
        match self.running.get(&id) {
            Some(r) => r.logs.lock().unwrap().clone(),
            None => vec!["Tunnel not running".to_string()],
        }
    }

    /// Spawn the az tunnel process and its output-monitor task.
    ///
    /// # Cleanup contract
    ///
    /// The monitor task does **not** remove its own entry from `self.running` on
    /// natural process exit — only [`TunnelManager::stop`] does.  After a
    /// natural exit, [`TunnelManager::is_running`] therefore still returns
    /// `true` and a second call to `start` for the same `id` would be
    /// rejected.  The consuming `App` must call [`TunnelManager::stop(id)`]
    /// when it receives [`BgEvent::TunnelExited`] to free the slot and allow
    /// a restart.
    pub fn start(&mut self, tunnel: &Tunnel) -> color_eyre::Result<()> {
        let id = tunnel.id;
        if self.running.contains_key(&id) {
            return Err(color_eyre::eyre::eyre!("tunnel already running"));
        }

        let mut cmd = Command::new("az");
        cmd.arg("network").arg("bastion").arg("tunnel");
        // Omit --subscription when blank (spec decision).
        if !tunnel.machine.bastion_subscription.is_empty() {
            cmd.arg("--subscription").arg(&tunnel.machine.bastion_subscription);
        }
        cmd.arg("--resource-group").arg(&tunnel.machine.bastion_resource_group)
            .arg("--name").arg(&tunnel.machine.bastion_name)
            .arg("--target-resource-id").arg(&tunnel.machine.target_resource_id)
            .arg("--resource-port").arg(&tunnel.remote_port)
            .arg("--port").arg(&tunnel.local_port)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Own process group so we can kill the whole az child tree.
        #[cfg(unix)]
        {
            cmd.process_group(0);
        }

        let mut child = cmd.spawn().map_err(|e| color_eyre::eyre::eyre!("failed to start tunnel: {e}"))?;
        let pid = child.id();
        let logs = Arc::new(Mutex::new(Vec::<String>::new()));
        let cancel = CancellationToken::new();

        let _ = self.tx.send(BgEvent::TunnelStatus { id, status: TunnelStatus::Connecting });

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let tx = self.tx.clone();
        let logs_task = logs.clone();
        let cancel_task = cancel.clone();

        tokio::spawn(async move {
            let mut out_lines = stdout.map(|s| BufReader::new(s).lines());
            let mut err_lines = stderr.map(|s| BufReader::new(s).lines());

            loop {
                tokio::select! {
                    _ = cancel_task.cancelled() => break,
                    line = read_opt(&mut out_lines) => {
                        match line {
                            Some(line) => handle_line(&tx, &logs_task, id, format!("[OUT] {line}"), &line, false),
                            None => out_lines = None,
                        }
                    }
                    line = read_opt(&mut err_lines) => {
                        match line {
                            Some(line) => handle_line(&tx, &logs_task, id, line.clone(), &line, true),
                            None => err_lines = None,
                        }
                    }
                    status = child.wait() => {
                        drain_remaining(&mut out_lines, &tx, &logs_task, id, false).await;
                        drain_remaining(&mut err_lines, &tx, &logs_task, id, true).await;
                        let err = match status {
                            Ok(s) if s.success() => None,
                            Ok(s) => Some(format!("tunnel process exited: {s}")),
                            Err(e) => Some(format!("tunnel process error: {e}")),
                        };
                        if let Some(ref e) = err {
                            push_log(&mut logs_task.lock().unwrap(), format!("[ERR] Process exited: {e}"));
                        }
                        let _ = tx.send(BgEvent::TunnelExited { id, error: err });
                        break;
                    }
                }
            }
        });

        self.running.insert(id, Running { cancel, pid, logs });
        Ok(())
    }

    /// Stop a tunnel: cancel its monitor task and kill the process group.
    pub fn stop(&mut self, id: TunnelId) {
        if let Some(r) = self.running.remove(&id) {
            r.cancel.cancel();
            if let Some(pid) = r.pid {
                kill_process_group(pid);
            }
        }
    }

    /// Kill every live tunnel (called on quit and from the panic hook).
    pub fn stop_all(&mut self) {
        let ids: Vec<TunnelId> = self.running.keys().copied().collect();
        for id in ids {
            self.stop(id);
        }
    }
}

/// Drain any buffered lines remaining after the child exits, so a final
/// error line still gets logged and classified (mirrors Go draining the
/// pipes to EOF independently of cmd.Wait).
async fn drain_remaining<R: AsyncBufReadExt + Unpin>(
    lines: &mut Option<tokio::io::Lines<R>>,
    tx: &UnboundedSender<BgEvent>,
    logs: &Arc<Mutex<Vec<String>>>,
    id: TunnelId,
    is_stderr: bool,
) {
    if let Some(l) = lines {
        while let Ok(Some(line)) = l.next_line().await {
            let stored = if is_stderr { line.clone() } else { format!("[OUT] {line}") };
            handle_line(tx, logs, id, stored, &line, is_stderr);
        }
    }
}

/// Read the next line from an optional line stream, or never-resolve if absent.
async fn read_opt<R: AsyncBufReadExt + Unpin>(
    lines: &mut Option<tokio::io::Lines<R>>,
) -> Option<String> {
    match lines {
        Some(l) => match l.next_line().await {
            Ok(Some(s)) => Some(s),
            _ => None,
        },
        None => std::future::pending().await,
    }
}

fn handle_line(
    tx: &UnboundedSender<BgEvent>,
    logs: &Arc<Mutex<Vec<String>>>,
    id: TunnelId,
    stored: String,
    raw: &str,
    is_stderr: bool,
) {
    push_log(&mut logs.lock().unwrap(), stored.clone());
    let _ = tx.send(BgEvent::TunnelLog { id, line: stored });
    if let Some(hint) = classify_status(raw) {
        let status = match hint {
            StatusHint::Active => TunnelStatus::Active,
            StatusHint::Connecting => TunnelStatus::Connecting,
        };
        let _ = tx.send(BgEvent::TunnelStatus { id, status });
    }
    if is_stderr && is_error_line(raw) {
        let _ = tx.send(BgEvent::TunnelStatus { id, status: TunnelStatus::Error(raw.to_string()) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_caps_at_100() {
        let mut logs: Vec<String> = Vec::new();
        for i in 0..150 {
            push_log(&mut logs, format!("line {i}"));
        }
        assert_eq!(logs.len(), 100);
        assert_eq!(logs.first().unwrap(), "line 50");
        assert_eq!(logs.last().unwrap(), "line 149");
    }

    #[test]
    fn classifies_status_lines() {
        assert_eq!(classify_status("Tunnel is ready, connect on port 2022"), Some(StatusHint::Active));
        assert_eq!(classify_status("Opening tunnel on port 2022"), Some(StatusHint::Connecting));
        assert_eq!(classify_status("nothing interesting"), None);
    }

    #[test]
    fn detects_error_lines() {
        assert!(is_error_line("ERROR: something broke"));
        assert!(is_error_line("operation Failed"));
        assert!(!is_error_line("all good"));
    }
}
