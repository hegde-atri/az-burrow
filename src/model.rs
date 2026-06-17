use std::time::Duration;

/// Stable identity for a tunnel instance (mirrors Go's Tunnel.ID).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TunnelId(pub u64);

/// An Azure VM target loaded from config.
#[derive(Debug, Clone)]
pub struct Machine {
    pub name: String,
    /// Parsed from config for completeness; not yet used by the tunnel command.
    #[allow(dead_code)]
    pub resource_group: String,
    pub target_resource_id: String,
    pub bastion_name: String,
    pub bastion_resource_group: String,
    pub bastion_subscription: String,
    /// Optional SSH config dir, e.g. ~/.ssh/az_ssh_config/vm-name (may contain a leading ~).
    pub ssh_config_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelStatus {
    Inactive,
    Starting,
    Connecting,
    Active,
    Error(String),
}

impl TunnelStatus {
    /// Whether a stop/delete is allowed (Go gated on Active/Connecting/Starting).
    pub fn is_running(&self) -> bool {
        matches!(
            self,
            TunnelStatus::Starting | TunnelStatus::Connecting | TunnelStatus::Active
        )
    }

    /// Display label shown in the table (matches Go status strings).
    pub fn label(&self) -> String {
        match self {
            TunnelStatus::Inactive => "Inactive".into(),
            TunnelStatus::Starting => "Starting".into(),
            TunnelStatus::Connecting => "Connecting...".into(),
            TunnelStatus::Active => "Active".into(),
            TunnelStatus::Error(e) => format!("Error: {e}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertStatus {
    Valid,
    ExpiringSoon,
    Renewing,
    Renewed,
    Expired,
    RenewalFailed,
}

impl CertStatus {
    /// Emoji label matching Go's formatCertStatus.
    pub fn label(&self) -> &'static str {
        match self {
            CertStatus::Valid => "🟢 valid",
            CertStatus::ExpiringSoon => "🟡 expiring",
            CertStatus::Renewing => "🔄 renewing",
            CertStatus::Renewed => "✅ renewed",
            CertStatus::Expired => "❌ expired",
            CertStatus::RenewalFailed => "⚠️ failed",
        }
    }
}

/// A configured/active tunnel and its runtime state.
#[derive(Debug, Clone)]
pub struct Tunnel {
    pub id: TunnelId,
    pub machine: Machine,
    pub local_port: String,
    pub remote_port: String,
    pub status: TunnelStatus,
    pub cert_status: Option<CertStatus>,
    pub cert_expires_in: Option<String>,
}

/// Human-readable duration, matching Go's formatDuration:
/// >=1h -> "3h25m", >=1m -> "45m30s", else "42s".
pub fn format_duration(d: Duration) -> String {
    let total = d.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}h{minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m{seconds}s")
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn format_duration_hours() {
        assert_eq!(
            format_duration(Duration::from_secs(3 * 3600 + 25 * 60 + 10)),
            "3h25m"
        );
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(format_duration(Duration::from_secs(45 * 60 + 30)), "45m30s");
    }

    #[test]
    fn format_duration_seconds() {
        assert_eq!(format_duration(Duration::from_secs(42)), "42s");
    }
}
