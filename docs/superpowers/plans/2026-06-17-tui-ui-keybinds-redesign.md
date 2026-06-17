# TUI UI & Keybinds Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refine the az-burrow TUI into a cleaner single-table layout with a 4-column tunnel list, scrolling, an empty state, a help overlay, name filtering, smart quit, start/stop-all, and nicer navigation — keeping the cosy palette.

**Architecture:** All work is in `src/tui/`. A new `theme.rs` centralizes the palette (currently duplicated across `view.rs` and `overlays.rs`). `app.rs` gains filter/selection state and the new key handling; `view.rs` renders the refined table via a `TableState`; `overlays.rs` gains a help overlay. The `cursor` becomes an index into a *filtered* view (`visible_indices()`), while all mutations still resolve to the real `tunnels` vec.

**Tech Stack:** Rust, ratatui 0.29-era widgets (`Table`, `TableState`, `Paragraph`, `Block`), crossterm key events, tokio event loop.

**Note on commits:** A stale `.pre-commit-config.yaml` (Go hooks, leftover from the Rust cutover) currently fails on commit. Use `git commit --no-verify` and run `cargo test` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt` manually instead. The flake already defines correct Rust hooks; the generated config self-corrects on next `nix develop`.

---

## File Structure

- `src/tui/theme.rs` — **new**. Shared palette constants and small style helpers.
- `src/tui/mod.rs` — modify: add `pub mod theme;`.
- `src/tui/app.rs` — modify: new state fields, filtered-view helpers, new key handling, start/stop-all, smart quit, help/filter modes.
- `src/tui/view.rs` — modify: import theme; summary line; 4-column scrollable table with `TableState`; empty state; updated footer.
- `src/tui/overlays.rs` — modify: import theme; add `draw_help`; accept `n` in confirm dialogs.

---

## Task 1: Extract a shared theme module

**Files:**
- Create: `src/tui/theme.rs`
- Modify: `src/tui/mod.rs`
- Modify: `src/tui/view.rs:10-12` (remove local color consts, import theme)
- Modify: `src/tui/overlays.rs:8-9` (remove local color consts, import theme)

- [ ] **Step 1: Create the theme module**

Create `src/tui/theme.rs`:

```rust
//! Shared "cosy" palette and style helpers for the TUI.

use ratatui::style::{Color, Modifier, Style};

pub const PRIMARY: Color = Color::Rgb(0x7D, 0x56, 0xF4); // cosy purple
pub const SECONDARY: Color = Color::Rgb(0xFF, 0x8C, 0x00); // warm orange
pub const MUTED: Color = Color::Rgb(0x6C, 0x6C, 0x6C); // dim grey
pub const DANGER: Color = Color::Rgb(0xFF, 0x6B, 0x6B); // soft red

