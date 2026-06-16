# az-burrow — Rust + ratatui Migration (Phase 1)

**Date:** 2026-06-16
**Branch:** `rust-rewrite`
**Status:** Approved design, ready for implementation planning

## Context

az-burrow is a ~2,000-line Go TUI (Bubble Tea) for managing Azure Bastion SSH
tunnels to VMs. It spawns `az network bastion tunnel` subprocesses, auto-renews
Azure AD SSH certificates (`az ssh cert`), and presents a tunnel table with a
create wizard, confirm dialogs, and a log viewer.

This is **Phase 1 of a two-phase effort**:

- **Phase 1 (this spec):** Port to Rust + ratatui + tokio. Fold in correctness
  bug fixes and a small set of "free" polish items. Goal: a working,
  behaviorally-equivalent (or better) Linux binary on the `rust-rewrite` branch
  that replaces the Go code at the repo root.
- **Phase 2 (later, separate spec):** Major UX overhaul and new functionality.

### Decisions locked during brainstorming

- Port fidelity: faithful port **plus** obvious bug fixes **plus** light polish.
- Repo layout: new `rust-rewrite` branch; Rust replaces Go in-place at repo root.
- Architecture: tokio + ratatui + crossterm, idiomatic Rust (message-driven,
  not dogmatically Elm).
- Platforms: build & test **Linux only** now; architect the process-cleanup
  seam so Windows drops in later (the author will do the Windows port).
- Bug scope: fix all **correctness** bugs; defer purely cosmetic issues to Phase 2.

## Architecture

**Pattern: Action-channel architecture** (the de-facto idiom for async ratatui
apps). A central `App` struct holds mutable state plus an `Action` enum. One
`tokio::select!` loop multiplexes three sources:

1. crossterm's async `EventStream` (keys / resize)
2. an mpsc channel fed by background tasks (tunnel + cert events)
3. a tick interval (1s) for countdown re-render

Background work — each `az` subprocess monitor and the cert-renewal loop — runs
as a tokio task that sends messages into the mpsc channel. This maps almost
line-for-line onto the existing Bubble Tea `Msg`/`Cmd` design (channels → mpsc,
`Cmd` → spawned tasks), so the port is mechanical and the concurrency model is
preserved.

Rejected alternatives: strict Elm/TEA (fights the borrow checker, clone-heavy,
and explicitly not required); std-threads-without-tokio (tokio already chosen;
`tokio::process` streams subprocess output more cleanly).

## Module / crate layout

```
az-burrow/                 (Cargo project root, replaces Go on rust-rewrite branch)
├─ Cargo.toml
├─ src/
│  ├─ main.rs              # CLI args, config-path resolution, terminal init/teardown, run()
│  ├─ config.rs            # serde structs (MachineConfig, Config) + load()  ← reads existing burrow.config.yaml verbatim
│  ├─ model.rs             # Machine, Tunnel, TunnelId, status enums (replaces internal/types)
│  ├─ azure/
│  │  ├─ mod.rs
│  │  ├─ tunnel.rs         # TunnelManager: spawn `az network bastion tunnel`, stream output → events
│  │  ├─ cert.rs           # CertManager: renewal loop, ssh-keygen/az parsing
│  │  └─ cleanup.rs        # platform process-kill seam (unix impl now; windows stub behind cfg)
│  └─ tui/
│     ├─ mod.rs
│     ├─ app.rs            # App state + the tokio::select! event loop + Action handling
│     ├─ action.rs         # Action enum (the message type)
│     ├─ event.rs          # input/background event multiplexing
│     ├─ view.rs           # draw(frame, &app): table + header + footer
│     └─ overlays.rs       # create wizard, confirm dialogs, log viewer
└─ flake.nix               # updated for Rust toolchain (keep nix support)
```

Boundaries mirror the current Go layout (config / types / azure / tui); the
1,076-line `tui/app.go` is split into focused, in-context-readable files. The
Windows cleanup logic becomes a `cfg`-gated seam so the Linux build stays clean
and Windows slots in later.

## Key types & status modeling

Model status as **enums**, not `String` (an improvement over the Go
strings-everywhere approach that eliminates fragile string comparisons like
`"Active"` / `"Connecting..."`):

