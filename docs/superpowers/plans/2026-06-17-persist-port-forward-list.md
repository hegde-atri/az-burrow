# Persisted port-forward list Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist the user's port-forward entries (machine + ports, not status) to a sibling state file and reload them as `Inactive` tunnels on startup, without auto-connecting.

**Architecture:** A new tolerant `state` module owns the YAML state file (`burrow.state.yaml`, sibling of the config). `App` holds the state path and writes the file after every add/delete. `main` loads the file at startup and builds `Inactive` tunnels for entries whose machine still exists in config.

**Tech Stack:** Rust, serde + serde_norway (YAML), color-eyre. Pre-commit hook runs clippy + tests + rustfmt on commit.

---

### Task 1: `state` module — structs, path, tolerant load, save

**Files:**
- Create: `src/state.rs`
- Modify: `src/main.rs:1-4` (add `mod state;`)
- Test: `src/state.rs` (inline `#[cfg(test)]` module)

- [ ] **Step 1: Register the module**

In `src/main.rs`, add the module declaration alongside the others (after `mod model;`):

```rust
mod azure;
mod config;
mod model;
mod state;
mod tui;
```

- [ ] **Step 2: Write `src/state.rs` with the failing tests first**

Create `src/state.rs` with the full module. Write the implementation AND tests together here (the test bodies reference the types, so they must exist to compile); the TDD loop is "write tests, watch them run, confirm green" for this pure module.

```rust
use color_eyre::eyre::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One persisted port-forward entry. Status is intentionally NOT stored —
/// reloaded tunnels always start Inactive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedTunnel {
    pub machine: String,
    pub local_port: String,
    pub remote_port: String,
}

/// The on-disk shape of `burrow.state.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedState {
    #[serde(default)]
    pub tunnels: Vec<PersistedTunnel>,
}

/// Sibling state file next to the config: same directory, `burrow.state.yaml`.
pub fn state_path(config_path: &Path) -> PathBuf {
    match config_path.parent() {
        Some(dir) => dir.join("burrow.state.yaml"),
        None => PathBuf::from("burrow.state.yaml"),
    }
}

/// Tolerant load: a missing or unparseable file yields an empty state rather
/// than an error. The state file is a cache, never critical config.
pub fn load(path: &Path) -> PersistedState {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_norway::from_str(&text).unwrap_or_default(),
        Err(_) => PersistedState::default(),
    }
}

