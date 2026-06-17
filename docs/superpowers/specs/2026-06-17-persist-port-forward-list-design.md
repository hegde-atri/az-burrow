# Persisted port-forward list — design

## Goal

Remember the user's port-forward entries across runs. On startup the app
reloads the previously-configured list of tunnels but does **not** connect to
them — they appear as `Inactive`. The user keeps their curated list instead of
rebuilding it via the create wizard every launch.

## Behaviour

- Persisted per entry: machine name, local port, remote port. **Not** persisted:
  runtime status, cert state, filter, cursor.
- On startup, reloaded entries are `Inactive`; nothing auto-connects.
- The state file is a cache, not critical config: a missing or unparseable file
  results in an empty list (start fresh), never an error.

## Decisions

- **Save timing:** write on every add/delete (immediately after a tunnel is
  created or removed). Survives crashes / kill-on-close.
- **Stale entries:** a saved entry whose machine name is no longer present in
  `burrow.config.yaml` is silently dropped on load.
- **Location:** sibling of the resolved config file — same directory, filename
  `burrow.state.yaml`. e.g. config `~/.config/burrow.config.yaml` → state
  `~/.config/burrow.state.yaml`.

## Storage format

YAML via `serde_norway` (consistent with config):

```yaml
tunnels:
  - machine: my-vm
    local_port: "1000"
    remote_port: "22"
```

## New module: `src/state.rs`

- `PersistedTunnel { machine: String, local_port: String, remote_port: String }`
  and `PersistedState { tunnels: Vec<PersistedTunnel> }` — derive
  `Serialize + Deserialize`.
- `state_path(config_path: &Path) -> PathBuf` — derives the sibling state path.
- `load(path: &Path) -> PersistedState` — missing or unparseable ⇒ empty list
  (tolerant; no error surfaced).
- `save(path: &Path, state: &PersistedState) -> Result<()>` — serialize + write.

## App changes (`src/tui/app.rs`)

- New field `state_path: PathBuf`, threaded through `App::new`.
- `persist(&self)` helper: build a `PersistedState` from `self.tunnels`, call
  `state::save`. Save errors are swallowed — a write failure must never crash or
  interrupt the TUI.
- Call `persist()` at the end of `finish_create()` and `remove_tunnel()` (the two
  list-mutation points).
- `new_for_test` passes a throwaway/temp path so unit tests never touch real
  files.

## Startup (`src/main.rs`)

After loading machines:

1. Compute `state_path` from the resolved config path.
2. `state::load` it.
3. For each persisted entry whose `machine` name exists in the current config,
   build an `Inactive` `Tunnel`. Drop entries whose machine is absent.
4. Pass the prebuilt tunnels and `state_path` into `App::new`; initialize
   `next_id` past the highest assigned id.

## Testing

- `state.rs` unit tests: round-trip save→load; missing file ⇒ empty; corrupt
  file ⇒ empty; `state_path` derivation.
- `app.rs`: `finish_create` followed by a fresh `state::load` reproduces the
  entry (using a temp path).

## Out of scope (YAGNI)

- Persisting status / auto-reconnect.
- Persisting cert state, active filter, or cursor position.
