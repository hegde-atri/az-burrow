# TUI UI & Keybinds Redesign

**Date:** 2026-06-17
**Status:** Approved

## Goal

Improve the terminal UI and keybindings of az-burrow. Breaking changes are
acceptable. Keep the "cosy" identity (badger mascot, purple/orange palette) while
making the interface more readable and the keys more capable and discoverable.

## Scope

In scope: `src/tui/` (`view.rs`, `overlays.rs`, `app.rs`, `mod.rs`), plus a new
`src/tui/theme.rs`. The data model (`model.rs`) and the azure layer are unchanged
except where a status/label tweak is needed for display.

Out of scope: tunnel/cert logic, config format, networking.

## Layout

Vertical regions, top to bottom:

1. **Header** (~4 lines): ASCII badger mascot on the left; on the right the title
   `Burrow v{version} · cosy Azure tunnels` and a **summary line**
   `{n} tunnels · {k} active`. When a filter is active the summary line is
   replaced by `Filter: {query} ({m} match[es]) — Esc to clear`.
2. **Table** titled `Tunnels` in the border. Scrollable via ratatui `TableState`;
   the selected row is marked with a `●` highlight symbol and a highlighted style.
   Columns collapse from 6 to **4**:
   - `Name`
   - `Ports` — rendered `{local}→{remote}`
   - `Status` — colored label (Active=green, Starting/Connecting=orange,
     Error=red, Inactive=muted)
   - `Cert` — merged emoji + expiry, e.g. `🟢 3h25m`; `🟢 valid` when no expiry is
     known; `N/A` when there is no cert status.
3. **Empty state**: when there are no tunnels, the table area renders a centered
   friendly prompt instead of an empty grid:
   `No tunnels yet — press c to create one`.
4. **Notification line**: unchanged behavior (transient, auto-clears after 3s).
5. **Footer**: short, context-aware hint that always ends with `? help`.

## Keybinds (main view)

| Key             | Action                                                       |
| --------------- | ------------------------------------------------------------ |
| `j`/`k`/`↓`/`↑` | move cursor (wrap-around at ends)                            |
| `g` / `G`       | jump to top / bottom                                         |
| `Enter`         | start/stop selected tunnel                                   |
| `Space`         | open logs for selected tunnel                                |
| `c`             | open create wizard                                           |
| `r`             | regenerate cert for selected tunnel                          |
| `d` / `Delete`  | delete selected tunnel (confirm)                             |
| `a`             | start/stop ALL — start every stopped tunnel; if all already running, stop all |
| `/`             | enter filter input mode                                      |
| `?`             | open help overlay                                            |
| `q` / `Ctrl+C`  | smart quit (see below)                                       |

**Smart quit:** `q`/`Ctrl+C` quits immediately when no tunnel is running
(`TunnelStatus::is_running()` is false for all). If any tunnel is running, show
the `ConfirmQuit` overlay so the user knows tunnels will be torn down.

**Start/stop all (`a`):** if any tunnel is not running, start all not-running
tunnels and notify `Starting all tunnels…`. Otherwise (all running), stop all and
notify `Stopping all tunnels…`.

## Overlays and modes

- **Help overlay** (`Overlay::Help`): a centered cheat-sheet listing all keybinds
  grouped by category (navigation, tunnel actions, app). Close with `?`, `Esc`,
  or `q`.
- **Filter input mode**: entered with `/`. While active, printable characters
  append to the query, `Backspace` deletes the last char, `Enter` or `Esc` exits
  input mode while keeping the filter applied. `Esc` when the query is already
  empty clears the filter entirely. The summary line reflects the live query and
  match count.
- **Confirm dialogs** (`ConfirmDelete`, `ConfirmQuit`): accept `y` to confirm and
  `n`/`q`/`Esc` to cancel.

## State changes (`App`)

Add:

- `table_state: ratatui::widgets::TableState` — drives scrolling and selection.
  Kept in sync with `cursor`.
- `filter: Option<String>` — active filter query; `None` means no filter.
- `filtering: bool` — whether filter input mode is capturing keystrokes.

**Filtered view:** a helper computes `visible_indices() -> Vec<usize>` (indices
into `tunnels`) matching the filter (case-insensitive substring of
`machine.name`); when `filter` is `None` it is all indices. The `cursor` indexes
into this visible list. `id_at_cursor`, toggle, regen, delete, and logs resolve
the real tunnel via `visible_indices()[cursor]`, so mutation logic continues to
operate on the real `tunnels` vec. After any change to the tunnel set or filter,
`cursor` is clamped to the visible range and `table_state` is synced.

## Code cleanup (in-scope)

`view.rs` and `overlays.rs` currently each redefine `PRIMARY`/`SECONDARY`/`MUTED`.
Extract a shared `src/tui/theme.rs` exposing the palette (and any derived styles
like row-highlight, muted text, error red) and refine contrast there. Both modules
import from `theme`.

## Error handling

No new failure modes. Start/stop-all reuses existing per-tunnel start/stop, which
already records `TunnelStatus::Error` on failure. Filtering and help are pure UI
state with no I/O.

## Testing

Keep existing tests green (update the render assertion if a visible label changes).
Add unit tests for:

- `visible_indices()` filtering (case-insensitive, empty query, no matches).
- cursor/id resolution through the filtered view (`id_at_cursor` after filter).
- start/stop-all branching (mixed states → start all; all running → stop all).
- smart-quit branching (no running → `Action::Quit`; running → `ConfirmQuit`).
- wrap-around navigation and `g`/`G` jumps.

## Non-goals / YAGNI

- No list+detail side panel (explicitly deferred in favor of the refined table).
- No persistent UI preferences or config for keybinds.
- No fuzzy filtering — plain case-insensitive substring is enough.
