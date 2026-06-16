# az-burrow Rust + ratatui Migration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the Go (Bubble Tea) `az-burrow` TUI to Rust (ratatui + crossterm + tokio) on the `rust-rewrite` branch, replacing the Go code at the repo root, while fixing the known correctness bugs and adding minimal polish.

**Architecture:** Action-channel design. A central `App` holds mutable state; one `tokio::select!` loop multiplexes crossterm's async `EventStream`, an mpsc channel fed by background tasks (tunnel/cert monitors), and a 1-second tick. The `az`/`ssh-keygen` subprocesses run as tokio tasks that push `BgEvent`s into the channel. Status is modeled with enums; tunnels/processes are keyed by stable `TunnelId`; cert state is keyed by VM name.

**Tech Stack:** Rust 2021, `ratatui` 0.29, `crossterm` 0.28 (event-stream), `tokio` 1 (rt-multi-thread, macros, process, time, sync), `serde` + `serde_norway`, `regex`, `chrono`, `color-eyre`, `nix` (Unix process groups), `home`.

**Spec:** `docs/superpowers/specs/2026-06-16-rust-ratatui-migration-design.md`

---

## File Structure

```
src/
  main.rs            # CLI args, config-path resolution, terminal init/teardown, run()
  config.rs          # serde structs + load() + path resolution + tilde expansion
  model.rs           # Machine, Tunnel, TunnelId, TunnelStatus, CertStatus, format_duration
  azure/
    mod.rs           # re-exports
    parse.rs         # pure cert-expiry parsers (regex) — fully unit-tested
    cleanup.rs       # platform process-kill seam (unix impl; windows cfg stub)
    tunnel.rs        # TunnelManager: spawn `az ... tunnel`, stream output → BgEvent
    cert.rs          # CertManager: renewal loop + state, ssh-keygen/az calls
  tui/
    mod.rs           # re-exports
    action.rs        # Action + BgEvent enums (the message types)
    app.rs           # App state, apply(action), the tokio::select! event loop
    view.rs          # draw(frame, &app): header + table + footer + notification
    overlays.rs      # create wizard, confirm dialogs, log viewer rendering
tests/               # integration tests where useful
Cargo.toml
flake.nix            # updated for Rust toolchain
```

**Note on Go coexistence:** The Go sources stay in place during the port so they can be referenced. The final task deletes them and updates `flake.nix`, `README.md`, and the pre-commit hooks. Until then, `go.mod`/`go.sum` remaining present is harmless — `cargo` ignores them.

---

## Task 1: Cargo project scaffold

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs` (temporary stub)
- Create: `rust-toolchain.toml`
- Modify: `.gitignore`

- [ ] **Step 1: Create `Cargo.toml`**

```toml
[package]
name = "az-burrow"
version = "0.2.0"
edition = "2021"
description = "A cosy terminal UI for managing Azure Bastion SSH tunnels"
license = "AGPL-3.0-only"

[[bin]]
name = "az-burrow"
path = "src/main.rs"

[dependencies]
ratatui = "0.29"
crossterm = { version = "0.28", features = ["event-stream"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "process", "time", "sync"] }
tokio-util = "0.7"
futures = "0.3"
serde = { version = "1", features = ["derive"] }
serde_norway = "0.9"
regex = "1"
chrono = "0.4"
color-eyre = "0.6"
home = "0.5"

[target.'cfg(unix)'.dependencies]
nix = { version = "0.29", features = ["signal", "process"] }

[dev-dependencies]
# (TestBackend ships inside ratatui; no extra dev-deps needed yet)
```

- [ ] **Step 2: Create `rust-toolchain.toml`**

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 3: Create temporary `src/main.rs` stub**

```rust
fn main() {
    println!("az-burrow rust scaffold");
}
```

- [ ] **Step 4: Append Rust ignores to `.gitignore`**

Add these lines to `.gitignore`:

```
/target
Cargo.lock
```

(Keep `Cargo.lock` ignored for now since this is a binary in flux; the final task will decide whether to commit it.)

- [ ] **Step 5: Verify it builds**

Run: `cargo build`
Expected: compiles, produces `target/debug/az-burrow`.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml rust-toolchain.toml src/main.rs .gitignore
git commit -m "build: scaffold rust cargo project"
```

---

## Task 2: Core domain types (`model.rs`)

**Files:**
- Create: `src/model.rs`
- Modify: `src/main.rs` (add `mod model;`)
- Test: inline `#[cfg(test)]` in `src/model.rs`

