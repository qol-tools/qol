# Reactive qol dev Dashboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Divergences (stale plugins, doctor warnings) surface on the `qol dev` dashboard automatically, cheaply, with event-driven dirtiness plus decaying periodic fallback; the tracer runs from session start.

**Architecture:** A generic `Poller<T>` (worker thread parked on `recv_timeout`, woken by `poke()`) replaces the bespoke probe threads in `dev_console.rs`. A `Probes` struct owns the pollers; `Dash` stays thread-free (testable) and records poke *intents* in a `Pokes` flag struct that the tick loop flushes. Doctor auto-runs reuse the prebuilt binary and back off 10s to 60s when results are unchanged.

**Tech Stack:** Rust, std mpsc threads (no async), ratatui 0.30, serde/serde_json, raw-socket HTTP client already in `dev_server.rs`.

**Spec:** `docs/superpowers/specs/2026-06-12-qol-dev-reactive-dashboard-design.md`

**Verification gate for every task:** `cargo test -p qol` and `cargo build -p qol` must pass before the task's commit. Final task runs the full workspace gate.

**Repo rules that apply (CLAUDE.md):** no code comments, table-driven tests with context in assertions, exhaustive matching (no `_ =>` on project enums), commit direct to main with one-line conventional messages, never push.

---

## File map

| File | Action | Responsibility |
|---|---|---|
| `tools/qol-cli/src/poller.rs` | Create | Generic `Poller<T>`: spawn, spawn_adaptive, latest, poke |
| `tools/qol-cli/src/main.rs` | Modify | Register `mod poller;` |
| `tools/qol-cli/Cargo.toml` | Modify | Add `serde` with derive |
| `tools/qol-cli/src/dev_server.rs` | Modify | `http_exchange` (status + body), `DevLink`, `fetch_dev_links` |
| `tools/qol-cli/src/dev_console.rs` | Modify | `Probes`/`Pokes`, doctor remodel, plugins/doctor/trace rows, endpoints lifecycle |

---

### Task 1: `Poller<T>` module

**Files:**
- Create: `tools/qol-cli/src/poller.rs`
- Modify: `tools/qol-cli/src/main.rs:1-9` (mod list)

- [ ] **Step 1: Create the module skeleton with the pure backoff function and its failing test**

Create `tools/qol-cli/src/poller.rs`:

```rust
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

fn next_wait(dirty: bool, wait: Duration, base: Duration, cap: Duration) -> Duration {
    if dirty {
        base
    } else {
        cap.min(wait.saturating_mul(2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_wait_resets_on_dirty_and_doubles_to_cap() {
        let cases = [
            (true, 40, 10, 60, 10),
            (false, 10, 10, 60, 20),
            (false, 20, 10, 60, 40),
            (false, 40, 10, 60, 60),
            (false, 60, 10, 60, 60),
        ];
        for (dirty, wait, base, cap, expected) in cases {
            assert_eq!(
                next_wait(
                    dirty,
                    Duration::from_secs(wait),
                    Duration::from_secs(base),
                    Duration::from_secs(cap),
                ),
                Duration::from_secs(expected),
                "dirty: {dirty} wait: {wait}"
            );
        }
    }
}
```

Add to `tools/qol-cli/src/main.rs` after `mod platform;`:

```rust
mod poller;
```

The unused imports (`channel`, `Receiver`, etc.) will trip `-D warnings`; for this intermediate step keep only `use std::time::Duration;` and add the mpsc imports in Step 3.

- [ ] **Step 2: Run the test**

Run: `cargo test -p qol poller`
Expected: PASS (1 test). The pure function is trivially right; the table is the spec for adaptive cadence.

- [ ] **Step 3: Add `Poller<T>` with thread tests**

Replace the top of `poller.rs` (above the tests module) with:

```rust
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

pub(crate) struct Poller<T> {
    rx: Receiver<T>,
    poke_tx: Sender<()>,
}

impl<T: Send + 'static> Poller<T> {
    pub(crate) fn spawn(interval: Duration, work: impl FnMut() -> T + Send + 'static) -> Self {
        Self::spawn_with_backoff(interval, interval, work, |_| true)
    }

    pub(crate) fn spawn_adaptive(
        base: Duration,
        cap: Duration,
        work: impl FnMut() -> T + Send + 'static,
    ) -> Self
    where
        T: Clone + PartialEq,
    {
        let mut prev: Option<T> = None;
        Self::spawn_with_backoff(base, cap, work, move |next| {
            let dirty = prev.as_ref() != Some(next);
            prev = Some(next.clone());
            dirty
        })
    }

    fn spawn_with_backoff(
        base: Duration,
        cap: Duration,
        mut work: impl FnMut() -> T + Send + 'static,
        mut changed: impl FnMut(&T) -> bool + Send + 'static,
    ) -> Self {
        let (result_tx, rx) = channel();
        let (poke_tx, poke_rx) = channel();
        std::thread::spawn(move || {
            let mut wait = base;
            loop {
                let result = work();
                let dirty = changed(&result);
                if result_tx.send(result).is_err() {
                    return;
                }
                match poke_rx.recv_timeout(wait) {
                    Ok(()) => {
                        while poke_rx.try_recv().is_ok() {}
                        wait = base;
                    }
                    Err(RecvTimeoutError::Timeout) => wait = next_wait(dirty, wait, base, cap),
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
        Self { rx, poke_tx }
    }

    pub(crate) fn latest(&self) -> Option<T> {
        let mut newest = None;
        while let Ok(value) = self.rx.try_recv() {
            newest = Some(value);
        }
        newest
    }

    pub(crate) fn poke(&self) {
        let _ = self.poke_tx.send(());
    }
}
```