pub fn title() -> Style {
    Style::default().fg(PRIMARY).add_modifier(Modifier::BOLD)
}
pub fn subtitle() -> Style {
    Style::default().fg(PRIMARY).add_modifier(Modifier::ITALIC)
}
pub fn accent() -> Style {
    Style::default().fg(SECONDARY).add_modifier(Modifier::BOLD)
}
pub fn muted() -> Style {
    Style::default().fg(MUTED)
}
pub fn hint() -> Style {
    Style::default().fg(Color::DarkGray)
}
pub fn selected_row() -> Style {
    Style::default()
        .bg(PRIMARY)
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}
pub fn border() -> Style {
    Style::default().fg(PRIMARY)
}
```

- [ ] **Step 2: Register the module**

In `src/tui/mod.rs`, add the module declaration alongside the existing ones:

```rust
pub mod theme;
```

- [ ] **Step 3: Switch `view.rs` to the theme**

In `src/tui/view.rs`, delete these three lines:

```rust
const PRIMARY: Color = Color::Rgb(0x7D, 0x56, 0xF4);
const SECONDARY: Color = Color::Rgb(0xFF, 0x8C, 0x00);
const MUTED: Color = Color::Rgb(0x62, 0x62, 0x62);
```

Add the import near the other `use crate::tui::...` lines:

```rust
use crate::tui::theme;
```

Then replace bare `PRIMARY`/`SECONDARY`/`MUTED` references in this file with `theme::PRIMARY`/`theme::SECONDARY`/`theme::MUTED`. (Leave `Color::Green`, `Color::Red`, `Color::White` as-is.)

- [ ] **Step 4: Switch `overlays.rs` to the theme**

In `src/tui/overlays.rs`, delete:

```rust
const PRIMARY: Color = Color::Rgb(0x7D, 0x56, 0xF4);
const SECONDARY: Color = Color::Rgb(0xFF, 0x8C, 0x00);
```

Add:

```rust
use crate::tui::theme;
```

Replace `PRIMARY`/`SECONDARY` references with `theme::PRIMARY`/`theme::SECONDARY`. Replace the inline quit-red `Color::Rgb(0xFF, 0x6B, 0x6B)` with `theme::DANGER`.

- [ ] **Step 5: Build and test (no behavior change)**

Run: `cargo build && cargo test --quiet`
Expected: builds clean; all 18 existing tests PASS.

- [ ] **Step 6: Commit**

```bash
git add src/tui/theme.rs src/tui/mod.rs src/tui/view.rs src/tui/overlays.rs
git commit --no-verify -m "refactor(tui): extract shared theme palette module"
```

---

## Task 2: Add filtered-view + selection state and helpers to App

This task adds state and pure helpers only (no key changes yet), so it is fully unit-testable.

**Files:**
- Modify: `src/tui/app.rs` (struct fields, `new`, helpers, tests)

- [ ] **Step 1: Write failing tests for the new helpers**

Add to the `tests` module at the bottom of `src/tui/app.rs`:

```rust
#[test]
fn visible_indices_no_filter_is_all() {
    let app = app_with_two_tunnels();
    assert_eq!(app.visible_indices(), vec![0, 1]);
}

#[test]
fn visible_indices_filters_by_name_case_insensitive() {
    let mut app = app_with_two_tunnels(); // tunnels named "a" and "b"
    app.filter = Some("B".into());
    assert_eq!(app.visible_indices(), vec![1]);
}

#[test]
fn selected_real_index_maps_through_filter() {
    let mut app = app_with_two_tunnels();
    app.filter = Some("b".into());
    app.cursor = 0; // first visible = real index 1
    assert_eq!(app.selected_real_index(), Some(1));
    assert_eq!(app.id_at_cursor(), Some(app.tunnels[1].id));
}

