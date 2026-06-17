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

The enum must reproduce the Go state machine: delete/stop is only allowed when
status is `Active` / `Connecting` / `Starting` (app.go:340), and the start/stop
toggle on Enter follows app.go:449-484.

Cert constants are ported verbatim (cert.go:50-55): `CERT_LIFETIME` = 1h,
`RENEWAL_WINDOW` = 5m, `RENEWAL_RETRY_DELAY` = 30s, `CHECK_INTERVAL` = 1m. The
renewal trigger logic is preserved (cert.go:210-225): renew if expired, **or** if
within the renewal window **and** ≥30s since the last attempt. Expiry-estimate
fallbacks are kept: file-mtime + 1h when `ssh-keygen -L` parsing fails
(cert.go:113-120), and `now + 1h` when `az` output parsing fails
(cert.go:280-282,351-353).

The `bastion_subscription` config field is included in the serde struct
(no default-skip, matching the Go tag). Decision: **omit the `--subscription`
flag entirely when the value is blank** rather than passing an empty string as
Go does (tunnel.go:57) — passing an empty `--subscription` risks `az` rejecting
it. This is a small, safe fidelity deviation; noted here so it's intentional.

Path handling: `ssh_config_path` and public-key paths get explicit **tilde
expansion** — a leading `~/` is replaced with the home dir (cert.go:75-81,
305-319). The Go code uses `path[2:]` and would mishandle a bare `~`; the Rust
port hardens this to handle `~` and `~/...` both.

## Data flow

```
crossterm EventStream ─┐
background mpsc (tunnel/cert events) ─┼─► tokio::select! in app.rs ─► Action ─► app.apply(action) ─► terminal.draw(view)
tick interval (1s) ────┘
```

- **Tunnel start:** `app` calls `TunnelManager::start(id, tunnel)` → spawns `az`
  via `tokio::process` (in its own process group, see *Process lifecycle*), plus
  a task that reads stdout+stderr lines and pushes
  `TunnelEvent::{Status, Log, Exited}` into the channel. App turns those into
  `Action`s. Status scraping must reproduce the Go rules **exactly** (tunnel.go:124-155),
  parsing **both** stdout and stderr (Azure CLI writes normal output to stderr):
  - → `Active` when a line contains `"Tunnel is ready"` **or** `"connect on port"`
  - → `Connecting` when a line contains `"Opening tunnel"`
  - emit a `TunnelEvent` error when a stderr line contains (case-insensitive)
    `"error"` **or** `"failed"`.
- **Log buffer:** each tunnel keeps a **100-line ring buffer** (tunnel.go:97,118-120).
  Line prefixes are preserved: stdout lines get `"[OUT] "`, stderr lines get **no**
  prefix, process exit appends `"[ERR] Process exited: …"`, and kill failures
  append `"[WARN] …"` (tunnel.go:116,140,165,195).
- **Cert renewal:** one long-lived task with a `tokio::time::interval` (the
  existing `CheckInterval` = 1 min); renews inside the renewal window, emits
  `CertEvent`. Renewal runs `az ssh cert` off-thread so it never blocks draw.
  **Cert state stays keyed by VM name** (`HashMap<String, CertInfo>`, mirroring
  cert.go:18,61) — a `CertEvent` fans out to *every* tunnel whose machine name
  matches (app.go:195-205), so multiple tunnels to one VM share one cert entry.
  `TunnelId` keying applies to tunnels/processes, **not** to certs.
- **Tick:** a **1-second** UI tick drives the human-readable "expires in"
  countdown re-render. This is a deliberate change from Go's 1-minute tick
  (app.go:1008-1011): the countdown now refreshes smoothly. The renewal *check*
  loop stays on its own 1-minute `CheckInterval` — only the display tick changes.

### Process lifecycle & teardown (was unaddressed — primary lifecycle concern)