Append to the tests module:

```rust
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    fn wait_for(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn periodic_poller_delivers_repeated_results() {
        let count = Arc::new(AtomicUsize::new(0));
        let work_count = count.clone();
        let poller = Poller::spawn(Duration::from_millis(5), move || {
            work_count.fetch_add(1, Ordering::SeqCst) + 1
        });
        assert!(
            wait_for(|| count.load(Ordering::SeqCst) >= 3),
            "expected repeated periodic runs"
        );
        assert!(wait_for(|| poller.latest().is_some()), "expected a result");
    }

    #[test]
    fn poke_wakes_a_long_interval_poller() {
        let count = Arc::new(AtomicUsize::new(0));
        let work_count = count.clone();
        let poller = Poller::spawn(Duration::from_secs(3600), move || {
            work_count.fetch_add(1, Ordering::SeqCst) + 1
        });
        assert!(
            wait_for(|| count.load(Ordering::SeqCst) == 1),
            "first run fires immediately on spawn"
        );
        poller.poke();
        assert!(
            wait_for(|| count.load(Ordering::SeqCst) == 2),
            "poke wakes the parked worker"
        );
    }

    #[test]
    fn drop_terminates_worker_thread() {
        let alive = Arc::new(());
        let held = alive.clone();
        let poller = Poller::spawn(Duration::from_millis(5), move || Arc::strong_count(&held));
        assert!(wait_for(|| poller.latest().is_some()), "poller produced a result");
        drop(poller);
        assert!(
            wait_for(|| Arc::strong_count(&alive) == 1),
            "worker thread exited and released its closure"
        );
    }
```

(The `wait_for` deadline loop bounds each assertion at 5s; assertions trigger far sooner in practice. No sleeps-as-assertions.)

- [ ] **Step 4: Run the tests**