#[test]
fn clamp_cursor_keeps_within_visible() {
    let mut app = app_with_two_tunnels();
    app.filter = Some("a".into()); // 1 match
    app.cursor = 5;
    app.clamp_cursor();
    assert_eq!(app.cursor, 0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --quiet 2>&1 | tail -20`
Expected: FAIL — `visible_indices`, `selected_real_index`, `clamp_cursor`, and field `filter` do not exist.

- [ ] **Step 3: Add imports, fields, and constructor defaults**

In `src/tui/app.rs`, update the ratatui import line to bring in `TableState`:

```rust
use ratatui::widgets::TableState;
```

Add these fields to the `pub struct App { ... }` (after `should_quit` is fine; keep them `pub` for tests):

```rust
    pub filter: Option<String>,
    pub filtering: bool,
    pub table_state: TableState,
```

In `App::new`, extend the struct literal's last line to initialize them:

```rust
            tunnel_mgr, cert_mgr, next_id: 1, should_quit: false,
            filter: None, filtering: false, table_state: TableState::default(),
```

- [ ] **Step 4: Implement the helpers**

Add these methods inside `impl App` (place them near `id_at_cursor`):

```rust
    /// Indices into `tunnels` that match the active filter (all when no filter).
    pub fn visible_indices(&self) -> Vec<usize> {
        match &self.filter {
            None => (0..self.tunnels.len()).collect(),
            Some(q) => {
                let q = q.to_lowercase();
                self.tunnels
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.machine.name.to_lowercase().contains(&q))
                    .map(|(i, _)| i)
                    .collect()
            }
        }
    }

    /// Real index into `tunnels` for the row under the cursor.
    pub fn selected_real_index(&self) -> Option<usize> {
        self.visible_indices().get(self.cursor).copied()
    }

    /// Keep `cursor` inside the visible range and sync the table selection.
    pub fn clamp_cursor(&mut self) {
        let len = self.visible_indices().len();
        if self.cursor >= len {
            self.cursor = len.saturating_sub(1);
        }
        self.table_state
            .select((len > 0).then_some(self.cursor));
    }
```

Replace the existing `id_at_cursor` body so it routes through the filter:

```rust
    pub fn id_at_cursor(&self) -> Option<TunnelId> {
        self.selected_real_index().map(|i| self.tunnels[i].id)
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --quiet 2>&1 | tail -20`
Expected: PASS — all prior tests plus the 4 new ones.

- [ ] **Step 6: Commit**

```bash
git add src/tui/app.rs
git commit --no-verify -m "feat(tui): add filtered-view and table selection state"
```

---

## Task 3: Route mutations through the filtered view

Make toggle/regen/delete resolve through `selected_real_index()` and keep the cursor clamped after deletes.

**Files:**
- Modify: `src/tui/app.rs` (`remove_tunnel`, `toggle_selected`, `trigger_regen`, delete keybind)

- [ ] **Step 1: Write a failing test for delete-under-filter**

Add to the `tests` module in `src/tui/app.rs`:

```rust
#[test]
fn delete_under_filter_removes_correct_tunnel() {
    let mut app = app_with_two_tunnels(); // "a"=idx0, "b"=idx1
    app.filter = Some("b".into());
    app.cursor = 0; // visible row 0 -> real index 1 ("b")
    let real = app.selected_real_index().unwrap();
    app.remove_tunnel(real);
    assert_eq!(app.tunnels.len(), 1);
    assert_eq!(app.tunnels[0].machine.name, "a");
}
```

- [ ] **Step 2: Run test to verify it fails or is ill-defined**

Run: `cargo test delete_under_filter --quiet 2>&1 | tail -20`
Expected: At minimum compiles; may already pass for `remove_tunnel` but will guard the next change. (If it passes, that's fine — it locks behavior.)

- [ ] **Step 3: Simplify `remove_tunnel` to clamp via the helper**

Replace the existing `remove_tunnel` body in `src/tui/app.rs` with:

```rust
    pub fn remove_tunnel(&mut self, idx: usize) {
        if idx >= self.tunnels.len() {
            return;
        }
        let id = self.tunnels[idx].id;
        self.tunnel_mgr.stop(id);
        self.tunnels.remove(idx);
        self.clamp_cursor();
    }
```

- [ ] **Step 4: Route `toggle_selected` through the filter**

Replace the first line of `toggle_selected` (the `let Some(idx) = ...` guard) with:

```rust
        let Some(idx) = self.selected_real_index() else { return };
```

(The rest of the method already operates on `self.tunnels[idx]`.)

- [ ] **Step 5: Route `trigger_regen` through the filter**

In `trigger_regen`, replace:

```rust
        let t = self.tunnels.get(self.cursor)?;
```

with:

```rust
        let t = self.tunnels.get(self.selected_real_index()?)?;
```

- [ ] **Step 6: Route the delete keybind through the filter**

In `handle_main_key`, replace the `KeyCode::Char('d') | KeyCode::Delete` arm with:

```rust
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(real) = self.selected_real_index() {
                    self.overlay = Overlay::ConfirmDelete(real);
                }
            }
```

- [ ] **Step 7: Run all tests**

Run: `cargo test --quiet 2>&1 | tail -20`
Expected: PASS, including `delete_under_filter_removes_correct_tunnel` and the existing `cursor_resolves_to_stable_id_after_delete`.

- [ ] **Step 8: Commit**

```bash
git add src/tui/app.rs
git commit --no-verify -m "feat(tui): resolve tunnel actions through filtered view"
```

---

## Task 4: Wrap-around navigation + g/G jumps

**Files:**
- Modify: `src/tui/app.rs` (`handle_main_key` nav arms, tests)

- [ ] **Step 1: Write failing navigation tests**

Add to the `tests` module:

```rust
fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn down_wraps_to_top() {
    let mut app = app_with_two_tunnels();
    app.cursor = 1; // last
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.cursor, 0);
}

#[test]
fn up_wraps_to_bottom() {
    let mut app = app_with_two_tunnels();
    app.cursor = 0;
    press(&mut app, KeyCode::Char('k'));
    assert_eq!(app.cursor, 1);
}

