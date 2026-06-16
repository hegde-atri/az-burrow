use crate::azure::parse::{parse_certificate_expiry, parse_expiry_from_output};
use crate::config::expand_tilde;
use crate::model::CertStatus;
use crate::tui::action::BgEvent;
use chrono::{DateTime, Duration as ChronoDuration, Local};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

const CERT_LIFETIME: ChronoDuration = ChronoDuration::hours(1);
const RENEWAL_WINDOW_MINS: i64 = 5;
const RENEWAL_RETRY: ChronoDuration = ChronoDuration::seconds(30);
const CHECK_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
struct CertInfo {
    vm_name: String,
    public_key_path: PathBuf,
    cert_path: PathBuf,
    expires_at: DateTime<Local>,
    last_renewal_try: Option<DateTime<Local>>,
    status: CertStatus,
}

/// Determine status from expiry, matching Go getRenewalStatus.
fn renewal_status(expires_at: DateTime<Local>) -> CertStatus {
    let remaining = expires_at - Local::now();
    if remaining <= ChronoDuration::zero() {
        CertStatus::Expired
    } else if remaining <= ChronoDuration::minutes(RENEWAL_WINDOW_MINS) {
        CertStatus::ExpiringSoon
    } else {
        CertStatus::Valid
    }
}

#[derive(Clone)]
pub struct CertManager {
    tx: UnboundedSender<BgEvent>,
    certs: Arc<Mutex<HashMap<String, CertInfo>>>,
}

impl CertManager {
    pub fn new(tx: UnboundedSender<BgEvent>) -> Self {
        Self { tx, certs: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Register a cert for monitoring (cert may not exist yet -> marked expired).
    pub fn register(&self, vm_name: &str, ssh_config_path: &str) {
        let dir = PathBuf::from(expand_tilde(ssh_config_path));
        let public_key_path = dir.join("id_rsa.pub");
        let cert_path = dir.join("id_rsa.pub-aadcert.pub");

        let (expires_at, status) = if cert_path.exists() {
            let exp = read_cert_expiry(&cert_path).unwrap_or_else(|| Local::now() + CERT_LIFETIME);
            (exp, renewal_status(exp))
        } else {
            (Local::now(), CertStatus::Expired)
        };

        let info = CertInfo {
            vm_name: vm_name.to_string(),
            public_key_path,
            cert_path,
            expires_at,
            last_renewal_try: None,
            status,
        };
        let expires_in = (info.expires_at - Local::now()).to_std().ok();
        self.certs.lock().unwrap().insert(vm_name.to_string(), info);
        let _ = self.tx.send(BgEvent::Cert { vm_name: vm_name.to_string(), status, expires_in });
    }

    /// Spawn the periodic check-and-renew loop.
    pub fn start_monitoring(&self) {
        let me = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(CHECK_INTERVAL);
            loop {
                ticker.tick().await;
                me.check_and_renew().await;
            }
        });
    }

    async fn check_and_renew(&self) {
        let snapshot: Vec<CertInfo> = self.certs.lock().unwrap().values().cloned().collect();
        let now = Local::now();
        for cert in snapshot {
            let new_status = renewal_status(cert.expires_at);
            if new_status != cert.status {
                if let Some(c) = self.certs.lock().unwrap().get_mut(&cert.vm_name) {
                    c.status = new_status;
                }
                let expires_in = (cert.expires_at - now).to_std().ok();
                let _ = self.tx.send(BgEvent::Cert { vm_name: cert.vm_name.clone(), status: new_status, expires_in });
            }

            let remaining = cert.expires_at - now;
            let should_renew = remaining <= ChronoDuration::zero()
                || (remaining <= ChronoDuration::minutes(RENEWAL_WINDOW_MINS)
                    && cert.last_renewal_try.is_none_or(|t| now - t >= RENEWAL_RETRY));
            if should_renew {
                self.renew(cert.vm_name.clone()).await;
            }
        }
    }