Run: `cargo test -p qol poller`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add tools/qol-cli/src/poller.rs tools/qol-cli/src/main.rs
git commit -m "feat(qol-cli): add generic probe poller"
```

---

### Task 2: dev links fetch in `dev_server.rs`

**Files:**
- Modify: `tools/qol-cli/Cargo.toml:13-19` (dependencies)
- Modify: `tools/qol-cli/src/dev_server.rs:112-137` (`http_request`), new items

- [ ] **Step 1: Add serde to Cargo.toml**

In `tools/qol-cli/Cargo.toml` `[dependencies]`, before `serde_json`:

```toml
serde = { version = "1.0", features = ["derive"] }
```

- [ ] **Step 2: Write failing tests for body extraction and payload parsing**

Append inside the existing `mod tests` in `dev_server.rs`:

```rust
    #[test]
    fn response_body_splits_headers_from_payload() {
        let cases = [
            ("HTTP/1.1 200 OK\r\nA: b\r\n\r\n[1,2]", "[1,2]"),
            ("HTTP/1.1 204 No Content\r\n\r\n", ""),
            ("HTTP/1.1 200 OK no separator", ""),
        ];
        for (response, expected) in cases {
            assert_eq!(response_body(response), expected, "response: {response:?}");
        }
    }

    #[test]
    fn parses_dev_links_payload_ignoring_unknown_fields() {
        let payload = r#"[{"id":"a","name":"foo","source":"/a/b/c","needs_rebuild":true,"rebuild_reason":"Source changed","fingerprint":"x"}]"#;
        let links: Vec<DevLink> = serde_json::from_str(payload).unwrap();
        assert_eq!(links.len(), 1, "one link parsed");
        assert_eq!(links[0].name, "foo");
        assert!(links[0].needs_rebuild, "needs_rebuild carried through");
        assert_eq!(links[0].rebuild_reason, "Source changed");
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p qol dev_server`
Expected: FAIL to compile - `response_body` and `DevLink` not defined.

- [ ] **Step 4: Implement**

In `dev_server.rs`, add below `DevLinkOutcome`:

```rust
#[derive(Clone, PartialEq, serde::Deserialize)]
pub(crate) struct DevLink {
    pub(crate) name: String,
    pub(crate) needs_rebuild: bool,
    pub(crate) rebuild_reason: String,
}

pub(crate) fn fetch_dev_links() -> Result<Vec<DevLink>> {
    let (status, body) = http_exchange("GET", DEV_LINKS_URL, None)?;
    if status != 200 {
        bail!("GET {DEV_LINKS_URL} returned {status}");
    }
    serde_json::from_str(&body).context("invalid dev links payload")
}
```

(`bail!` may need adding to the existing `use anyhow::{...}` line.)

Refactor `http_request` into a status-only wrapper over a new `http_exchange`:

```rust
fn http_request(method: &str, url: &str, body: Option<&str>) -> Result<u16> {
    Ok(http_exchange(method, url, body)?.0)
}

fn http_exchange(method: &str, url: &str, body: Option<&str>) -> Result<(u16, String)> {
    let target = HttpTarget::parse(url)?;
    let mut addrs = (target.host.as_str(), target.port)
        .to_socket_addrs()
        .with_context(|| format!("failed to resolve {}", target.host))?;
    let addr = addrs
        .next()
        .ok_or_else(|| anyhow!("no address for {}", target.host))?;
    let mut stream = TcpStream::connect_timeout(&addr, HTTP_TIMEOUT)
        .with_context(|| format!("failed to connect to {}", target.host))?;
    stream.set_read_timeout(Some(HTTP_TIMEOUT))?;
    stream.set_write_timeout(Some(HTTP_TIMEOUT))?;
    let body = body.unwrap_or("");
    let request = format!(
        "{method} {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        target.path,
        target.host,
        target.port,
        body.len(),
        body
    );
    stream.write_all(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let status = parse_http_status(&response)?;
    Ok((status, response_body(&response)))
}

fn response_body(response: &str) -> String {
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default()
}
```

(The dev server responds with `content-length` framing, verified live; no chunked decoding needed.)

- [ ] **Step 5: Run tests**

Run: `cargo test -p qol dev_server`
Expected: PASS (existing + 2 new).

- [ ] **Step 6: Commit**

```bash
git add tools/qol-cli/Cargo.toml Cargo.lock tools/qol-cli/src/dev_server.rs
git commit -m "feat(qol-cli): fetch dev links from the dev server"
```

---

### Task 3: `Probes` + `Pokes`; convert health and emu

**Files:**
- Modify: `tools/qol-cli/src/dev_console.rs` (Dash struct, run_session, tui_session, open_emu, drain_emu_run, emu_status callers)

All changes in `dev_console.rs`.

- [ ] **Step 1: Write the failing intent test**

Add to the `mod tests` at the bottom:

```rust
    #[test]
    fn diving_into_emu_requests_an_emu_poke() {
        let mut dash = Dash::new(Vec::new());
        dash.cursor = 3;
        apply_action(&mut dash, Action::Dive, false);
        assert!(dash.pokes.emu, "emu dive marks the emu probe dirty");
        assert!(matches!(dash.view, View::Emu), "dive opened the emu view");
    }
```

Run: `cargo test -p qol diving_into_emu`
Expected: FAIL to compile - no `pokes` field.

- [ ] **Step 2: Add the types and rewire**

Add near `HealthSnapshot`:

```rust
struct Probes {
    health: Poller<HealthSnapshot>,
    emu: Poller<Result<Vec<EnvironmentStatus>, String>>,
}

impl Probes {
    fn spawn() -> Self {
        Self {
            health: Poller::spawn(HEALTH_PROBE_INTERVAL, || HealthSnapshot {
                api: health_ok(),
                web: web_ok(),
            }),
            emu: Poller::spawn(EMU_REFRESH_INTERVAL, || {
                environment_statuses().map_err(|error| format!("{error:#}"))
            }),
        }
    }
}

#[derive(Default)]
struct Pokes {
    emu: bool,
}

fn flush_pokes(dash: &mut Dash, probes: &Probes) {
    if std::mem::take(&mut dash.pokes.emu) {
        probes.emu.poke();
    }
}
```

Import: `use crate::poller::Poller;`

`Dash` struct: delete fields `emu_rx`, `emu_last_refresh`; add `pokes: Pokes`. In `Dash::new` delete the two initializers, add `pokes: Pokes::default()`. Delete the `start_emu_refresh` method, `spawn_health_probe`, and `spawn_emu_probe`.

`run_session`: replace `let health = spawn_health_probe();` with `let mut probes = Probes::spawn();` and pass `&mut probes` to `tui_session` in place of `&health`.

`tui_session` signature: replace `health: &Receiver<HealthSnapshot>` with `probes: &mut Probes`. In its loop body:

- Delete `dash.start_emu_refresh(false);`.
- Replace the health drain block with:

```rust
        if let Some(snapshot) = probes.health.latest() {
            apply_health(dash, snapshot);
        }
```

and add the helper (next to `health_state`):

```rust
fn apply_health(dash: &mut Dash, snapshot: HealthSnapshot) {
    dash.health = health_state(snapshot.api);
    dash.web = health_state(snapshot.web);
}
```

- Replace the emu drain block with:

```rust
        if let Some(outcome) = probes.emu.latest() {
            dash.emu = match outcome {
                Ok(statuses) => EmuState::Done(statuses),
                Err(error) => EmuState::Failed(error),
            };
        }
```

- Add `flush_pokes(dash, probes);` immediately before `terminal.draw(...)`.

`open_emu`: replace `dash.start_emu_refresh(true);` with `dash.pokes.emu = true;` (silent refresh: no `EmuState::Probing` reset; the initial value in `Dash::new` stays `EmuState::Probing` until the first result lands).

`drain_emu_run`: replace the final `dash.start_emu_refresh(true);` with `dash.pokes.emu = true;`.

- [ ] **Step 3: Run tests and build**

Run: `cargo test -p qol && cargo build -p qol`
Expected: PASS, including the new intent test and the untouched cursor/action tests.

- [ ] **Step 4: Commit**

```bash
git add tools/qol-cli/src/dev_console.rs
git commit -m "refactor(qol-cli): drive health and emu probes through pollers"
```

---

### Task 4: live plugins row

**Files:**
- Modify: `tools/qol-cli/src/dev_console.rs` (Probes, Pokes, Dash, plugins_status, draw_plugins, draw_dashboard)

- [ ] **Step 1: Write failing tests for the row spans**

Add to `mod tests`:

```rust
    fn span_text(spans: &[Span<'static>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    #[test]
    fn plugins_status_reflects_link_state() {
        let fresh = DevLink {
            name: "foo".to_string(),
            needs_rebuild: false,
            rebuild_reason: "Up to date".to_string(),
        };
        let stale = DevLink {
            name: "bar".to_string(),
            needs_rebuild: true,
            rebuild_reason: "Source changed".to_string(),
        };
        let cases = [
            (LinksState::Live(vec![fresh.clone(), stale.clone()]), Color::Yellow, "2 linked · 1 stale"),
            (LinksState::Live(vec![fresh.clone()]), Color::Green, "1 linked"),
            (LinksState::Unknown, Color::Green, "3 linked"),
            (LinksState::Unreachable, Color::Yellow, "3 linked · api down"),
        ];
        for (links, expected_color, expected_text) in cases {
            let (color, spans) = plugins_status(&RebuildState::Idle, 3, &links);
            assert_eq!(color, expected_color, "text: {expected_text}");
            assert_eq!(span_text(&spans), expected_text);
        }
    }

    #[test]
    fn plugins_status_appends_reload_failure() {
        let (color, spans) = plugins_status(
            &RebuildState::Failed("boom".to_string()),
            3,
            &LinksState::Unknown,
        );
        assert_eq!(color, Color::Red, "failed reload turns the row red");
        assert!(span_text(&spans).contains("reload failed · boom"), "spans: {}", span_text(&spans));
    }
```

`Color` needs `PartialEq` in assertions - ratatui's `Color` derives it already. `RebuildState` and `LinksState` need no derives (constructed directly).

Run: `cargo test -p qol plugins_status`
Expected: FAIL to compile - no `LinksState`, `plugins_status` has the old arity.

- [ ] **Step 2: Implement**

Imports: add `DevLink` and `fetch_dev_links` to the existing `use crate::dev_server::{...}` list.

Add near `EmuState`:

```rust
enum LinksState {
    Unknown,
    Live(Vec<DevLink>),
    Unreachable,
}
```

Add `const LINKS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);` next to `EMU_REFRESH_INTERVAL`.

`Probes`: add field `links: Poller<Result<Vec<DevLink>, String>>`, spawned in `Probes::spawn` as:

```rust
            links: Poller::spawn(LINKS_REFRESH_INTERVAL, || {
                fetch_dev_links().map_err(|error| format!("{error:#}"))
            }),
```

`Pokes`: add `links: bool`. `flush_pokes`: add

```rust
    if std::mem::take(&mut dash.pokes.links) {
        probes.links.poke();
    }
```

`Dash`: add field `links: LinksState`, init `LinksState::Unknown`.

`apply_health` gains the first-health-up poke:

```rust
fn apply_health(dash: &mut Dash, snapshot: HealthSnapshot) {
    let was_up = dash.health == Health::Up;
    dash.health = health_state(snapshot.api);
    dash.web = health_state(snapshot.web);
    if !was_up && dash.health == Health::Up {
        dash.pokes.links = true;
    }
}
```

`tui_session` loop, after the emu drain:

```rust
        if let Some(outcome) = probes.links.latest() {
            dash.links = match outcome {
                Ok(links) => LinksState::Live(links),
                Err(_) => LinksState::Unreachable,
            };
        }
```

`trigger_reload`: on the `Ok` arm set `dash.pokes.links = true;`:

```rust
fn trigger_reload(dash: &mut Dash) {
    dash.plugin_reload = match post_reload_plugins() {
        Ok(()) => {
            dash.pokes.links = true;
            RebuildState::Requested(Instant::now())
        }
        Err(error) => RebuildState::Failed(format!("{error:#}")),
    };
}
```

Rewrite `plugins_status`:

```rust
fn plugins_status(
    state: &RebuildState,
    boot_count: usize,
    links: &LinksState,
) -> (Color, Vec<Span<'static>>) {
    let (live_color, mut value) = match links {
        LinksState::Live(links) => {
            let stale = links.iter().filter(|link| link.needs_rebuild).count();
            if stale > 0 {
                (
                    Color::Yellow,
                    vec![
                        format!("{} linked", links.len()).fg(Color::Green),
                        format!(" · {stale} stale").fg(Color::Yellow).bold(),
                    ],
                )
            } else {
                (
                    Color::Green,
                    vec![format!("{} linked", links.len()).fg(Color::Green)],
                )
            }
        }
        LinksState::Unknown => (
            Color::Green,
            vec![format!("{boot_count} linked").fg(Color::DarkGray)],
        ),
        LinksState::Unreachable => (
            Color::Yellow,
            vec![
                format!("{boot_count} linked").fg(Color::DarkGray),
                " · api down".fg(Color::DarkGray),
            ],
        ),
    };
    let color = match state {
        RebuildState::Requested(at) if at.elapsed() < ACK_TTL => {
            value.push(" · reload sent".fg(Color::Yellow));
            live_color
        }
        RebuildState::Failed(error) => {
            value.push(" · reload ".fg(Color::DarkGray));
            value.push("failed".fg(Color::Red).bold());
            value.push(format!(" · {error}").fg(Color::DarkGray));
            Color::Red
        }
        RebuildState::Idle | RebuildState::Requested(_) => live_color,
    };
    (color, value)
}
```

`draw_dashboard` call site becomes:

```rust
    let (plugins_color, plugins_value) =
        plugins_status(&dash.plugin_reload, dash.plugin_names.len(), &dash.links);
```

Rewrite `draw_plugins` to render live links:

```rust
fn draw_plugins(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let accent = frame_accent(dash);
    let entries = plugin_view_lines(dash);
    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new("  no dev-linked plugins").block(panel(" plugins ", accent)),
            area,
        );
        return;
    }
    let total = entries.len();
    let (start, height) = list_window(dash, area, total);
    let visible: Vec<Line> = entries.into_iter().skip(start).take(height).collect();
    let title = format!(" plugins · {} ", list_status(total, dash.scroll_offset));
    frame.render_widget(Paragraph::new(visible).block(panel(&title, accent)), area);
}

fn plugin_view_lines(dash: &Dash) -> Vec<Line<'static>> {
    match &dash.links {
        LinksState::Live(links) => links.iter().map(plugin_link_line).collect(),
        LinksState::Unknown | LinksState::Unreachable => dash
            .plugin_names
            .iter()
            .map(|name| {
                Line::from(vec![
                    "  ".into(),
                    "●".fg(Color::DarkGray).bold(),
                    format!(" {name}").fg(Color::White),
                    " · link state unknown".fg(Color::DarkGray),
                ])
            })
            .collect(),
    }
}