- [ ] **Step 1: Write the failing test for `format_duration`**

Create `src/model.rs` with only the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn format_duration_hours() {
        assert_eq!(format_duration(Duration::from_secs(3 * 3600 + 25 * 60 + 10)), "3h25m");
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
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test --lib model 2>&1 | head -30`
Expected: FAIL — `format_duration` not found / `src/model.rs` not a module.

(If main doesn't declare the module yet, add `mod model;` to `src/main.rs` first; the failure should then be "cannot find function `format_duration`".)

- [ ] **Step 3: Implement the types and `format_duration`**

Prepend to `src/model.rs` (above the test module):

```rust
use std::time::Duration;

/// Stable identity for a tunnel instance (mirrors Go's Tunnel.ID).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TunnelId(pub u64);

/// An Azure VM target loaded from config.
#[derive(Debug, Clone)]
pub struct Machine {
    pub name: String,
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
        matches!(self, TunnelStatus::Starting | TunnelStatus::Connecting | TunnelStatus::Active)
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
```

Add `mod model;` to `src/main.rs` if not already present.

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test --lib model`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/model.rs src/main.rs
git commit -m "feat: core domain types and format_duration"
```

---

## Task 3: Config structs, loading, and path resolution (`config.rs`)

**Files:**
- Create: `src/config.rs`
- Create: `tests/config_fixture.yaml`
- Modify: `src/main.rs` (add `mod config;`)
- Test: inline `#[cfg(test)]` in `src/config.rs`

- [ ] **Step 1: Write failing tests**

Create `src/config.rs` with the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
machines:
  - name: my-vm
    resource_group: MY-RG
    target_resource_id: /subscriptions/x/virtualMachines/my-vm
    bastion_name: my-bastion
    bastion_resource_group: BASTION-RG
    ssh_config_path: ~/.ssh/az_ssh_config/my-vm
  - name: bare-vm
    resource_group: RG2
    target_resource_id: /subscriptions/y/virtualMachines/bare
    bastion_name: b2
    bastion_resource_group: BRG2
"#;

    #[test]
    fn parses_machines_with_optional_fields() {
        let cfg = parse(SAMPLE).unwrap();
        assert_eq!(cfg.machines.len(), 2);
        assert_eq!(cfg.machines[0].name, "my-vm");
        assert_eq!(cfg.machines[0].ssh_config_path.as_deref(), Some("~/.ssh/az_ssh_config/my-vm"));
        // bastion_subscription defaults to empty when omitted
        assert_eq!(cfg.machines[0].bastion_subscription, "");
        // ssh_config_path absent -> None
        assert_eq!(cfg.machines[1].ssh_config_path, None);
    }

    #[test]
    fn empty_machines_is_an_error_via_validate() {
        let cfg = parse("machines: []").unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn expand_tilde_replaces_leading_tilde() {
        let home = std::path::Path::new("/home/test");
        assert_eq!(expand_tilde_with("~/.ssh/x", home), "/home/test/.ssh/x");
        assert_eq!(expand_tilde_with("~", home), "/home/test");
        assert_eq!(expand_tilde_with("/abs/path", home), "/abs/path");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib config 2>&1 | head -20`
Expected: FAIL — items not found. (Add `mod config;` to `src/main.rs` first.)

- [ ] **Step 3: Implement `config.rs`**

Prepend above the test module:

```rust
use color_eyre::eyre::{eyre, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct MachineConfig {
    pub name: String,
    pub resource_group: String,
    pub target_resource_id: String,
    pub bastion_name: String,
    pub bastion_resource_group: String,
    #[serde(default)]
    pub bastion_subscription: String,
    #[serde(default)]
    pub ssh_config_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub machines: Vec<MachineConfig>,
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.machines.is_empty() {
            return Err(eyre!("no machines defined in config file"));
        }
        Ok(())
    }
}

pub fn parse(text: &str) -> Result<Config> {
    serde_norway::from_str(text).wrap_err("failed to parse config file")
}

/// Read + parse + validate, reproducing Go's LoadOrPrompt error messages.
pub fn load(path: &Path) -> Result<Config> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(eyre!(
                "config file not found at {}\n\nPlease create a burrow.config.yaml file with your Azure VM configurations.\nSee the example in the repository for the required format",
                path.display()
            ));
        }
        Err(e) => return Err(e).wrap_err("failed to read config file"),
    };
    let cfg = parse(&text)?;
    cfg.validate()?;
    Ok(cfg)
}

/// Replicates Go main.go config-path resolution.
/// If `arg` is Some, use it. Otherwise: prefer `burrow.config.yaml` in CWD,
/// then `<home>/.config/burrow.config.yaml`, picking the first that exists;
/// fall back to the first candidate. The result is canonicalized to absolute.
pub fn resolve_config_path(arg: Option<&str>) -> Result<PathBuf> {
    let chosen: PathBuf = if let Some(a) = arg {
        PathBuf::from(a)
    } else {
        let mut candidates = vec![PathBuf::from("burrow.config.yaml")];
        if let Some(h) = home::home_dir() {
            candidates.push(h.join(".config").join("burrow.config.yaml"));
        }
        candidates
            .iter()
            .find(|c| c.exists())
            .cloned()
            .unwrap_or_else(|| candidates[0].clone())
    };
    // Go uses filepath.Abs (does not require the file to exist).
    if chosen.is_absolute() {
        Ok(chosen)
    } else {
        Ok(std::env::current_dir()
            .wrap_err("resolving config path")?
            .join(chosen))
    }
}

/// Expand a leading `~` or `~/` to the home directory. Hardened vs Go's `[2:]`.
pub fn expand_tilde(p: &str) -> String {
    match home::home_dir() {
        Some(h) => expand_tilde_with(p, &h),
        None => p.to_string(),
    }
}

fn expand_tilde_with(p: &str, home: &Path) -> String {
    if p == "~" {
        return home.to_string_lossy().into_owned();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        return home.join(rest).to_string_lossy().into_owned();
    }
    p.to_string()
}
```

Add `mod config;` to `src/main.rs`.

- [ ] **Step 4: Run to verify passes**

Run: `cargo test --lib config`
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs src/main.rs
git commit -m "feat: config parsing, loading, path resolution, tilde expansion"
```

---

## Task 4: Cert-expiry parsers (`azure/parse.rs`)

**Files:**
- Create: `src/azure/mod.rs`
- Create: `src/azure/parse.rs`
- Modify: `src/main.rs` (add `mod azure;`)
- Test: inline `#[cfg(test)]` in `src/azure/parse.rs`

- [ ] **Step 1: Create `src/azure/mod.rs`**

```rust
pub mod parse;
```

- [ ] **Step 2: Write failing tests in `src/azure/parse.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[test]
    fn parses_az_output_expiry() {
        let out = "Generated SSH certificate /tmp/x is valid until 2025-10-15 18:06:23 in local time.";
        let t = parse_expiry_from_output(out).unwrap();
        assert_eq!((t.year(), t.month(), t.day()), (2025, 10, 15));
        assert_eq!((t.hour(), t.minute(), t.second()), (18, 6, 23));
    }

    #[test]
    fn az_output_without_marker_errors() {
        assert!(parse_expiry_from_output("nothing here").is_err());
    }

    #[test]
    fn parses_ssh_keygen_validity() {
        let out = "        Valid: from 2025-10-15T17:31:23 to 2025-10-15T18:31:23\n";
        let t = parse_certificate_expiry(out).unwrap();
        assert_eq!((t.hour(), t.minute(), t.second()), (18, 31, 23));
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test --lib parse 2>&1 | head -20`
Expected: FAIL — functions not found. (Add `mod azure;` to `src/main.rs`.)

- [ ] **Step 4: Implement parsers (prepend above tests)**

```rust
use chrono::{Local, NaiveDateTime, TimeZone};
use color_eyre::eyre::{eyre, Result};
use regex::Regex;

/// Parse `az ssh cert` output: "... is valid until YYYY-MM-DD HH:MM:SS in local time".
/// Returns the expiry as a UTC-based timestamp interpreted in local tz.
pub fn parse_expiry_from_output(output: &str) -> Result<chrono::DateTime<Local>> {
    let re = Regex::new(r"is valid until (\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}) in local time").unwrap();
    let caps = re
        .captures(output)
        .ok_or_else(|| eyre!("could not parse expiry time from output"))?;
    let naive = NaiveDateTime::parse_from_str(&caps[1], "%Y-%m-%d %H:%M:%S")?;
    local_from_naive(naive)
}

/// Parse `ssh-keygen -L -f <cert>` output: "Valid: from ... to YYYY-MM-DDTHH:MM:SS".
pub fn parse_certificate_expiry(output: &str) -> Result<chrono::DateTime<Local>> {
    let re = Regex::new(r"Valid: from .+ to (\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})").unwrap();
    let caps = re
        .captures(output)
        .ok_or_else(|| eyre!("could not parse certificate expiry from ssh-keygen output"))?;
    let naive = NaiveDateTime::parse_from_str(&caps[1], "%Y-%m-%dT%H:%M:%S")?;
    local_from_naive(naive)
}

fn local_from_naive(naive: NaiveDateTime) -> Result<chrono::DateTime<Local>> {
    match Local.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) => Ok(dt),
        chrono::LocalResult::Ambiguous(dt, _) => Ok(dt),
        chrono::LocalResult::None => Err(eyre!("invalid local time")),
    }
}
```

- [ ] **Step 5: Run to verify passes**

Run: `cargo test --lib parse`
Expected: 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/azure/mod.rs src/azure/parse.rs src/main.rs
git commit -m "feat: cert-expiry parsers with unit tests"
```

---

## Task 5: Process-cleanup seam (`azure/cleanup.rs`)

**Files:**
- Create: `src/azure/cleanup.rs`
- Modify: `src/azure/mod.rs` (add `pub mod cleanup;`)

- [ ] **Step 1: Implement the Unix process-group kill + Windows stub**

`src/azure/cleanup.rs`:

```rust
//! Platform seam for killing the `az` subprocess tree.
//! `az` is a Python wrapper that can fork children holding the port, so on Unix
//! we kill the whole process group. Windows kill-by-port is a stub for the
//! later Windows port (see spec).

#[cfg(unix)]
pub fn kill_process_group(pid: u32) {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;
    // Negative pgid semantics handled by killpg; ignore errors (process may be gone).
    let pgid = Pid::from_raw(pid as i32);
    let _ = killpg(pgid, Signal::SIGTERM);
    let _ = killpg(pgid, Signal::SIGKILL);
}

#[cfg(windows)]
pub fn kill_process_group(_pid: u32) {
    // TODO(windows port): replicate Go netstat/taskkill kill-by-port.
}
```

- [ ] **Step 2: Add the module**

Add to `src/azure/mod.rs`:

```rust
pub mod cleanup;
```

- [ ] **Step 3: Verify it builds**

Run: `cargo build`
Expected: compiles (the `nix` dep resolves on Linux).

- [ ] **Step 4: Commit**

```bash
git add src/azure/cleanup.rs src/azure/mod.rs
git commit -m "feat: unix process-group kill seam"
```

---

## Task 6: Message types (`tui/action.rs`)

**Files:**
- Create: `src/tui/mod.rs`
- Create: `src/tui/action.rs`
- Modify: `src/main.rs` (add `mod tui;`)

- [ ] **Step 1: Create `src/tui/mod.rs`**

```rust
pub mod action;
pub mod app;
pub mod overlays;
pub mod view;
```

(The referenced modules are created in later tasks; create empty placeholder files now so the crate compiles: `app.rs`, `overlays.rs`, `view.rs` each containing `// filled in later`. They will be overwritten.)

- [ ] **Step 2: Implement `src/tui/action.rs`**

```rust
use crate::model::{CertStatus, TunnelId, TunnelStatus};

/// Background events pushed from tokio tasks (tunnel monitors, cert manager)
/// into the single mpsc channel the event loop drains.
#[derive(Debug, Clone)]
pub enum BgEvent {
    /// A tunnel's status changed (parsed from az output).
    TunnelStatus { id: TunnelId, status: TunnelStatus },
    /// A new log line for a tunnel (already prefixed).
    TunnelLog { id: TunnelId, line: String },
    /// The az process for a tunnel exited (with an optional error description).
    TunnelExited { id: TunnelId, error: Option<String> },
    /// A certificate status update, keyed by VM name (fans out to matching tunnels).
    Cert { vm_name: String, status: CertStatus, expires_in: Option<std::time::Duration> },
    /// Result of a manual cert (re)generation triggered by `r`.
    CertRegenResult { vm_name: String, ok: bool, message: String },
}

/// High-level actions the event loop applies to `App`.
#[derive(Debug, Clone)]
pub enum Action {
    /// A 1s UI tick (refresh countdowns / live logs).
    Tick,
    /// Background event arrived.
    Bg(BgEvent),
    /// Clear the transient notification line.
    ClearNotification,
    /// Quit the program (after teardown).
    Quit,
}
```

- [ ] **Step 3: Add `mod tui;` to `src/main.rs` and build**

Run: `cargo build`
Expected: compiles (placeholders are empty but valid).

- [ ] **Step 4: Commit**

```bash
git add src/tui/mod.rs src/tui/action.rs src/tui/app.rs src/tui/overlays.rs src/tui/view.rs src/main.rs
git commit -m "feat: action and background-event message types"
```

---

## Task 7: TunnelManager (`azure/tunnel.rs`)

**Files:**
- Create: `src/azure/tunnel.rs`
- Modify: `src/azure/mod.rs` (add `pub mod tunnel;`)
- Test: inline `#[cfg(test)]` in `src/azure/tunnel.rs`

- [ ] **Step 1: Write a failing test for the log ring buffer + line classification**

Add to `src/azure/tunnel.rs`:

```rust
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
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib tunnel 2>&1 | head -20`
Expected: FAIL — items not found. (Add `pub mod tunnel;` to `src/azure/mod.rs`.)

- [ ] **Step 3: Implement `TunnelManager` + helpers (prepend above tests)**

```rust
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
            use std::os::unix::process::CommandExt;
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

/// Read the next line from an optional line stream, or never-resolve if absent.
async fn read_opt<R: AsyncBufReadExt + Unpin>(
    lines: &mut Option<tokio::io::Lines<R>>,
) -> Option<String> {
    match lines {
        Some(l) => match l.next_line().await {
            Ok(Some(s)) => Some(s),
            _ => {
                // EOF or error: signal the caller to drop this stream.
                None
            }
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
```

> Note: `read_opt` returning `None` on EOF means the loop sets the stream to `None`; thereafter that branch parks via `pending()`. The `child.wait()` branch is what actually terminates the loop on exit. This mirrors Go's goroutines + `cmd.Wait`.

- [ ] **Step 4: Run to verify passes**

Run: `cargo test --lib tunnel`
Expected: the 3 helper tests pass.

- [ ] **Step 5: Build the whole crate**

Run: `cargo build`
Expected: compiles.

- [ ] **Step 6: Commit**

```bash
git add src/azure/tunnel.rs src/azure/mod.rs
git commit -m "feat: TunnelManager with id-keyed processes and ring-buffer logs"
```

---

## Task 8: CertManager (`azure/cert.rs`)

**Files:**
- Create: `src/azure/cert.rs`
- Modify: `src/azure/mod.rs` (add `pub mod cert;`)
- Test: inline `#[cfg(test)]` in `src/azure/cert.rs`

- [ ] **Step 1: Write failing tests for status-from-expiry**

Add to `src/azure/cert.rs`:

```rust
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
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib cert 2>&1 | head -20`
Expected: FAIL — `renewal_status` not found. (Add `pub mod cert;` to `src/azure/mod.rs`.)

- [ ] **Step 3: Implement `CertManager` (prepend above tests)**

```rust
use crate::azure::parse::{parse_certificate_expiry, parse_expiry_from_output};
use crate::config::expand_tilde;
use crate::model::CertStatus;
use crate::tui::action::BgEvent;
use chrono::{DateTime, Duration as ChronoDuration, Local};
use color_eyre::eyre::Result;
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
                    && cert.last_renewal_try.map_or(true, |t| now - t >= RENEWAL_RETRY));
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
            other => {
                let msg = match other {
                    Ok(out) => String::from_utf8_lossy(&out.stderr).to_string(),
                    Err(e) => e.to_string(),
                };
                if let Some(c) = self.certs.lock().unwrap().get_mut(&vm_name) {
                    c.status = CertStatus::RenewalFailed;
                }
                let _ = self.tx.send(BgEvent::Cert { vm_name, status: CertStatus::RenewalFailed, expires_in: None });
                let _ = msg; // surfaced via status; full message available if needed
            }
        }
    }

    /// Manual (re)generation triggered by `r`. Runs ssh-keygen if no key, then az ssh cert.
    pub async fn generate(&self, vm_name: String, ssh_config_path: String) {
        let dir = PathBuf::from(expand_tilde(&ssh_config_path));
        let public_key_path = dir.join("id_rsa.pub");
        let private_key_path = dir.join("id_rsa");
        let cert_path = dir.join("id_rsa.pub-aadcert.pub");

        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
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
```

- [ ] **Step 4: Run to verify passes**

Run: `cargo test --lib cert`
Expected: 3 tests pass.

- [ ] **Step 5: Build**

Run: `cargo build`
Expected: compiles.

- [ ] **Step 6: Commit**

```bash
git add src/azure/cert.rs src/azure/mod.rs
git commit -m "feat: CertManager with renewal loop and manual regeneration"
```

---

## Task 9: App state and the event loop (`tui/app.rs`)

**Files:**
- Overwrite: `src/tui/app.rs`
- Test: inline `#[cfg(test)]` (cursor → TunnelId resolution + stale-event drop)

- [ ] **Step 1: Write failing tests**

Put this test module at the bottom of `src/tui/app.rs`:

```rust
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
        // delete index 0; cursor 0 should now map to what was the second tunnel
        app.remove_tunnel(0);
        assert_eq!(app.tunnels.len(), 1);
        assert_ne!(app.tunnels[0].id, first_id);
        assert_eq!(app.id_at_cursor(), Some(app.tunnels[0].id));
    }

    #[test]
    fn stale_bg_event_for_unknown_id_is_ignored() {
        let mut app = app_with_two_tunnels();
        let ghost = TunnelId(99999);
        // Should not panic or mutate anything.
        app.apply_bg(crate::tui::action::BgEvent::TunnelStatus { id: ghost, status: TunnelStatus::Active });
        assert!(app.tunnels.iter().all(|t| t.status != TunnelStatus::Active));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --lib app 2>&1 | head -20`
Expected: FAIL — `App` / methods not found.

- [ ] **Step 3: Implement `App` (overwrite `src/tui/app.rs`)**

```rust
use crate::azure::cert::CertManager;
use crate::azure::tunnel::TunnelManager;
use crate::model::{CertStatus, Machine, Tunnel, TunnelId, TunnelStatus};
use crate::tui::action::{Action, BgEvent};
use crate::tui::view;
use crate::model::format_duration;
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
    // create wizard
    pub create_step: CreateStep,
    pub selected_machine: usize,
    pub create_local: String,
    pub create_remote: String,
    // notification line
    pub notification: Option<String>,
    // live logs being shown
    pub shown_logs: Vec<String>,
    // infra
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
                // If the log viewer is open for this tunnel, refresh from the manager.
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

    /// Handle a key in the main (no-overlay) view. Returns an optional Action.
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
            // schedule notification auto-clear (3s) when one is set
            if self.notification.is_some() && notif_clear_at.is_none() {
                notif_clear_at = Some(Instant::now() + Duration::from_secs(3));
            }

            let action: Option<Action> = tokio::select! {
                maybe_ev = events.next() => {
                    match maybe_ev {
                        Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => self.handle_key(key),
                        Some(Ok(Event::Resize(_, _))) => None,
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
                // refresh live logs if a viewer is open
                if let Overlay::Logs(id) = self.overlay {
                    self.shown_logs = self.tunnel_mgr.logs(id);
                }
            }
            // auto-clear notification
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
```

> Ignore the unused `KeyEvent`/`UnboundedSender` warnings if any appear only in non-test builds; `cargo clippy` is run in the final task.

- [ ] **Step 4: Run to verify passes**

Run: `cargo test --lib app`
Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/tui/app.rs
git commit -m "feat: App state and tokio event loop"
```

---

## Task 10: Rendering (`tui/view.rs` + `tui/overlays.rs`)

**Files:**
- Overwrite: `src/tui/view.rs`
- Overwrite: `src/tui/overlays.rs`
- Test: inline `#[cfg(test)]` snapshot in `src/tui/view.rs` using `TestBackend`

- [ ] **Step 1: Implement `src/tui/view.rs`**

```rust
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
        Constraint::Length(4), // header
        Constraint::Min(3),    // table
        Constraint::Length(1), // notification
        Constraint::Length(1), // footer
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
    let title = Line::from(vec![
        Span::styled(
            format!("Burrow v{} ~ hegde-atri", app.version),
            Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD),
        ),
    ]);
    let subtitle = Line::from(Span::styled(
        "Your cosy tunnel to Azure VMs",
        Style::default().fg(PRIMARY).add_modifier(Modifier::ITALIC),
    ));
    let p = Paragraph::new(vec![title, subtitle]);
    f.render_widget(p, area);
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
```

- [ ] **Step 2: Implement `src/tui/overlays.rs`**

```rust
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

fn dialog_block(title: &str, color: Color) -> Block {
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
```

- [ ] **Step 3: Run the snapshot test + build**

Run: `cargo test --lib view`
Expected: `renders_without_panicking_and_shows_title` passes.

Run: `cargo build`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add src/tui/view.rs src/tui/overlays.rs
git commit -m "feat: ratatui rendering for main view and overlays"
```

---

## Task 11: Entry point + terminal lifecycle (`main.rs`)

**Files:**
- Overwrite: `src/main.rs`

- [ ] **Step 1: Implement `src/main.rs`**

```rust
mod azure;
mod config;
mod model;
mod tui;

use crate::azure::cert::CertManager;
use crate::azure::tunnel::TunnelManager;
use crate::model::Machine;
use color_eyre::eyre::Result;
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::stdout;

const VERSION: &str = "0.2.0";

fn print_help() {
    print!(
        r#"az-burrow v{VERSION} - A cosy TUI for managing Azure Bastion SSH tunnels

Usage:
  az-burrow [config-file]
  az-burrow -h | --help
  az-burrow --version

Arguments:
  config-file    Path to YAML configuration file (default: burrow.config.yaml)

Configuration:
  Looks for a config file in this order:
    1. The path you pass as an argument
    2. ./burrow.config.yaml
    3. ~/.config/burrow.config.yaml

For more information:
  https://github.com/hegde-atri/az-burrow
"#
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(first) = args.first() {
        match first.as_str() {
            "-h" | "--help" => { print_help(); return Ok(()); }
            "--version" => { println!("Az-Burrow v{VERSION}"); return Ok(()); }
            _ => {}
        }
    }

    let config_path = config::resolve_config_path(args.first().map(|s| s.as_str()))?;
    let cfg = config::load(&config_path)?;

    let machines: Vec<Machine> = cfg.machines.into_iter().map(|m| Machine {
        name: m.name, resource_group: m.resource_group, target_resource_id: m.target_resource_id,
        bastion_name: m.bastion_name, bastion_resource_group: m.bastion_resource_group,
        bastion_subscription: m.bastion_subscription, ssh_config_path: m.ssh_config_path,
    }).collect();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let tunnel_mgr = TunnelManager::new(tx.clone());
    let cert_mgr = CertManager::new(tx.clone());

    for m in &machines {
        if let Some(p) = &m.ssh_config_path {
            if !p.is_empty() { cert_mgr.register(&m.name, p); }
        }
    }
    cert_mgr.start_monitoring();

    // Terminal setup with panic-safe restore.
    install_panic_hook();
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut app = tui::app::App::new(VERSION.to_string(), machines, tunnel_mgr, cert_mgr);
    let run_result = app.run(&mut terminal, rx).await;

    // Teardown (stop_all already called inside run() on quit).
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;

    run_result
}

/// Restore the terminal before printing a panic, so a crash never leaves a broken TTY.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
        original(info);
    }));
}
```

- [ ] **Step 2: Build**

Run: `cargo build`
Expected: compiles.

- [ ] **Step 3: Run `--help` and `--version` smoke checks**

Run: `cargo run -- --version`
Expected: prints `Az-Burrow v0.2.0`.

Run: `cargo run -- --help`
Expected: prints the help text.

- [ ] **Step 4: Run against the sample config (manual)**

Run: `cargo run -- burrow.config.yaml`
Expected: the TUI launches and shows the header, an empty tunnel table, and the footer. Press `c` to open the create wizard, `Esc` to cancel, `q` then `y` to quit. (No real `az` calls happen until you start a tunnel.) Confirm the terminal is restored cleanly on exit.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: entry point, CLI args, terminal lifecycle with panic-safe restore"
```