#[test]
fn g_and_shift_g_jump_ends() {
    let mut app = app_with_two_tunnels();
    press(&mut app, KeyCode::Char('G'));
    assert_eq!(app.cursor, 1);
    press(&mut app, KeyCode::Char('g'));
    assert_eq!(app.cursor, 0);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test wraps --quiet 2>&1 | tail -20`
Expected: FAIL — `j` at the bottom currently does nothing (no wrap); `g`/`G` unhandled.

- [ ] **Step 3: Implement wrap-around and jumps**

In `handle_main_key`, replace the Up/Down arms with the following, and add `g`/`G`:

```rust
            KeyCode::Up | KeyCode::Char('k') => {
                let len = self.visible_indices().len();
                if len > 0 {
                    self.cursor = (self.cursor + len - 1) % len;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.visible_indices().len();
                if len > 0 {
                    self.cursor = (self.cursor + 1) % len;
                }
            }
            KeyCode::Char('g') => self.cursor = 0,
            KeyCode::Char('G') => {
                self.cursor = self.visible_indices().len().saturating_sub(1);
            }
```

At the end of `handle_main_key`, just before `None`, add a clamp/sync so the table selection always tracks the cursor:

```rust
        self.clamp_cursor();
        None
```

(Replace the existing trailing `None` with the two lines above.)

- [ ] **Step 4: Run tests**

Run: `cargo test --quiet 2>&1 | tail -20`
Expected: PASS, including the three nav tests.

- [ ] **Step 5: Commit**

```bash
git add src/tui/app.rs
git commit --no-verify -m "feat(tui): wrap-around navigation and g/G jumps"
```

---

## Task 5: Start/stop all (`a`)

**Files:**
- Modify: `src/tui/app.rs` (`toggle_all`, keybind, tests)

- [ ] **Step 1: Write failing tests**

Add to the `tests` module:

```rust
#[test]
fn toggle_all_starts_when_some_inactive() {
    let mut app = app_with_two_tunnels(); // both Inactive
    app.toggle_all();
    // Inactive -> Starting for every tunnel.
    assert!(app.tunnels.iter().all(|t| t.status == TunnelStatus::Starting));
    assert!(app.notification.as_deref().unwrap().contains("Starting all"));
}

#[test]
fn toggle_all_stops_when_all_running() {
    let mut app = app_with_two_tunnels();
    for t in app.tunnels.iter_mut() {
        t.status = TunnelStatus::Active;
    }
    app.toggle_all();
    assert!(app.tunnels.iter().all(|t| t.status == TunnelStatus::Inactive));
    assert!(app.notification.as_deref().unwrap().contains("Stopping all"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test toggle_all --quiet 2>&1 | tail -20`
Expected: FAIL — `toggle_all` does not exist.

- [ ] **Step 3: Implement `toggle_all`**

Add to `impl App` (near `toggle_selected`):

```rust
    /// Start every stopped tunnel, or — if all are already running — stop them all.
    fn toggle_all(&mut self) {
        if self.tunnels.is_empty() {
            return;
        }
        let any_stopped = self.tunnels.iter().any(|t| !t.status.is_running());
        if any_stopped {
            for i in 0..self.tunnels.len() {
                if !self.tunnels[i].status.is_running() {
                    self.tunnels[i].status = TunnelStatus::Starting;
                    let tunnel = self.tunnels[i].clone();
                    if let Err(e) = self.tunnel_mgr.start(&tunnel) {
                        self.tunnels[i].status = TunnelStatus::Error(e.to_string());
                    }
                }
            }
            self.notification = Some("▶ Starting all tunnels…".into());
        } else {
            for t in self.tunnels.iter_mut() {
                self.tunnel_mgr.stop(t.id);
                t.status = TunnelStatus::Inactive;
            }
            self.notification = Some("■ Stopping all tunnels…".into());
        }
    }
```

- [ ] **Step 4: Wire the keybind**

In `handle_main_key`, add an arm (before the `_ => {}`):

```rust
            KeyCode::Char('a') => self.toggle_all(),
```

- [ ] **Step 5: Run tests**

Run: `cargo test --quiet 2>&1 | tail -20`
Expected: PASS, including both `toggle_all` tests.

- [ ] **Step 6: Commit**

```bash
git add src/tui/app.rs
git commit --no-verify -m "feat(tui): start/stop all tunnels with 'a'"
```

---

## Task 6: Smart quit

**Files:**
- Modify: `src/tui/app.rs` (`any_running`, `q` handling, tests)

- [ ] **Step 1: Write failing tests**

Add to the `tests` module:

```rust
#[test]
fn quit_is_immediate_when_nothing_running() {
    let mut app = app_with_two_tunnels(); // both Inactive
    let action = app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
    assert!(matches!(action, Some(Action::Quit)));
    assert_eq!(app.overlay, Overlay::None);
}

#[test]
fn quit_confirms_when_a_tunnel_is_running() {
    let mut app = app_with_two_tunnels();
    app.tunnels[0].status = TunnelStatus::Active;
    let action = app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
    assert!(action.is_none());
    assert_eq!(app.overlay, Overlay::ConfirmQuit);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test quit_ --quiet 2>&1 | tail -20`
Expected: FAIL — `q` currently always opens `ConfirmQuit`, so `quit_is_immediate_when_nothing_running` fails.

- [ ] **Step 3: Implement `any_running` and smart quit**

Add to `impl App`:

```rust
    fn any_running(&self) -> bool {
        self.tunnels.iter().any(|t| t.status.is_running())
    }
```

In `handle_main_key`, replace the `KeyCode::Char('q')` arm with:

```rust
            KeyCode::Char('q') => {
                if self.any_running() {
                    self.overlay = Overlay::ConfirmQuit;
                } else {
                    return Some(Action::Quit);
                }
            }
```

- [ ] **Step 4: Run tests**

Run: `cargo test --quiet 2>&1 | tail -20`
Expected: PASS, including both quit tests.

- [ ] **Step 5: Commit**

```bash
git add src/tui/app.rs
git commit --no-verify -m "feat(tui): smart quit skips confirm when nothing is running"
```

---

## Task 7: Filter input mode + Esc-to-clear

**Files:**
- Modify: `src/tui/app.rs` (`handle_key` dispatch, `handle_filter_key`, `/` and `Esc` in main, tests)

- [ ] **Step 1: Write failing tests**

Add to the `tests` module:

```rust
#[test]
fn slash_enters_filter_mode_and_typing_filters() {
    let mut app = app_with_two_tunnels();
    press(&mut app, KeyCode::Char('/'));
    assert!(app.filtering);
    press(&mut app, KeyCode::Char('b'));
    assert_eq!(app.filter.as_deref(), Some("b"));
    assert_eq!(app.visible_indices(), vec![1]);
}

#[test]
fn enter_commits_filter_and_exits_input() {
    let mut app = app_with_two_tunnels();
    press(&mut app, KeyCode::Char('/'));
    press(&mut app, KeyCode::Char('a'));
    press(&mut app, KeyCode::Enter);
    assert!(!app.filtering);
    assert_eq!(app.filter.as_deref(), Some("a"));
}

#[test]
fn esc_in_main_clears_active_filter() {
    let mut app = app_with_two_tunnels();
    app.filter = Some("a".into());
    press(&mut app, KeyCode::Esc);
    assert!(app.filter.is_none());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test filter --quiet 2>&1 | tail -20`
Expected: FAIL — `/`, filter input, and main-view `Esc` are unhandled.

- [ ] **Step 3: Dispatch to filter input when active**

In `handle_key`, change the `Overlay::None` arm so filter input takes precedence:

```rust
            Overlay::None => {
                if self.filtering {
                    self.handle_filter_key(key);
                    return None;
                }
                return self.handle_main_key(key);
            }
```

- [ ] **Step 4: Add `/` and `Esc` to the main keymap**

In `handle_main_key`, add these arms (before `_ => {}`):

```rust
            KeyCode::Char('/') => {
                self.filtering = true;
                self.filter = Some(String::new());
            }
            KeyCode::Esc => {
                if self.filter.is_some() {
                    self.filter = None;
                }
            }
```

- [ ] **Step 5: Implement `handle_filter_key`**

Add to `impl App`:

```rust
    fn handle_filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c) => {
                if let Some(q) = self.filter.as_mut() {
                    q.push(c);
                }
            }
            KeyCode::Backspace => {
                if let Some(q) = self.filter.as_mut() {
                    q.pop();
                }
            }
            KeyCode::Enter => {
                self.filtering = false;
                if self.filter.as_deref() == Some("") {
                    self.filter = None;
                }
            }
            KeyCode::Esc => {
                self.filtering = false;
                if self.filter.as_deref() == Some("") {
                    self.filter = None;
                }
            }
            _ => {}
        }
        self.clamp_cursor();
    }
```

- [ ] **Step 6: Run tests**

Run: `cargo test --quiet 2>&1 | tail -20`
Expected: PASS, including the three filter tests.

- [ ] **Step 7: Commit**

```bash
git add src/tui/app.rs
git commit --no-verify -m "feat(tui): name filtering with / and Esc-to-clear"
```

---

## Task 8: Help overlay + `n` to cancel confirms

**Files:**
- Modify: `src/tui/app.rs` (`Overlay::Help` variant, `?` keybind, overlay handling, `n` arms, tests)
- Modify: `src/tui/overlays.rs` (`draw_help`)
- Modify: `src/tui/view.rs` (dispatch to `draw_help`)

- [ ] **Step 1: Write failing tests**

Add to the `tests` module in `src/tui/app.rs`:

```rust
#[test]
fn question_mark_opens_help_and_closes() {
    let mut app = app_with_two_tunnels();
    press(&mut app, KeyCode::Char('?'));
    assert_eq!(app.overlay, Overlay::Help);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.overlay, Overlay::None);
}

#[test]
fn n_cancels_confirm_quit() {
    let mut app = app_with_two_tunnels();
    app.overlay = Overlay::ConfirmQuit;
    press(&mut app, KeyCode::Char('n'));
    assert_eq!(app.overlay, Overlay::None);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test help question n_cancels --quiet 2>&1 | tail -20`
Expected: FAIL — `Overlay::Help` does not exist; `n` not handled.

- [ ] **Step 3: Add the `Help` overlay variant**

In `src/tui/app.rs`, add to the `Overlay` enum:

```rust
    Help,
```

- [ ] **Step 4: Handle `?` and the Help overlay keys**

In `handle_main_key`, add an arm (before `_ => {}`):

```rust
            KeyCode::Char('?') => self.overlay = Overlay::Help,
```

In `handle_key`'s `match self.overlay`, add a new arm:

```rust
            Overlay::Help => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?')) {
                    self.overlay = Overlay::None;
                }
            }
```

- [ ] **Step 5: Accept `n` in both confirm dialogs**

In `handle_key`, update the cancel arms for `ConfirmQuit` and `ConfirmDelete` to include `n`:

```rust
            Overlay::ConfirmQuit => match key.code {
                KeyCode::Char('y') => return Some(Action::Quit),
                KeyCode::Char('q') | KeyCode::Char('n') | KeyCode::Esc => self.overlay = Overlay::None,
                _ => {}
            },
            Overlay::ConfirmDelete(idx) => match key.code {
                KeyCode::Char('y') => { self.remove_tunnel(idx); self.overlay = Overlay::None; }
                KeyCode::Char('q') | KeyCode::Char('n') | KeyCode::Esc => self.overlay = Overlay::None,
                _ => {}
            },
```

- [ ] **Step 6: Implement `draw_help`**

Add to `src/tui/overlays.rs`:

```rust
pub fn draw_help(f: &mut Frame, area: Rect) {
    let rect = centered(area, 56, 18);
    f.render_widget(Clear, rect);
    let block = dialog_block("❓ Keybindings", theme::PRIMARY);
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let row = |key: &'static str, desc: &'static str| {
        Line::from(vec![
            Span::styled(format!(" {key:<12}"), theme::accent()),
            Span::raw(desc),
        ])
    };
    let lines = vec![
        Line::from(Span::styled("Navigation", theme::title())),
        row("j / k  ↑ ↓", "move (wraps)"),
        row("g / G", "jump to top / bottom"),
        row("/", "filter by name"),
        Line::from(""),
        Line::from(Span::styled("Tunnels", theme::title())),
        row("Enter", "start / stop selected"),
        row("a", "start / stop all"),
        row("Space", "view logs"),
        row("r", "regenerate cert"),
        row("c", "create new tunnel"),
        row("d / Del", "delete tunnel"),
        Line::from(""),
        Line::from(Span::styled("App", theme::title())),
        row("?", "toggle this help"),
        row("q", "quit"),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}
```

- [ ] **Step 7: Dispatch Help in `view::draw`**

In `src/tui/view.rs`, add a match arm in the overlay dispatch block:

```rust
        Overlay::Help => overlays::draw_help(f, area),
```

- [ ] **Step 8: Run tests**

Run: `cargo test --quiet 2>&1 | tail -20`
Expected: PASS, including the help and cancel tests.

- [ ] **Step 9: Commit**

```bash
git add src/tui/app.rs src/tui/overlays.rs src/tui/view.rs
git commit --no-verify -m "feat(tui): help overlay and n-to-cancel confirms"
```

---

## Task 9: Refined view — summary line, 4-column scrollable table, empty state, footer

**Files:**
- Modify: `src/tui/view.rs` (`draw`, `draw_header`, `draw_table`, `draw_footer`, render test)

- [ ] **Step 1: Update `draw` to take `&mut App` and reserve the summary line**

`TableState` rendering needs `&mut`. In `src/tui/view.rs`, change the signature:

```rust
pub fn draw(f: &mut Frame, app: &mut App) {
```

Keep the existing four vertical chunks (header 5, table min 3, notification 1, footer 1). Update the dispatch calls to pass `app` (the header/notification/footer take `&App` via reborrow; the table takes `&mut App`):

```rust
    draw_header(f, chunks[0], app);
    draw_table(f, chunks[1], app);
    draw_notification(f, chunks[2], app);
    draw_footer(f, chunks[3], app);
```

In `run()` in `src/tui/app.rs`, the two `terminal.draw(|f| view::draw(f, self))?;` calls already pass `self` (which is `&mut App`); no change needed there.

- [ ] **Step 2: Add the summary line to the header**

Replace the body of `draw_header` so the right-hand column shows title, subtitle, and a status/filter summary:

```rust
fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::horizontal([Constraint::Length(8), Constraint::Min(0)]).split(area);

    let ascii = Paragraph::new(vec![
        Line::from("  ___"),
        Line::from(" (o o)"),
        Line::from(" (. .)"),
        Line::from("  \\-/ "),
    ])
    .style(theme::accent());
    f.render_widget(ascii, cols[0]);

    let title = Line::from(Span::styled(
        format!("Burrow v{} · cosy Azure tunnels", app.version),
        theme::title(),
    ));

    let active = app.tunnels.iter().filter(|t| t.status.is_running()).count();
    let visible = app.visible_indices().len();
    let summary = match &app.filter {
        Some(q) => {
            let unit = if visible == 1 { "match" } else { "matches" };
            Line::from(Span::styled(
                format!("Filter: {q} ({visible} {unit}) — Esc to clear"),
                theme::subtitle(),
            ))
        }
        None => Line::from(Span::styled(
            format!("{} tunnels · {} active", app.tunnels.len(), active),
            theme::subtitle(),
        )),
    };

    f.render_widget(Paragraph::new(vec![Line::from(""), title, summary]), cols[1]);
}
```

- [ ] **Step 3: Rewrite `draw_table` — 4 columns, merged cert, empty state, scrolling**

Replace `draw_table` entirely:

```rust
fn draw_table(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border())
        .title(Span::styled(" Tunnels ", theme::title()));

    if app.tunnels.is_empty() {
        let inner = block.inner(area);
        f.render_widget(block, area);
        let msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled("No tunnels yet", theme::accent())),
            Line::from(Span::styled("press c to create one", theme::muted())),
        ])
        .alignment(Alignment::Center);
        f.render_widget(msg, inner);
        return;
    }

    let header = Row::new(["Name", "Ports", "Status", "Cert"]).style(theme::title());

    let visible = app.visible_indices();
    let rows: Vec<Row> = visible
        .iter()
        .map(|&i| {
            let t = &app.tunnels[i];
            let ports = format!("{}→{}", t.local_port, t.remote_port);
            let cert = match (t.cert_status, &t.cert_expires_in) {
                (Some(c), Some(exp)) => format!("{} {}", c.label(), exp),
                (Some(c), None) => c.label().to_string(),
                (None, _) => "N/A".into(),
            };
            Row::new(vec![
                Cell::from(t.machine.name.clone()),
                Cell::from(ports),
                Cell::from(Line::from(status_span(&t.status))),
                Cell::from(cert),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(30),
        Constraint::Length(14),
        Constraint::Length(16),
        Constraint::Min(14),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::selected_row())
        .highlight_symbol("● ")
        .block(block);

    app.table_state
        .select((!visible.is_empty()).then_some(app.cursor.min(visible.len() - 1)));
    f.render_stateful_widget(table, area, &mut app.table_state);
}
```

- [ ] **Step 4: Update the footer hint**

Replace `draw_footer` body text:

```rust
fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let text = if app.tunnels.is_empty() {
        "c: create • q: quit • ?: help"
    } else {
        "↵ start/stop • ␣ logs • c new • a all • / filter • d del • ? help"
    };
    let p = Paragraph::new(text).style(theme::muted()).alignment(Alignment::Center);
    f.render_widget(p, area);
}
```

- [ ] **Step 5: Fix imports and the render test**

Ensure `src/tui/view.rs` still imports `Modifier` only if used; `cargo build` will flag unused imports. Remove any now-unused `Modifier`/`Style`/`Color` imports the compiler reports.

Update the render test at the bottom of `src/tui/view.rs` to pass `&mut`:

```rust
    #[test]
    fn renders_without_panicking_and_shows_title() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new("9.9".into(), Vec::new(),
            crate::azure::tunnel::TunnelManager::new(tx.clone()),
            crate::azure::cert::CertManager::new(tx));
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(content.contains("Burrow v9.9"));
        assert!(content.contains("No tunnels yet"));
    }