fn plugin_link_line(link: &DevLink) -> Line<'static> {
    if link.needs_rebuild {
        return Line::from(vec![
            "  ".into(),
            "●".fg(Color::Yellow).bold(),
            format!(" {}", link.name).fg(Color::White),
            " · stale · ".fg(Color::Yellow),
            link.rebuild_reason.clone().fg(Color::DarkGray),
        ]);
    }
    Line::from(vec![
        "  ".into(),
        "●".fg(Color::Green).bold(),
        format!(" {}", link.name).fg(Color::White),
        " · dev-linked".fg(Color::DarkGray),
    ])
}
```

- [ ] **Step 3: Run tests and build**

Run: `cargo test -p qol && cargo build -p qol`
Expected: PASS including both new tests.

- [ ] **Step 4: Commit**

```bash
git add tools/qol-cli/src/dev_console.rs
git commit -m "feat(qol-cli): live plugins row with stale link detection"
```

---

### Task 5: doctor auto-refresh with adaptive cadence

**Files:**
- Modify: `tools/qol-cli/src/dev_console.rs` (DoctorReport/DoctorRun derives, DoctorPanel replaces DoctorState, run_doctor split, Probes/Pokes, doctor_status, draw_doctor, relative_age, act_row, open_doctor, run_session)

- [ ] **Step 1: Write failing tests**

Add to `mod tests`:

```rust
    #[test]
    fn relative_age_gains_a_seconds_bucket() {
        let cases = [
            (5_000, "just now"),
            (10_000, "10s ago"),
            (59_000, "59s ago"),
            (60_000, "1m ago"),
        ];
        for (elapsed_ms, expected) in cases {
            assert_eq!(
                relative_age(1_000_000_000 + elapsed_ms, 1_000_000_000),
                expected,
                "elapsed_ms: {elapsed_ms}"
            );
        }
    }

    #[test]
    fn doctor_status_covers_panel_states() {
        let report = DoctorReport { ok: 11, warn: 0, error: 0, crash: 0 };
        let run = DoctorRun { report, lines: Vec::new() };
        let warn_report = DoctorReport { ok: 9, warn: 2, error: 0, crash: 0 };
        let warn_run = DoctorRun { report: warn_report, lines: Vec::new() };
        let now = 1_000_000_000;
        let cases = [
            (
                DoctorPanel { last: None, last_at_ms: None, manual: None, error: None },
                Color::Yellow,
                "waiting for first check",
            ),
            (
                DoctorPanel {
                    last: Some(run.clone()),
                    last_at_ms: Some(now - 15_000),
                    manual: None,
                    error: None,
                },
                Color::Green,
                "all good · 11 checks · 15s ago",
            ),
            (
                DoctorPanel {
                    last: Some(warn_run.clone()),
                    last_at_ms: Some(now - 5_000),
                    manual: None,
                    error: None,
                },
                Color::Yellow,
                "2 divergences · 2 warn · 0 err · just now",
            ),
            (
                DoctorPanel {
                    last: Some(run.clone()),
                    last_at_ms: Some(now - 15_000),
                    manual: None,
                    error: Some("boom".to_string()),
                },
                Color::Green,
                "all good · 11 checks · 15s ago · probe failed",
            ),
            (
                DoctorPanel {
                    last: None,
                    last_at_ms: None,
                    manual: None,
                    error: Some("doctor binary not built · press d".to_string()),
                },
                Color::Yellow,
                "doctor binary not built · press d",
            ),
        ];
        for (panel, expected_color, expected_text) in cases {
            let (color, spans) = doctor_status(&panel, now);
            assert_eq!(color, expected_color, "text: {expected_text}");
            assert_eq!(span_text(&spans), expected_text);
        }
    }