    async fn renew(&self, vm_name: String) {
        let (public_key_path, cert_path) = {
            let mut guard = self.certs.lock().unwrap();
            let Some(c) = guard.get_mut(&vm_name) else { return };
            c.last_renewal_try = Some(Local::now());
            c.status = CertStatus::Renewing;
            (c.public_key_path.clone(), c.cert_path.clone())
        };
        let _ = self.tx.send(BgEvent::Cert { vm_name: vm_name.clone(), status: CertStatus::Renewing, expires_in: None });

        let output = Command::new("az")
            .arg("ssh").arg("cert")
            .arg("--file").arg(&cert_path)
            .arg("--public-key-file").arg(&public_key_path)
            .output().await;

        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                let expires_at = parse_expiry_from_output(&text).unwrap_or_else(|_| Local::now() + CERT_LIFETIME);
                if let Some(c) = self.certs.lock().unwrap().get_mut(&vm_name) {
                    c.expires_at = expires_at;
                    c.status = CertStatus::Valid;
                }
                let expires_in = (expires_at - Local::now()).to_std().ok();
                let _ = self.tx.send(BgEvent::Cert { vm_name, status: CertStatus::Renewed, expires_in });
            }
            _ => {
                // Renewal failed (az error or non-zero exit). We surface this only as
                // the RenewalFailed status, matching the Go TUI, which likewise does not
                // display the underlying error message. A diagnostic log file is Phase 2.
                if let Some(c) = self.certs.lock().unwrap().get_mut(&vm_name) {
                    c.status = CertStatus::RenewalFailed;
                }
                let _ = self.tx.send(BgEvent::Cert { vm_name, status: CertStatus::RenewalFailed, expires_in: None });
            }
        }
    }

    /// Manual (re)generation triggered by `r`. Runs ssh-keygen if no key, then az ssh cert.
    pub async fn generate(&self, vm_name: String, ssh_config_path: String) {
        let dir = PathBuf::from(expand_tilde(&ssh_config_path));
        let public_key_path = dir.join("id_rsa.pub");
        let private_key_path = dir.join("id_rsa");
        let cert_path = dir.join("id_rsa.pub-aadcert.pub");

        if let Err(e) = std::fs::create_dir_all(&dir) {
            let _ = self.tx.send(BgEvent::CertRegenResult { vm_name, ok: false, message: format!("mkdir failed: {e}") });
            return;
        }

        if !public_key_path.exists() {
            let kg = Command::new("ssh-keygen")
                .arg("-t").arg("rsa").arg("-b").arg("4096")
                .arg("-f").arg(&private_key_path).arg("-N").arg("")
                .output().await;
            if let Ok(out) = &kg {
                if !out.status.success() {
                    let _ = self.tx.send(BgEvent::CertRegenResult { vm_name, ok: false, message: String::from_utf8_lossy(&out.stderr).to_string() });
                    return;
                }
            } else if let Err(e) = kg {
                let _ = self.tx.send(BgEvent::CertRegenResult { vm_name, ok: false, message: e.to_string() });
                return;
            }
        }

        let out = Command::new("az")
            .arg("ssh").arg("cert")
            .arg("--file").arg(&cert_path)
            .arg("--public-key-file").arg(&public_key_path)
            .output().await;

        match out {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                let expires_at = parse_expiry_from_output(&text).unwrap_or_else(|_| Local::now() + CERT_LIFETIME);
                self.certs.lock().unwrap().insert(vm_name.clone(), CertInfo {
                    vm_name: vm_name.clone(),
                    public_key_path,
                    cert_path,
                    expires_at,
                    last_renewal_try: None,
                    status: CertStatus::Valid,
                });
                let expires_in = (expires_at - Local::now()).to_std().ok();
                let _ = self.tx.send(BgEvent::Cert { vm_name: vm_name.clone(), status: CertStatus::Valid, expires_in });
                let _ = self.tx.send(BgEvent::CertRegenResult { vm_name, ok: true, message: "Certificate regenerated".into() });
            }
            other => {
                let msg = match other {
                    Ok(o) => String::from_utf8_lossy(&o.stderr).to_string(),
                    Err(e) => e.to_string(),
                };
                let _ = self.tx.send(BgEvent::CertRegenResult { vm_name, ok: false, message: msg });
            }
        }
    }
}

/// Read cert expiry via `ssh-keygen -L -f <cert>`, falling back to file mtime + 1h.
fn read_cert_expiry(cert_path: &std::path::Path) -> Option<DateTime<Local>> {
    let out = std::process::Command::new("ssh-keygen").arg("-L").arg("-f").arg(cert_path).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    if let Ok(exp) = parse_certificate_expiry(&text) {
        return Some(exp);
    }
    let meta = std::fs::metadata(cert_path).ok()?;
    let modified: DateTime<Local> = meta.modified().ok()?.into();
    Some(modified + CERT_LIFETIME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;

    #[test]
    fn status_expired_when_past() {
        let exp = chrono::Local::now() - ChronoDuration::minutes(1);
        assert_eq!(renewal_status(exp), crate::model::CertStatus::Expired);
    }

    #[test]
    fn status_expiring_within_window() {
        let exp = chrono::Local::now() + ChronoDuration::minutes(3);
        assert_eq!(renewal_status(exp), crate::model::CertStatus::ExpiringSoon);
    }

    #[test]
    fn status_valid_when_far() {
        let exp = chrono::Local::now() + ChronoDuration::minutes(50);
        assert_eq!(renewal_status(exp), crate::model::CertStatus::Valid);
    }
}
