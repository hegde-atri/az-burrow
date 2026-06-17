use crate::model::{CertStatus, TunnelId, TunnelStatus};

/// Background events pushed from tokio tasks (tunnel monitors, cert manager)
/// into the single mpsc channel the event loop drains.
#[derive(Debug, Clone)]
pub enum BgEvent {
    /// A tunnel's status changed (parsed from az output).
    TunnelStatus { id: TunnelId, status: TunnelStatus },
    /// A new log line for a tunnel (already prefixed). The line itself is kept
    /// for completeness; the UI pulls the full buffer via `TunnelManager::logs`.
    TunnelLog {
        id: TunnelId,
        #[allow(dead_code)]
        line: String,
    },
    /// The az process for a tunnel exited (with an optional error description).
    TunnelExited { id: TunnelId, error: Option<String> },
    /// A certificate status update, keyed by VM name (fans out to matching tunnels).
    Cert {
        vm_name: String,
        status: CertStatus,
        expires_in: Option<std::time::Duration>,
    },
    /// Result of a manual cert (re)generation triggered by `r`.
    CertRegenResult {
        vm_name: String,
        ok: bool,
        message: String,
    },
}

/// High-level actions the event loop applies to `App`.
#[derive(Debug, Clone)]
pub enum Action {
    /// A 1s UI tick (refresh countdowns / live logs).
    Tick,
    /// Background event arrived. Reserved for a future channel-driven refactor.
    #[allow(dead_code)]
    Bg(BgEvent),
    /// Clear the transient notification line. Reserved; clearing is currently
    /// driven by the event loop's timer rather than an explicit action.
    #[allow(dead_code)]
    ClearNotification,
    /// Quit the program (after teardown).
    Quit,
}