```

Also update the existing `relative_age_buckets_seconds_minutes_hours_days` test rows `(59_000, "just now")` to `(59_000, "59s ago")` and `(5_000, "just now")` stays.

Run: `cargo test -p qol doctor_status`
Expected: FAIL to compile - `DoctorPanel` undefined, `doctor_status` has the old signature.

- [ ] **Step 2: Implement the state remodel**

Derives:

```rust
#[derive(Clone, Copy, PartialEq)]
struct DoctorReport { ... }   // existing fields unchanged

#[derive(Clone, PartialEq)]
struct DoctorRun { ... }      // existing fields unchanged
```

Replace `enum DoctorState` (delete it) with:

```rust
struct DoctorPanel {
    last: Option<DoctorRun>,
    last_at_ms: Option<u64>,
    manual: Option<(DoctorMode, Receiver<Result<DoctorRun, String>>)>,
    error: Option<String>,
}
```

`Dash`: replace fields `doctor: DoctorState`, `doctor_lines: Vec<String>`, `doctor_rx: Option<...>` with `doctor: DoctorPanel`. Init in `Dash::new`:

```rust
            doctor: DoctorPanel {
                last: None,
                last_at_ms: None,
                manual: None,
                error: None,
            },
```

Replace `start_doctor`/`start_doctor_fix` methods with one:

```rust
    fn start_doctor(&mut self, mode: DoctorMode) {
        self.doctor.manual = Some((mode, spawn_doctor(mode)));
    }