/// Serialize and write the state file.
pub fn save(path: &Path, state: &PersistedState) -> Result<()> {
    let text = serde_norway::to_string(state).wrap_err("serializing state")?;
    std::fs::write(path, text).wrap_err("writing state file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("az-burrow-state-test-{name}.yaml"))
    }

    #[test]
    fn state_path_is_sibling_of_config() {
        let cfg = Path::new("/home/u/.config/burrow.config.yaml");
        assert_eq!(
            state_path(cfg),
            PathBuf::from("/home/u/.config/burrow.state.yaml")
        );
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = tmp("roundtrip");
        let _ = std::fs::remove_file(&path);
        let state = PersistedState {
            tunnels: vec![PersistedTunnel {
                machine: "vm1".into(),
                local_port: "1234".into(),
                remote_port: "22".into(),
            }],
        };
        save(&path, &state).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.tunnels, state.tunnels);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_loads_empty() {
        let path = tmp("does-not-exist");
        let _ = std::fs::remove_file(&path);
        assert!(load(&path).tunnels.is_empty());
    }

    #[test]
    fn corrupt_file_loads_empty() {
        let path = tmp("corrupt");
        std::fs::write(&path, "this: : is not valid: yaml: [").unwrap();
        assert!(load(&path).tunnels.is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
```

- [ ] **Step 3: Run the state tests**

Run: `cargo test --lib state::`
Expected: PASS (4 tests: `state_path_is_sibling_of_config`, `save_then_load_round_trips`, `missing_file_loads_empty`, `corrupt_file_loads_empty`).

If `cargo test --lib` complains there is no lib target, run the binary's tests instead: `cargo test state::`.

- [ ] **Step 4: Commit**

```bash
git add src/state.rs src/main.rs
git commit -m "feat(state): tolerant YAML persistence module for tunnel list"
```

---

### Task 2: Thread the state path through `App` and persist on add/delete

**Files:**
- Modify: `src/tui/app.rs` — imports, `App` struct field, `App::new` signature, `new_for_test`, `finish_create`, `remove_tunnel`, add `persist`
- Test: `src/tui/app.rs` (inline `#[cfg(test)]` module)

- [ ] **Step 1: Add the `PathBuf` import**

At the top of `src/tui/app.rs`, the existing imports include `use std::time::{Duration, Instant};`. Add below it:

```rust
use std::path::PathBuf;
```

- [ ] **Step 2: Add the `state_path` field to `App`**

In the `pub struct App { ... }` block, add the field next to the other state (e.g. after `table_state: TableState,`):

```rust
    state_path: PathBuf,
```

- [ ] **Step 3: Update `App::new` to accept prebuilt tunnels + state path and assign ids**

Replace the existing `App::new` (currently `pub fn new(version, machines, tunnel_mgr, cert_mgr)`) with this version. It takes a `tunnels: Vec<Tunnel>` (ids ignored on input) and a `state_path`, assigns sequential ids, and sets `next_id` past the last:

```rust
    pub fn new(
        version: String,
        machines: Vec<Machine>,
        tunnels: Vec<Tunnel>,
        state_path: PathBuf,
        tunnel_mgr: TunnelManager,
        cert_mgr: CertManager,
    ) -> Self {
        // Reassign ids sequentially so callers can pass tunnels with placeholder
        // ids; next_id continues past the last assigned id.
        let mut next_id = 1u64;
        let tunnels: Vec<Tunnel> = tunnels
            .into_iter()
            .map(|mut t| {
                t.id = TunnelId(next_id);
                next_id += 1;
                t
            })
            .collect();
        Self {
            version,
            machines,
            tunnels,
            cursor: 0,
            overlay: Overlay::None,
            create_step: CreateStep::Machine,
            selected_machine: 0,
            create_local: String::new(),
            create_remote: String::new(),
            notification: None,
            shown_logs: Vec::new(),
            tunnel_mgr,
            cert_mgr,
            next_id,
            should_quit: false,
            filter: None,
            filtering: false,
            table_state: TableState::default(),
            state_path,
        }
    }
```

- [ ] **Step 4: Update `new_for_test` to pass empty tunnels and a temp state path**

Replace the body of the `#[cfg(test)] pub fn new_for_test` so it matches the new signature:

```rust
    #[cfg(test)]
    pub fn new_for_test(tx: tokio::sync::mpsc::UnboundedSender<BgEvent>) -> Self {
        Self::new(
            "test".into(),
            Vec::new(),
            Vec::new(),
            std::env::temp_dir().join("az-burrow-test-state.yaml"),
            TunnelManager::new(tx.clone()),
            CertManager::new(tx),
        )
    }
```

- [ ] **Step 5: Add the `persist` helper**

Add this private method inside `impl App` (e.g. directly after `remove_tunnel`). Save errors are deliberately swallowed — a write failure must never crash or interrupt the TUI:

```rust
    /// Best-effort write of the current tunnel list to the state file.
    /// Errors are intentionally ignored — persistence must never break the UI.
    fn persist(&self) {
        let state = crate::state::PersistedState {
            tunnels: self
                .tunnels
                .iter()
                .map(|t| crate::state::PersistedTunnel {
                    machine: t.machine.name.clone(),
                    local_port: t.local_port.clone(),
                    remote_port: t.remote_port.clone(),
                })
                .collect(),
        };
        let _ = crate::state::save(&self.state_path, &state);
    }
```

- [ ] **Step 6: Call `persist()` from the two mutation points**

In `finish_create`, the last line is `self.overlay = Overlay::None;`. Add a persist call after it:

```rust
        self.overlay = Overlay::None;
        self.persist();
```

In `remove_tunnel`, the body ends with `self.clamp_cursor();`. Add a persist call after it:

```rust
        self.tunnels.remove(idx);
        self.clamp_cursor();
        self.persist();
```

- [ ] **Step 7: Write the failing integration test**

Add this test inside the `#[cfg(test)] mod tests` block in `src/tui/app.rs` (it uses the existing `mk_machine` helper defined in that module):

```rust
    #[test]
    fn finish_create_persists_entry() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new_for_test(tx);
        let path = std::env::temp_dir().join("az-burrow-test-finish-create.yaml");
        let _ = std::fs::remove_file(&path);
        app.state_path = path.clone();
        app.machines = vec![mk_machine("vm1")];
        app.selected_machine = 0;
        app.create_local = "1234".into();
        app.create_remote = "22".into();
        app.finish_create();

        let loaded = crate::state::load(&path);
        assert_eq!(loaded.tunnels.len(), 1);
        assert_eq!(loaded.tunnels[0].machine, "vm1");
        assert_eq!(loaded.tunnels[0].local_port, "1234");
        assert_eq!(loaded.tunnels[0].remote_port, "22");
        let _ = std::fs::remove_file(&path);
    }
```

- [ ] **Step 8: Run the app tests**

Run: `cargo test app::`
Expected: PASS, including the new `finish_create_persists_entry`. The existing app tests still pass because `new_for_test` was updated to the new signature.

- [ ] **Step 9: Commit**

```bash
git add src/tui/app.rs
git commit -m "feat(app): persist tunnel list on create and delete"
```

---

### Task 3: Load persisted entries at startup in `main`

**Files:**
- Modify: `src/main.rs:6-8` (imports), `src/main.rs:80-104` (startup wiring + `App::new` call)

- [ ] **Step 1: Import the tunnel model types**

In `src/main.rs` the existing import is `use crate::model::Machine;`. Replace it with:

```rust
use crate::model::{Machine, Tunnel, TunnelId, TunnelStatus};
```

- [ ] **Step 2: Build `Inactive` tunnels from the state file**

In `main`, after the `machines` vector is built and before the `let (tx, rx) = ...` line, insert the load step. Entries whose machine name is not in the current config are silently dropped:

```rust
    let state_path = state::state_path(&config_path);
    let restored = state::load(&state_path);
    let tunnels: Vec<Tunnel> = restored
        .tunnels
        .into_iter()
        .filter_map(|p| {
            machines
                .iter()
                .find(|m| m.name == p.machine)
                .map(|m| Tunnel {
                    id: TunnelId(0), // reassigned by App::new
                    machine: m.clone(),
                    local_port: p.local_port,
                    remote_port: p.remote_port,
                    status: TunnelStatus::Inactive,
                    cert_status: None,
                    cert_expires_in: None,
                })
        })
        .collect();
```

- [ ] **Step 3: Pass tunnels and state path into `App::new`**

Update the `App::new` call (currently `tui::app::App::new(VERSION.to_string(), machines, tunnel_mgr, cert_mgr)`) to the new signature:

```rust
    let mut app = tui::app::App::new(
        VERSION.to_string(),
        machines,
        tunnels,
        state_path,
        tunnel_mgr,
        cert_mgr,
    );
```

- [ ] **Step 4: Build and run the full test suite**

Run: `cargo build && cargo test`
Expected: clean build, all tests PASS.

- [ ] **Step 5: Manual smoke test**

Run the app against a config with at least one machine, create a tunnel via the `c` wizard, quit with `q`, and confirm `burrow.state.yaml` appeared next to the config:

```bash
cargo run -- <path-to-burrow.config.yaml>
# create a tunnel (c), then quit (q)
ls -l "$(dirname <path-to-burrow.config.yaml>)/burrow.state.yaml"
cat "$(dirname <path-to-burrow.config.yaml>)/burrow.state.yaml"
```

Relaunch and confirm the tunnel reappears as `Inactive` (not connected):

```bash
cargo run -- <path-to-burrow.config.yaml>
```

Expected: the previously-created entry is listed with status `Inactive` and no `az` process starts until you press Enter.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "feat(main): restore saved tunnel list on startup as Inactive"
```

---

## Notes for the implementer

- The pre-commit hook runs clippy, tests, and rustfmt. Run `cargo fmt` before committing if the hook reports formatting changes, then re-stage.
- `next_id` and id reassignment live entirely in `App::new`; `main` passes `TunnelId(0)` placeholders — do not try to assign real ids in `main`.
- Persistence is best-effort: `persist()` ignores save errors by design. Do not change this to propagate errors into the event loop.