---

## Task 12: Manual end-to-end verification against Azure

**Files:** none (verification only)

- [ ] **Step 1: Build release binary**

Run: `cargo build --release`
Expected: `target/release/az-burrow` produced.

- [ ] **Step 2: Verify with a real config + Azure login**

Pre-req: `az login` done, valid subscription selected, a real `burrow.config.yaml`.

Run: `./target/release/az-burrow`
Verify, comparing against the Go binary's behavior:
- Create a tunnel (`c`), start it (`Enter`) → status goes `Connecting...` → `Active`.
- Open logs (`Space`) → lines stream live; close with `Esc`.
- For a machine with `ssh_config_path`, the Cert column shows status and a live-counting "Expires" value; press `r` → notification shows regenerating then success.
- Create a second tunnel, delete the FIRST one (`d`,`y`) → confirm the SECOND tunnel keeps running and its logs/stop still target the right process (the index/ID bug regression check).
- Quit (`q`,`y`) → confirm via `ps aux | grep "bastion tunnel"` that **no** `az` processes are left running.

- [ ] **Step 3: Document any deviations**

If anything diverges from the Go behavior, note it and decide fix-now vs Phase 2.

---

## Task 13: Cut over — remove Go, update tooling and docs

**Files:**
- Delete: `cmd/`, `internal/`, `go.mod`, `go.sum`
- Modify: `flake.nix`, `README.md`, pre-commit config, `.gitignore`