```

Call sites: `act_row` Doctor arm becomes

```rust
        Row::Doctor => {
            if modified {
                dash.start_doctor(DoctorMode::Fix);
            } else {
                dash.start_doctor(DoctorMode::Check);
            }
        }
```

`open_doctor` no longer rebuilds; it opens the view and marks the probe dirty:

```rust
fn open_doctor(dash: &mut Dash) {
    dash.view = View::Doctor;
    dash.scroll_offset = 0;
    dash.pokes.doctor = true;
}
```

`run_session`: delete the `dash.start_doctor();` line (the auto poller's immediate first run replaces it; boot preflight already built the binary).

- [ ] **Step 3: Split the doctor runner and add the probe**

Split `run_doctor` so the prebuilt path skips the build:

```rust
fn run_doctor(mode: DoctorMode) -> Result<DoctorRun, String> {
    let root = crate::workspace::repo_root().map_err(|error| format!("{error:#}"))?;
    build_doctor(&root)?;
    run_doctor_binary(&doctor_binary(&root), &root, mode)
}

fn run_doctor_prebuilt() -> Result<DoctorRun, String> {
    let root = crate::workspace::repo_root().map_err(|error| format!("{error:#}"))?;
    let binary = doctor_binary(&root);
    if !binary.exists() {
        return Err("doctor binary not built · press d".to_string());
    }
    run_doctor_binary(&binary, &root, DoctorMode::Check)
}

fn doctor_binary(root: &std::path::Path) -> std::path::PathBuf {
    root.join("target")
        .join("debug")
        .join(host_facade::exe_name("qol-tray-doctor"))
}

fn run_doctor_binary(
    binary: &std::path::Path,
    root: &std::path::Path,
    mode: DoctorMode,
) -> Result<DoctorRun, String> {
    let output = Command::new(binary)
        .current_dir(root)
        .arg(mode.arg())
        .output()
        .map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout);
    let report =
        parse_doctor_summary(&text).ok_or_else(|| "could not read doctor summary".to_string())?;
    Ok(DoctorRun {
        report,
        lines: doctor_lines(&text, mode),
    })
}
```

Constants next to the other intervals:

```rust
const DOCTOR_BASE_INTERVAL: Duration = Duration::from_secs(10);
const DOCTOR_CAP_INTERVAL: Duration = Duration::from_secs(60);
```

`Probes`: add field `doctor: Poller<Result<DoctorRun, String>>`, spawned via a helper (also used for respawn):

```rust
fn spawn_doctor_probe() -> Poller<Result<DoctorRun, String>> {
    Poller::spawn_adaptive(DOCTOR_BASE_INTERVAL, DOCTOR_CAP_INTERVAL, run_doctor_prebuilt)
}
```

`Pokes`: add `doctor: bool`; `flush_pokes` add the matching take/poke block.

`apply_health`: also set `dash.pokes.doctor = true;` inside the health-up transition branch.

`trigger_rebuild` Ok arm: set `dash.pokes.doctor = true;` (same shape as `trigger_reload` in Task 4). `trigger_reload` Ok arm: additionally set `dash.pokes.doctor = true;`. `drain_emu_run` end: add `dash.pokes.doctor = true;` next to the emu poke.

- [ ] **Step 4: Rewire the tick loop drain**

Replace the old `doctor_outcome` block in `tui_session` with:

```rust
        let manual_outcome = dash
            .doctor
            .manual
            .as_ref()
            .and_then(|(_, rx)| rx.try_recv().ok());
        if let Some(outcome) = manual_outcome {
            dash.doctor.manual = None;
            apply_doctor_outcome(dash, outcome);
            probes.doctor = spawn_doctor_probe();
        } else if dash.doctor.manual.is_none() {
            if let Some(outcome) = probes.doctor.latest() {
                apply_doctor_outcome(dash, outcome);
            }
        } else {
            let _ = probes.doctor.latest();
        }