```

- [ ] **Step 6: Build, clippy, test**

Run: `cargo build && cargo clippy --all-targets -- -D warnings && cargo test --quiet 2>&1 | tail -20`
Expected: builds clean, no clippy warnings, all tests PASS.

- [ ] **Step 7: Commit**

```bash
git add src/tui/view.rs src/tui/app.rs
git commit --no-verify -m "feat(tui): refined 4-column scrollable table, summary line, empty state"
```

---

## Task 10: Final polish — fmt, manual smoke test, docs

**Files:**
- Modify: `README.md` (Usage/keybinds section, if present)

- [ ] **Step 1: Format**

Run: `cargo fmt`
Then: `git diff --stat` to see what changed.

- [ ] **Step 2: Full check**

Run: `cargo clippy --all-targets -- -D warnings && cargo test --quiet 2>&1 | tail -20`
Expected: clean clippy, all tests PASS.

- [ ] **Step 3: Manual smoke test**

Run: `cargo run` (with a `burrow.config.yaml` present, or just to see the empty state).
Verify: empty state shows "No tunnels yet"; `c` opens the wizard; `?` opens help and `Esc` closes it; `/` filters; `q` quits immediately when nothing is running.

- [ ] **Step 4: Update README keybind/usage docs**

In `README.md`, update the Usage section to reflect the new keys (`a` start/stop all, `/` filter, `g`/`G`, `?` help, smart quit). Keep wording consistent with the footer hint.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit --no-verify -m "docs: update README for new TUI keybindings"
```