- [ ] **Step 1: Run clippy + fmt and fix warnings**

Run: `cargo clippy --all-targets -- -D warnings`
Fix any reported issues.

Run: `cargo fmt`

- [ ] **Step 2: Remove the Go sources**

```bash
git rm -r cmd internal go.mod go.sum
```

- [ ] **Step 3: Update `flake.nix` for Rust**

Replace the Go dev/build inputs with a Rust toolchain. Minimal devShell change:

```nix
# devShell packages: replace `go` with the rust toolchain + tools
# buildInputs = [ pkgs.cargo pkgs.rustc pkgs.rustfmt pkgs.clippy pkgs.openssl pkgs.pkg-config ];
# Package build: use pkgs.rustPlatform.buildRustPackage {
#   pname = "az-burrow"; version = "0.2.0";
#   src = ./.; cargoLock.lockFile = ./Cargo.lock;
# };
```

Apply the concrete edits to match the existing `flake.nix` structure (read it first, then swap the Go derivation for `rustPlatform.buildRustPackage` and the devShell tools). Commit `Cargo.lock` (remove it from `.gitignore`) since `buildRustPackage` needs it.

- [ ] **Step 4: Update pre-commit hooks**

Remove the `gofumpt` / `golangci-lint` / Go `tests` hooks; add `cargo fmt --check`, `cargo clippy`, and `cargo test`. Update the devenv/pre-commit config accordingly (read the existing config under `.devenv`/git-hooks first).