- `TunnelStatus { Inactive, Starting, Connecting, Active, Error(String) }`
- `CertStatus { Valid, ExpiringSoon, Renewing, Renewed, Expired, RenewalFailed }`
- `TunnelId(u64)` newtype for stable identity.

## Data flow

```
crossterm EventStream ─┐
background mpsc (tunnel/cert events) ─┼─► tokio::select! in app.rs ─► Action ─► app.apply(action) ─► terminal.draw(view)
tick interval (1s) ────┘
```

- **Tunnel start:** `app` calls `TunnelManager::start(id, tunnel)` → spawns `az`
  via `tokio::process`, plus a task that reads stdout+stderr lines, scrapes for
  "Tunnel is ready" / "Opening tunnel", and pushes
  `TunnelEvent::{Status, Log, Exited}` into the channel. App turns those into
  `Action`s.
- **Cert renewal:** one long-lived task with a `tokio::time::interval` (the
  existing `CheckInterval`); renews inside the renewal window, emits `CertEvent`.
  Renewal runs `az ssh cert` off-thread so it never blocks draw.
- **Tick:** drives the human-readable "expires in" countdown re-render.

## Correctness bugs fixed in Phase 1

1. **Index/ID mismatch (primary):** the Go `TunnelManager` keys its process map
   by the table **cursor index** at start time, but tunnels carry a stable `ID`.
   Deleting a tunnel mid-list shifts cursor indices out from under the map →
   stop/logs hit the wrong tunnel or none. **Fix:** `TunnelManager` keyed by
   `TunnelId` everywhere (`start` / `stop` / `logs` / `is_running`). The TUI
   resolves cursor → `TunnelId` once, then only passes the ID down.
2. **`r` cert regeneration blocks the UI thread:** runs `ssh-keygen` + `az ssh
   cert` synchronously in the update path. **Fix:** spawn as a task; UI shows
   "Regenerating…" and updates on completion via an Action.
3. **Log viewer is a frozen snapshot keyed by slice index:** **Fix:** key by
   `TunnelId`, refresh live on each tick while the viewer is open.
4. **Status string fragility:** **Fix:** resolved by the status enums above.

Cosmetic items (e.g. can't `q` straight out of the log viewer, no
duplicate-local-port warning) are **deferred to Phase 2**.

## Light polish included with the port (deliberately minimal)

- Graceful terminal restore on panic (panic hook + raw-mode teardown) so a crash
  never leaves a broken terminal.
- Real `--help` / `--version` matching today's text.

Anything larger is Phase 2.

## Dependencies (intentionally lean)

- `ratatui`, `crossterm` (with `event-stream`)
- `tokio` (`rt-multi-thread`, `macros`, `process`, `time`, `sync`)
- `serde` + `serde_yaml` (drop-in for the existing YAML)
- `regex` (cert-expiry parsing)
- `chrono` (local-time parsing matching the Go code)
- `color-eyre` (errors + panic hook)
- `directories` (the `~/.config` lookup)
- Optional `tui-input` for the port text fields, or hand-rolled to keep deps minimal.

## Error handling

`color-eyre::Result` through `main`; library modules return typed errors.
Config-not-found / empty-machines reproduce today's friendly messages.
Subprocess and renewal failures become `Error` / `RenewalFailed` states surfaced
in the table + notification line — never panics.

## Testing

Unit tests for pure logic (untested in the Go version):

- cert-expiry regex parsing (`parseExpiryFromOutput`, `parseCertificateExpiry`)
- `format_duration`
- config deserialization
- status-enum transitions
- cursor → `TunnelId` resolution

The `az` / `ssh-keygen` calls sit behind a small command-runner trait so they
can be faked in tests without Azure. TUI rendering verified manually against the
Go version, with a couple of ratatui `TestBackend` snapshot tests where useful.

## Config compatibility

Existing `burrow.config.yaml` loads **unchanged** — same fields, same
`~/.config/burrow.config.yaml` fallback, same optional positional arg path. A
current user's config works on the Rust binary with no changes.

## Out of scope (Phase 2)

Major UX overhaul and new functionality, plus the Windows process-cleanup
implementation and the deferred cosmetic fixes above. Phase 2 gets its own
spec / plan / implementation cycle.