- [ ] **Step 6: Push**

```bash
git push origin master
```

---

## Self-Review

**Spec coverage:**
- Header summary line + filter display → Task 9 Step 2. ✓
- 4-column scrollable table, merged Ports/Cert, status colors → Task 9 Step 3 (colors via existing `status_span`). ✓
- Empty state → Task 9 Step 3. ✓
- Footer ending in help hint → Task 9 Step 4. ✓
- Keybinds (nav/wrap, g/G → Task 4; Enter/Space/c/r/d existing + routed Task 3; a → Task 5; / → Task 7; ? → Task 8; smart quit → Task 6). ✓
- Help overlay → Task 8. ✓
- Filter input mode + Esc-to-clear → Task 7. ✓
- Confirm dialogs accept n → Task 8. ✓
- State additions (table_state, filter, filtering) → Task 2. ✓
- visible_indices/filtered resolution → Tasks 2–3. ✓
- theme.rs extraction → Task 1. ✓
- Tests for filter mapping, start/stop-all, smart quit, wrap nav → Tasks 2–8. ✓

**Placeholder scan:** No TBD/TODO; all steps include concrete code and commands.

**Type consistency:** `visible_indices`, `selected_real_index`, `clamp_cursor`, `toggle_all`, `any_running`, `handle_filter_key`, `Overlay::Help`, `draw_help` are defined where first referenced. `status_span` and `centered`/`dialog_block` reused from existing code. `draw` signature change to `&mut App` is propagated to its only callers (the two `terminal.draw` closures already pass `self: &mut App`) and the render test.