```

(Respawning the poller after a manual run discards stale in-flight auto results; its immediate first run doubles as the post-manual re-check.)

Helper:

```rust
fn apply_doctor_outcome(dash: &mut Dash, outcome: Result<DoctorRun, String>) {
    match outcome {
        Ok(run) => {
            dash.doctor.last = Some(run);
            dash.doctor.last_at_ms = Some(now_unix_ms());
            dash.doctor.error = None;
        }
        Err(error) => dash.doctor.error = Some(error),
    }
}
```

- [ ] **Step 5: Rewrite rendering**

`relative_age` seconds bucket:

```rust
fn relative_age(now_ms: u64, then_ms: u64) -> String {
    let seconds = now_ms.saturating_sub(then_ms) / 1000;
    match seconds {
        0..=9 => "just now".to_string(),
        10..=59 => format!("{seconds}s ago"),
        60..=3599 => format!("{}m ago", seconds / 60),
        3600..=86399 => format!("{}h ago", seconds / 3600),
        _ => format!("{}d ago", seconds / 86400),
    }
}
```

`doctor_status` rewrite:

```rust
fn doctor_status(panel: &DoctorPanel, now_ms: u64) -> (Color, Vec<Span<'static>>) {
    if let Some((mode, _)) = &panel.manual {
        return (Color::Yellow, vec![mode.gerund().fg(Color::Yellow)]);
    }
    let Some(run) = &panel.last else {
        let detail = panel
            .error
            .clone()
            .unwrap_or_else(|| "waiting for first check".to_string());
        return (Color::Yellow, vec![detail.fg(Color::DarkGray)]);
    };
    let report = run.report;
    let (color, mut value) = if report.divergences() == 0 {
        (
            Color::Green,
            vec![
                "all good".fg(Color::Green).bold(),
                format!(" · {} checks", report.ok).fg(Color::DarkGray),
            ],
        )
    } else {
        let color = if report.error + report.crash > 0 {
            Color::Red
        } else {
            Color::Yellow
        };
        (
            color,
            vec![
                format!("{} divergences", report.divergences())
                    .fg(color)
                    .bold(),
                format!(
                    " · {} warn · {} err",
                    report.warn,
                    report.error + report.crash
                )
                .fg(Color::DarkGray),
            ],
        )
    };
    if let Some(at) = panel.last_at_ms {
        value.push(format!(" · {}", relative_age(now_ms, at)).fg(Color::DarkGray));
    }
    if panel.error.is_some() {
        value.push(" · probe failed".fg(Color::DarkGray));
    }
    (color, value)
}
```

`draw_dashboard` call site: `let (doctor_color, doctor_value) = doctor_status(&dash.doctor, now_unix_ms());`

`draw_doctor` rewrite (lines come from the panel; title gains the age):

```rust
fn draw_doctor(frame: &mut Frame, dash: &mut Dash, area: Rect) {
    let accent = frame_accent(dash);
    let lines = doctor_view_lines(&dash.doctor);
    if lines.is_empty() {
        let message = match &dash.doctor.manual {
            Some((mode, _)) => mode.progress_message(),
            None => "  no checks reported · press d to run",
        };
        frame.render_widget(
            Paragraph::new(message).block(panel(" doctor ", accent)),
            area,
        );
        return;
    }
    let total = lines.len();
    let (start, height) = list_window(dash, area, total);
    let render = if dash.armed {
        styled_doctor_line
    } else {
        friendly_doctor_line
    };
    let visible: Vec<Line> = lines
        .iter()
        .skip(start)
        .take(height)
        .map(|line| render(line))
        .collect();
    let age = dash
        .doctor
        .last_at_ms
        .map(|at| format!(" · {}", relative_age(now_unix_ms(), at)))
        .unwrap_or_default();
    let title = format!(" doctor · {}{age} ", list_status(total, dash.scroll_offset));
    frame.render_widget(Paragraph::new(visible).block(panel(&title, accent)), area);
}

fn doctor_view_lines(panel: &DoctorPanel) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(error) = &panel.error {
        lines.push(format!("[ERR] doctor: {error}"));
    }
    if let Some(run) = &panel.last {
        lines.extend(run.lines.iter().cloned());
    }
    lines
}
```

(`styled_doctor_line`/`friendly_doctor_line` take `&str` and return owned `Line<'static>` data today via `.to_string()`/`.into()` on owned values; if the borrow checker objects to `Line<'_>` tied to `lines`, map with `|line| render(line.as_str())` - the function bodies already produce owned spans.)

- [ ] **Step 6: Run tests and build**

Run: `cargo test -p qol && cargo build -p qol`
Expected: PASS, including both new tests and the updated `relative_age` table.

- [ ] **Step 7: Commit**

```bash
git add tools/qol-cli/src/dev_console.rs
git commit -m "feat(qol-cli): auto-refresh doctor with adaptive cadence"
```

---

### Task 6: endpoints re-probe while the view is open

**Files:**
- Modify: `tools/qol-cli/src/dev_console.rs` (Probes, tui_session, open_endpoints, Dash)

- [ ] **Step 1: Implement (no unit test - pure lifecycle wiring, covered by the build and smoke test)**

`Dash`: delete field `endpoints_rx`. Delete `spawn_endpoints_probe`.

Constant: `const ENDPOINTS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);`

`Probes`: add field `endpoints: Option<Poller<Vec<EndpointStatus>>>`, init `None` in `spawn()`.

`open_endpoints` shrinks to:

```rust
fn open_endpoints(dash: &mut Dash) {
    dash.view = View::Endpoints;
    dash.scroll_offset = 0;
}
```

(`dash.endpoints` keeps its last results; `EndpointsState::Probing` from `Dash::new` shows only before the first ever probe.)

`tui_session` loop, replace the old endpoints drain with lifecycle + drain:

```rust
        match (dash.view == View::Endpoints, probes.endpoints.is_some()) {
            (true, false) => {
                probes.endpoints = Some(Poller::spawn(ENDPOINTS_REFRESH_INTERVAL, probe_endpoints));
            }
            (false, true) => probes.endpoints = None,
            (true, true) | (false, false) => {}
        }
        if let Some(results) = probes.endpoints.as_ref().and_then(|poller| poller.latest()) {
            dash.endpoints = EndpointsState::Done(results);
        }