- [ ] **Step 5: Update `README.md`**

- Change the Technology Stack section: Rust, ratatui, tokio (instead of Go / Bubble Tea).
- Update build-from-source instructions to `cargo build --release` / `cargo install --path .`.
- Update the Go version badge / dev-setup note (Rust stable instead of Go 1.25).
- Keep the config format and usage sections unchanged (they still apply).

- [ ] **Step 6: Verify a clean build + tests from scratch**

Run: `cargo build --release && cargo test`
Expected: builds, all unit tests pass.

Run (if nix available): `nix build` — Expected: produces the binary.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "chore: remove Go sources, switch tooling and docs to Rust"
```

- [ ] **Step 8: Open a PR (when ready)**

```bash
git push -u origin rust-rewrite
gh pr create --title "Rewrite in Rust + ratatui (Phase 1)" --body "Faithful port of az-burrow to Rust/ratatui/tokio with correctness bug fixes. See docs/superpowers/specs/2026-06-16-rust-ratatui-migration-design.md"
```

---

## Self-Review (completed during authoring)

**Spec coverage:**
- Config (drop-in YAML, path resolution, `<home>/.config` not XDG) → Task 3 ✓
- Status enums + state machine → Task 2 ✓ (used in Task 9)
- Cert constants/parsers/fallbacks/machine-name keying → Tasks 4, 8 ✓
- Index/ID bug fix (TunnelManager keyed by TunnelId; cursor→id resolution) → Tasks 7, 9 ✓
- Blocking cert-regen fix (spawned task) → Task 9 `trigger_regen` ✓
- Frozen-log-snapshot fix (id-keyed + live refresh on tick/log event) → Tasks 9, 10 ✓
- Process lifecycle: kill_on_drop, process-group kill, stop_all on quit + panic hook, per-tunnel cancellation, stale-event drop → Tasks 5, 7, 9, 11 ✓
- Status-scrape rules (both streams, 4 substrings, error scrape) + 100-line ring buffer + prefixes → Task 7 ✓
- 1s UI tick vs 1m cert check → Tasks 8, 9 ✓
- Notification auto-clear (3s) → Task 9 ✓
- Hand-rolled numeric port input → Task 9 `handle_create_key` ✓
- Panic-safe terminal restore + --help/--version → Task 11 ✓
- Tests: parsers, format_duration, config, status transitions, cursor→id → Tasks 2,3,4,8,9 ✓
- Cut-over to Rust tooling/docs → Task 13 ✓

**Placeholder scan:** flake.nix/pre-commit edits in Task 13 are described against files that must be read in-task (their exact current contents aren't reproducible here); every code file has complete content.

**Type consistency:** `BgEvent`/`Action` (Task 6) used identically in Tasks 7–9; `TunnelManager::{new,start,stop,stop_all,logs,is_running}` and `CertManager::{new,register,start_monitoring,generate}` signatures match across Tasks 7/8/9/11; `App` fields/methods consistent between Tasks 9 and 10.