`tokio::process::Child` does **not** kill the OS process on drop. Without explicit
handling, quitting would orphan every `az network bastion tunnel` and leak local
ports — a regression against the Go `StopAll()`-on-exit (app.go:113, tunnel.go:286-292)
and a broken promise of the confirm-quit dialog ("All active SSH tunnels will be
terminated", app.go:874). Phase 1 must implement:

- `TunnelManager` owns the children and exposes `stop(id)` and `stop_all()`.
- `stop_all()` is called on **normal exit** and from the **panic hook** (after
  terminal restore).
- Children are spawned with `kill_on_drop(true)` as a backstop.
- **Unix kill mechanism:** `az` is a Python wrapper that may fork a child holding
  the port; killing only the immediate PID (as Go does at tunnel.go:191-198) can
  orphan it. Spawn each child in its **own process group** (`process_group(0)` /
  `setsid`) and kill the **group** (`killpg(SIGTERM/SIGKILL)`) on stop. The
  Windows kill-by-port path (netstat/taskkill) remains a `cfg`-gated stub in
  `cleanup.rs` for the later Windows port.
- **Per-tunnel task cancellation:** stopping or deleting a tunnel aborts its
  monitor task (via an abort handle / `CancellationToken`). `app.apply()` must
  **silently drop** any late `TunnelEvent` whose `TunnelId` is no longer present
  (replaces Go's channel-close idiom at tunnel.go:173-174, app.go:982-994).

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
3. **Log viewer is a frozen snapshot keyed by slice index:** snapshots once on
   open (app.go:302-305) and never refreshes (the tick rebuilds the table but not
   the logs, app.go:182-186); it also inherits bug #1's index/ID mismatch.
   **Fix (two-for-one):** key by `TunnelId` and refresh live on each tick while
   the viewer is open.
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
- `serde` + a YAML deserializer. **`serde_yaml` is archived/unmaintained** — use
  the maintained `serde_norway` (a `serde_yaml` fork), verifying it deserializes
  the existing file identically to Go's `gopkg.in/yaml.v3`.
- `regex` (cert-expiry parsing)
- `chrono` (local-time parsing matching the Go code)
- `color-eyre` (errors + panic hook)
- `nix` (or `libc`) for Unix process-group creation + `killpg` (see *Process lifecycle*).
- Home-directory lookup: replicate Go's `os.UserHomeDir()` exactly via
  `std::env::home_dir`-equivalent (the `home` crate). **Do not** use `directories`
  / XDG `config_dir()` for the config path — Go hardcodes `<home>/.config`
  regardless of `$XDG_CONFIG_HOME`, and the compat guarantee requires matching that.
- Port text fields are **hand-rolled** (decided, not optional): numeric-only
  input with backspace and a block `█` cursor (app.go:397-413,670) is trivial and
  an exact match — no `tui-input` dependency.

## Error handling

`color-eyre::Result` through `main`; library modules return typed errors.
Config-not-found / empty-machines reproduce today's friendly messages.
Subprocess and renewal failures become `Error` / `RenewalFailed` states surfaced
in the table + notification line — never panics. The notification auto-clear
(3s, used by the `r` regenerate flow — app.go:1072-1075) is preserved as a
timed `Action`.

## Testing

Priority is the pure logic that is currently untested in Go and needs zero
abstraction to test:

- cert-expiry regex parsing (`parseExpiryFromOutput`, `parseCertificateExpiry`)
- `format_duration`
- config deserialization (incl. the `bastion_subscription` and optional
  `ssh_config_path` fields, and tilde expansion)
- status-enum transitions
- cursor → `TunnelId` resolution
- the config-path resolution candidate logic

A small command-runner trait over `az` / `ssh-keygen` is **optional** and only
worth it for manager-orchestration tests; do not let it expand Phase 1 scope —
land the pure-logic tests above first. TUI rendering verified manually against
the Go version, with a couple of ratatui `TestBackend` snapshot tests where useful.

## Config compatibility

Existing `burrow.config.yaml` loads **unchanged** — same fields, same optional
positional arg path. The path-resolution candidate logic is ported literally
(main.go:79-107): positional arg overrides everything; otherwise try
`burrow.config.yaml` in CWD, then `<home>/.config/burrow.config.yaml`, picking
the first that exists and falling back to the first candidate; then canonicalize
to an absolute path. Critically, the `<home>/.config` lookup uses the **home
directory joined with `.config`**, matching Go's `os.UserHomeDir()` — **not** an
XDG `config_dir()`, which would diverge when `$XDG_CONFIG_HOME` is set. A current
user's config works on the Rust binary with no changes.

## Out of scope (Phase 2)

Major UX overhaul and new functionality, plus the Windows process-cleanup
implementation and the deferred cosmetic fixes above. Phase 2 gets its own
spec / plan / implementation cycle.