```

- [ ] **Step 2: Run tests and build**

Run: `cargo test -p qol && cargo build -p qol`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tools/qol-cli/src/dev_console.rs
git commit -m "feat(qol-cli): re-probe endpoints while the view is open"
```

---

### Task 7: trace auto-start

**Files:**
- Modify: `tools/qol-cli/src/dev_console.rs` (run_session, open_trace, apply_action Back arm, trace_value, Dash)

- [ ] **Step 1: Implement**

`Dash`: add field `trace_unavailable: bool`, init `false`.

Extract the spawn-or-flag logic from `open_trace` into:

```rust
fn start_trace(dash: &mut Dash) {
    if dash.trace_child.is_some() {
        return;
    }
    match spawn_trace() {
        Some((child, rx)) => {
            dash.trace_child = Some(child);
            dash.trace_rx = Some(rx);
            dash.trace_unavailable = false;
        }
        None => {
            if !dash.trace_unavailable {
                dash.trace.push(
                    "[qol dev] could not start tracer (need python3 + tools/compact_trace.py)"
                        .to_string(),
                );
            }
            dash.trace_unavailable = true;
        }
    }
}

fn open_trace(dash: &mut Dash) {
    dash.view = View::Trace;
    dash.scroll_offset = 0;
    start_trace(dash);
}
```

`run_session`, after `dash.start_doctor()` was removed in Task 5 (i.e. right after `dash.boot_rx = boot;`):

```rust
    start_trace(&mut dash);
```

`apply_action` `Action::Back` arm: delete the two lines

```rust
            if dash.view == View::Trace {
                stop_trace(dash);
            }
```

(The tracer still stops via the existing `stop_trace` calls on quit, reload-ready, and child exit.)

`trace_value`:

```rust
fn trace_value(dash: &Dash) -> Vec<Span<'static>> {
    if dash.trace_child.is_some() {
        return vec![format!("{} lines", dash.trace.len()).fg(Color::DarkGray)];
    }
    if dash.trace_unavailable {
        return vec!["tracer unavailable".fg(Color::DarkGray)];
    }
    vec!["idle · → open".fg(Color::DarkGray)]
}
```

- [ ] **Step 2: Run tests and build**

Run: `cargo test -p qol && cargo build -p qol`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tools/qol-cli/src/dev_console.rs
git commit -m "feat(qol-cli): start the tracer at session boot"
```

---

### Task 8: full gate + smoke test

- [ ] **Step 1: Workspace gate**

Run, in order, from the repo root:

```bash
cargo fmt --all --check
cargo clippy -p qol --all-targets --all-features --keep-going -- -D warnings
cargo test --workspace
cargo build -p qol
```

Expected: all green. (`cargo test --workspace` is the real gate per repo memory - `-p` feature unification differs. Tolerated noise: the `block v0.1.6` future-incompat warning.)

- [ ] **Step 2: Manual smoke (requires the user's terminal; report the checklist, do not run `qol dev` from an agent)**

Checklist to hand to the user:

1. `qol dev` - doctor row populates within ~12s of boot without pressing anything, then shows a dim age that ticks (`just now` → `15s ago`).
2. Touch a plugin source file - plugins row shows `· N stale` within ~5s; the plugins view names the stale plugin with its reason.
3. Press ctrl+r - after the rebuild lands, doctor and links rows catch up without keypresses.
4. Trace row counts lines from boot without ever opening the trace view; entering and leaving the trace view does not reset the count.
5. Open web → endpoints, kill the tray, watch endpoint rows flip to ✗ within ~5s while the view stays open.
6. Idle for 3+ minutes - confirm (via Activity Monitor or noise) that doctor runs decay rather than firing every 10s.

- [ ] **Step 3: No commit unless fixes were needed; if fixes, amend the relevant task commit**

---

## Self-review notes (already applied)

- Spec coverage: Poller (T1), http_get_json equivalent `http_exchange`+`fetch_dev_links` (T2), probe inventory (T3-T6), doctor semantics incl. silent refresh/age/respawn/no-build (T5), plugins row (T4), endpoints lifecycle (T6), trace auto-start (T7), first-health-up pokes (T4 links, T5 doctor), tests (each task).
- Deviation from spec wording: spec names the helper `http_get_json<T>`; the implementation is `http_exchange` + typed `fetch_dev_links` because only one call site exists (YAGNI).
- Type consistency: `DoctorPanel { last, last_at_ms, manual, error }` used identically in T5 tests and code; `Pokes { emu, links, doctor }` grows monotonically T3→T5; `plugins_status(&RebuildState, usize, &LinksState)` arity matches tests.
- `Dash::new` keeps its `Vec<String>` signature, so existing tests stay valid; no poller is constructed in any unit test (no HTTP/process side effects - `trigger_rebuild`/`trigger_reload` are deliberately untested because they POST to a possibly-live server).
