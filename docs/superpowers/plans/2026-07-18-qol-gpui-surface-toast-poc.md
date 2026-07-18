# qol-gpui Surface Toast POC Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `qol_gpui::surface` with a `Toast` kind and use it in qol-shot as an opt-in replacement for the OS "saved" notification.

**Architecture:** A builder in qol-gpui composes existing primitives (MonitorTracker placement, gpui window options, timeout dismiss with generation guard) into a never-focused toast window.
qol-shot gains a `capture.saved_feedback` select; when set to `toast`, the daemon shows the toast (click reveals the file via the existing `RevealTarget`), falling back to the OS notification if window creation fails.

**Tech Stack:** Rust, gpui 0.2, qol-gpui, qol-theme, qol-config contracts.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-07-18-qol-gpui-surface-toast-poc-design.md`.
- No code comments anywhere.
- Toast content is text-only (title + file name). No thumbnail.
- The toast never takes focus; keyboard reveal stays on the notification path.
- `background_saved` (headless CLI path, no gpui App) keeps the OS notification unconditionally.
- Default behavior is unchanged: `saved_feedback` defaults to `notification`.
- Nothing beyond `SurfaceKind::Toast` lands in the surface module.
- Never create popup windows on the host session to verify behavior; runtime checks happen in a `qol env up` guest.
- All commits direct to `main`, conventional one-liners, no AI attribution.

---

### Task 1: Toast placement math in qol-gpui

**Files:**
- Create: `libs/qol-gpui/src/surface.rs`
- Modify: `libs/qol-gpui/src/lib.rs` (add `pub mod surface;` after `pub mod scroll_list;`)

**Interfaces:**
- Produces: `surface::Corner` (enum: `TopLeft | TopRight | BottomLeft | BottomRight`), `surface::Anchor::CornerStack(Corner)`, and private `corner_anchored_bounds(monitor: Bounds<Pixels>, corner: Corner, win: Size<Pixels>, margin: f32) -> Bounds<Pixels>` used by Task 2.

- [ ] **Step 1: Write the failing test**

Create `libs/qol-gpui/src/surface.rs`:

```rust
use gpui::*;

pub const CORNER_MARGIN: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    CornerStack(Corner),
}

#[cfg(test)]
mod tests {
    use super::{corner_anchored_bounds, Corner};
    use gpui::{point, px, size, Bounds};

    #[test]
    fn corner_anchored_bounds_places_each_corner_inside_margins() {
        let monitor = Bounds::new(point(px(1920.0), px(0.0)), size(px(2560.0), px(1440.0)));
        let win = size(px(340.0), px(76.0));
        let cases = [
            (Corner::TopLeft, (1944.0, 24.0)),
            (Corner::TopRight, (4116.0, 24.0)),
            (Corner::BottomLeft, (1944.0, 1340.0)),
            (Corner::BottomRight, (4116.0, 1340.0)),
        ];

        for (corner, expected) in cases {
            let bounds = corner_anchored_bounds(monitor, corner, win, 24.0);
            assert_eq!(
                (
                    bounds.origin.x.to_f64() as f32,
                    bounds.origin.y.to_f64() as f32
                ),
                expected,
                "corner: {corner:?}"
            );
        }
    }

    #[test]
    fn corner_anchored_bounds_supports_negative_origins_and_tiny_monitors() {
        let win = size(px(340.0), px(76.0));

        let negative = corner_anchored_bounds(
            Bounds::new(point(px(-1920.0), px(-200.0)), size(px(1920.0), px(1080.0))),
            Corner::BottomRight,
            win,
            24.0,
        );
        assert_eq!(negative.origin.x.to_f64(), -364.0);
        assert_eq!(negative.origin.y.to_f64(), 780.0);

        let tiny = corner_anchored_bounds(
            Bounds::new(point(px(0.0), px(0.0)), size(px(200.0), px(50.0))),
            Corner::BottomRight,
            win,
            24.0,
        );
        assert_eq!(tiny.origin.x.to_f64(), 24.0);
        assert_eq!(tiny.origin.y.to_f64(), 24.0);
    }
}
```

Add to `libs/qol-gpui/src/lib.rs` after `pub mod scroll_list;`:

```rust
pub mod surface;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p qol-gpui surface -- --nocapture`
Expected: FAIL to compile with "cannot find function `corner_anchored_bounds`".

- [ ] **Step 3: Write minimal implementation**

Add above the `#[cfg(test)]` module in `surface.rs`:

```rust
fn corner_anchored_bounds(
    monitor: Bounds<Pixels>,
    corner: Corner,
    win: Size<Pixels>,
    margin: f32,
) -> Bounds<Pixels> {
    let min_x = monitor.origin.x.to_f64() as f32 + margin;
    let max_x = ((monitor.origin.x + monitor.size.width - win.width).to_f64() as f32 - margin)
        .max(min_x);
    let min_y = monitor.origin.y.to_f64() as f32 + margin;
    let max_y = ((monitor.origin.y + monitor.size.height - win.height).to_f64() as f32 - margin)
        .max(min_y);
    let x = match corner {
        Corner::TopLeft | Corner::BottomLeft => min_x,
        Corner::TopRight | Corner::BottomRight => max_x,
    };
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => min_y,
        Corner::BottomLeft | Corner::BottomRight => max_y,
    };
    Bounds::new(point(px(x), px(y)), win)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p qol-gpui surface -- --nocapture`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add libs/qol-gpui/src/surface.rs libs/qol-gpui/src/lib.rs
git commit -m "feat(qol-gpui): add toast surface placement math" -- libs/qol-gpui/src/surface.rs libs/qol-gpui/src/lib.rs
```

---

### Task 2: Surface builder with toast window and timeout dismiss

**Files:**
- Modify: `libs/qol-gpui/src/surface.rs`
- Modify: `libs/qol-gpui/src/window.rs` (make `display_id_for_monitor` at line 284 `pub`)

**Interfaces:**
- Consumes: `corner_anchored_bounds`, `Corner`, `Anchor` from Task 1; `crate::monitor::MonitorTracker::snapshot_cursor()`; `crate::window::display_id_for_monitor(Option<&ActiveMonitor>, &App) -> Option<DisplayId>`.
- Produces (used by Task 5):
  - `SurfaceKind` (enum: `Toast`)
  - `Surface::new(SurfaceKind) -> Surface`, `.title(impl Into<String>)`, `.anchor(Anchor)`, `.timeout(Duration)`, `.size(Size<Pixels>)`
  - `Surface::show<V: Render + 'static>(self, tracker: &MonitorTracker, cx: &mut App, build: impl FnOnce(SurfaceDismisser, &mut Window, &mut Context<V>) -> V + 'static) -> anyhow::Result<SurfaceDismisser>`
  - `SurfaceDismisser: Clone` with `pub fn dismiss(&self, cx: &mut App)`

No unit test cycle for this task: window creation cannot be exercised on the host session (see Global Constraints), and the placement/guard logic is already covered by Task 1 tests plus the generation pattern proven in `capture_status.rs`. The compile gate plus Task 6 guest verification cover it.

- [ ] **Step 1: Make the display-id helper public**

In `libs/qol-gpui/src/window.rs` change:

```rust
fn display_id_for_monitor(monitor: Option<&ActiveMonitor>, cx: &App) -> Option<DisplayId> {
```

to:

```rust
pub fn display_id_for_monitor(monitor: Option<&ActiveMonitor>, cx: &App) -> Option<DisplayId> {
```

- [ ] **Step 2: Implement the builder**

Add to `libs/qol-gpui/src/surface.rs` (below the `Anchor` enum, above `corner_anchored_bounds`), and extend the imports at the top of the file:

```rust
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use anyhow::{anyhow, Result};

use crate::monitor::MonitorTracker;
```

```rust
pub enum SurfaceKind {
    Toast,
}

pub struct Surface {
    kind: SurfaceKind,
    title: String,
    anchor: Anchor,
    timeout: Option<Duration>,
    size: Size<Pixels>,
}

struct DismissState {
    close: RefCell<Option<Box<dyn Fn(&mut App)>>>,
    generation: Cell<u64>,
}

#[derive(Clone)]
pub struct SurfaceDismisser {
    state: Rc<DismissState>,
}

impl SurfaceDismisser {
    fn new() -> Self {
        Self {
            state: Rc::new(DismissState {
                close: RefCell::new(None),
                generation: Cell::new(0),
            }),
        }
    }

    pub fn dismiss(&self, cx: &mut App) {
        self.state
            .generation
            .set(self.state.generation.get().wrapping_add(1));
        if let Some(close) = self.state.close.borrow_mut().take() {
            close(cx);
        }
    }
}

impl Surface {
    pub fn new(kind: SurfaceKind) -> Self {
        Self {
            kind,
            title: "qol-surface".into(),
            anchor: Anchor::CornerStack(Corner::BottomRight),
            timeout: None,
            size: size(px(320.0), px(72.0)),
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn size(mut self, size: Size<Pixels>) -> Self {
        self.size = size;
        self
    }

    pub fn show<V: Render + 'static>(
        self,
        tracker: &MonitorTracker,
        cx: &mut App,
        build: impl FnOnce(SurfaceDismisser, &mut Window, &mut Context<V>) -> V + 'static,
    ) -> Result<SurfaceDismisser> {
        let monitor = tracker
            .snapshot_cursor()
            .map(|(monitor, _)| monitor)
            .ok_or_else(|| anyhow!("no monitor state available for surface placement"))?;
        let Anchor::CornerStack(corner) = self.anchor;
        let bounds = corner_anchored_bounds(monitor.bounds(), corner, self.size, CORNER_MARGIN);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id: crate::window::display_id_for_monitor(Some(&monitor), cx),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: self.window_kind(),
            focus: false,
            is_movable: true,
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some(self.title.clone()),
            ..Default::default()
        };
        let dismisser = SurfaceDismisser::new();
        let build_dismisser = dismisser.clone();
        let title = self.title.clone();
        let handle = cx.open_window(options, move |window, cx| {
            window.set_window_title(&title);
            cx.new(|cx| build(build_dismisser, window, cx))
        })?;
        dismisser
            .state
            .close
            .borrow_mut()
            .replace(Box::new(move |cx: &mut App| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }));
        if let Some(timeout) = self.timeout {
            schedule_dismiss(dismisser.clone(), timeout, cx);
        }
        Ok(dismisser)
    }

    fn window_kind(&self) -> WindowKind {
        match self.kind {
            SurfaceKind::Toast => WindowKind::PopUp,
        }
    }
}

fn schedule_dismiss(dismisser: SurfaceDismisser, timeout: Duration, cx: &mut App) {
    let scheduled = dismisser.state.generation.get();
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor().timer(timeout).await;
        if dismisser.state.generation.get() != scheduled {
            return;
        }
        let _ = cx.update(|cx| dismisser.dismiss(cx));
    })
    .detach();
}
```

Notes locked in by prior debugging (do not "fix" these):
- `WindowKind::PopUp` is correct for Toast: on Linux it maps to a non-focusable NOTIFICATION window, which is the desired never-steal-focus behavior. Mouse clicks still arrive. Interactive focus-taking kinds would need `Normal`; none exist yet.
- `is_movable: true` stays, even though the toast never moves: `is_movable: false` makes Muffin refuse all geometry operations.

- [ ] **Step 3: Verify it compiles and existing tests pass**

Run: `cargo test -p qol-gpui`
Expected: PASS, including the Task 1 placement tests.

- [ ] **Step 4: Commit**

```bash
git add libs/qol-gpui/src/surface.rs libs/qol-gpui/src/window.rs
git commit -m "feat(qol-gpui): add Surface builder with toast window and timeout dismiss" -- libs/qol-gpui/src/surface.rs libs/qol-gpui/src/window.rs
```

---

### Task 3: qol-shot saved_feedback config

**Files:**
- Modify: `plugins/qol-shot/src/config.rs`
- Modify: `plugins/qol-shot/qol-config.toml`

**Interfaces:**
- Produces (used by Task 5): `config::SavedFeedback` (enum: `Notification` default, `Toast`) at `config.capture.saved_feedback`.

- [ ] **Step 1: Write the failing test assertion**

In `plugins/qol-shot/src/config.rs`, extend `contract_defaults_match_runtime_fallbacks` with:

```rust
        assert_eq!(
            defaults.capture.saved_feedback,
            SavedFeedback::Notification
        );
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p qol-shot contract_defaults_match_runtime_fallbacks`
Expected: FAIL to compile with "cannot find type `SavedFeedback`".

- [ ] **Step 3: Implement config field and contract**

In `plugins/qol-shot/src/config.rs` add below the `CopyCommand` enum:

```rust
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SavedFeedback {
    #[default]
    Notification,
    Toast,
}
```

In `CaptureConfig` add after `open_folder_after_save`:

```rust
    #[serde(default)]
    pub saved_feedback: SavedFeedback,
```

And in `impl Default for CaptureConfig` add:

```rust
            saved_feedback: SavedFeedback::default(),
```

In `plugins/qol-shot/qol-config.toml` add after the `capture_open_folder_after_save` field:

```toml
[field.capture_saved_feedback]
type = "select"
config_key = "capture.saved_feedback"
label = "Saved Feedback"
description = "How the saved confirmation appears after a screenshot. Toast shows a clickable QoL popup instead of a system notification."
section = "capture"
default = "notification"
options = ["notification", "toast"]

[field.capture_saved_feedback.option_labels]
notification = "System Notification"
toast = "QoL Toast"
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p qol-shot contract`
Expected: PASS, including `validate_qol_contracts` (it revalidates the edited TOML) and the defaults test.

- [ ] **Step 5: Commit**

```bash
git add plugins/qol-shot/src/config.rs plugins/qol-shot/qol-config.toml
git commit -m "feat(qol-shot): add saved_feedback config select" -- plugins/qol-shot/src/config.rs plugins/qol-shot/qol-config.toml
```

---

### Task 4: Extract SavedAnnouncement from the notification path

**Files:**
- Modify: `plugins/qol-shot/src/completion.rs`

**Interfaces:**
- Produces (used by Task 5):
  - `completion::SavedAnnouncement` (Clone) with fields `title: &'static str`, `message: String`, `target: RevealTarget`, `open_automatically: bool`, and method `reveal_automatically(&self)`
  - `PreviewCompletion::announce(&self) -> Option<SavedAnnouncement>` (one-shot; second call returns `None`)
  - `RevealSource::Toast`
- Consumes: existing `RevealTarget`, `PreviewLifecycle::announce`, `file_label`.

- [ ] **Step 1: Refactor announce_saved through announce()**

In `plugins/qol-shot/src/completion.rs`:

Add a `Toast` variant to `RevealSource` (not cfg-gated; the toast exists on every platform the daemon runs on) and its label:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RevealSource {
    Automatic,
    #[cfg(target_os = "linux")]
    Notification,
    PreviewAction,
    Toast,
}

impl RevealSource {
    fn label(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            #[cfg(target_os = "linux")]
            Self::Notification => "notification",
            Self::PreviewAction => "preview-action",
            Self::Toast => "toast",
        }
    }
}
```

Add above `impl PreviewCompletion`:

```rust
#[derive(Clone)]
pub(crate) struct SavedAnnouncement {
    pub(crate) title: &'static str,
    pub(crate) message: String,
    pub(crate) target: RevealTarget,
    pub(crate) open_automatically: bool,
}

impl SavedAnnouncement {
    pub(crate) fn reveal_automatically(&self) {
        if let Err(error) = self.target.open(RevealSource::Automatic) {
            eprintln!("[qol-shot] automatic folder reveal failed: {error:#}");
        }
    }
}
```

Replace `announce_saved` and remove the now-unneeded private `open_automatically` (keep it if `finish` still uses it; `finish` does, so keep it):

```rust
    pub(crate) fn announce(&self) -> Option<SavedAnnouncement> {
        let open_automatically = self.lifecycle.announce()?;
        Some(SavedAnnouncement {
            title: "Screenshot saved",
            message: file_label(self.target.path()),
            target: self.target.clone(),
            open_automatically,
        })
    }

    pub(crate) fn announce_saved(&self) {
        let Some(announcement) = self.announce() else {
            return;
        };
        crate::platform::show_saved_notification(
            announcement.title,
            &announcement.message,
            8_000,
            announcement.target.clone(),
        );
        if announcement.open_automatically {
            announcement.reveal_automatically();
        }
    }
```

- [ ] **Step 2: Verify behavior is unchanged**

Run: `cargo test -p qol-shot completion`
Expected: PASS (all existing lifecycle tests; they pin the announce/finish ordering this refactor must preserve).

Note: `RevealSource::Toast` and `announce` are consumed in Task 5; between these commits `cargo build -p qol-shot` may warn dead_code. Verify with `cargo test -p qol-shot` only, and land Task 5 before the Task 6 `-D warnings` gates.

- [ ] **Step 3: Commit**

```bash
git add plugins/qol-shot/src/completion.rs
git commit -m "refactor(qol-shot): extract saved announcement from notification path" -- plugins/qol-shot/src/completion.rs
```

---

### Task 5: Saved toast view and daemon wiring

**Files:**
- Create: `plugins/qol-shot/src/saved_toast.rs`
- Modify: `plugins/qol-shot/src/lib.rs` (add `mod saved_toast;` alongside the existing module list)
- Modify: `plugins/qol-shot/src/daemon_app.rs`

**Interfaces:**
- Consumes: `Surface`/`SurfaceKind::Toast`/`Anchor::CornerStack(Corner::BottomRight)`/`SurfaceDismisser` (Task 2), `SavedFeedback` (Task 3), `SavedAnnouncement`/`announce`/`RevealSource::Toast` (Task 4), `qol_gpui::theme::{shot_preview_runtime, ShotPreviewPalette}`.
- Produces: `saved_toast::show(announcement: SavedAnnouncement, tracker: &MonitorTracker, cx: &mut App) -> anyhow::Result<()>`.

- [ ] **Step 1: Implement the toast view**

Create `plugins/qol-shot/src/saved_toast.rs`:

```rust
use std::time::Duration;

use gpui::*;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::surface::{Anchor, Corner, Surface, SurfaceDismisser, SurfaceKind};
use qol_gpui::theme::{shot_preview_runtime, ShotPreviewPalette};

use crate::completion::{RevealSource, SavedAnnouncement};

const TOAST_WIDTH: f32 = 340.0;
const TOAST_HEIGHT: f32 = 76.0;
const TOAST_TIMEOUT_MS: u64 = 8_000;

pub(crate) fn show(
    announcement: SavedAnnouncement,
    tracker: &MonitorTracker,
    cx: &mut App,
) -> anyhow::Result<()> {
    let title = format!("qol-shot-toast-{}", std::process::id());
    Surface::new(SurfaceKind::Toast)
        .title(title)
        .anchor(Anchor::CornerStack(Corner::BottomRight))
        .size(size(px(TOAST_WIDTH), px(TOAST_HEIGHT)))
        .timeout(Duration::from_millis(TOAST_TIMEOUT_MS))
        .show(tracker, cx, move |dismisser, _window, _cx| SavedToastView {
            announcement,
            dismisser,
            palette: shot_preview_runtime(),
        })
        .map(|_| ())
}

struct SavedToastView {
    announcement: SavedAnnouncement,
    dismisser: SurfaceDismisser,
    palette: ShotPreviewPalette,
}

impl Render for SavedToastView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let target = self.announcement.target.clone();
        let dismisser = self.dismisser.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_center()
            .gap_1()
            .px_4()
            .rounded_xl()
            .border_1()
            .border_color(rgb(self.palette.thumb_border))
            .bg(rgb(self.palette.window_bg))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, _event, _window, cx| {
                    if let Err(error) = target.open(RevealSource::Toast) {
                        eprintln!("[qol-shot] toast reveal failed: {error:#}");
                    }
                    dismisser.dismiss(cx);
                }),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(self.palette.label_text))
                    .child(self.announcement.title),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.label_text))
                    .child(self.announcement.message.clone()),
            )
    }
}
```

In `plugins/qol-shot/src/lib.rs` add `mod saved_toast;` in alphabetical position among the existing `mod` declarations.

- [ ] **Step 2: Wire the daemon feedback dispatch**

In `plugins/qol-shot/src/daemon_app.rs`:

In `capture_and_preview`, pass the tracker to the completion future. Change:

```rust
    let presented = present(cx, state, capture);
    let status = state.capture_status.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        complete_screenshot(path, file_ready, completion, presented, status, cx).await;
    })
    .detach();
```

to:

```rust
    let presented = present(cx, state, capture);
    let status = state.capture_status.clone();
    let tracker = state.tracker.clone();
    cx.spawn(async move |cx: &mut AsyncApp| {
        complete_screenshot(path, file_ready, completion, presented, status, tracker, cx).await;
    })
    .detach();
```

Change `complete_screenshot`'s signature and announcement call from:

```rust
async fn complete_screenshot(
    path: std::path::PathBuf,
    file_ready: crate::screenshot::CaptureFileReady,
    completion: Option<crate::completion::PreviewCompletion>,
    presented: bool,
    status: crate::capture_status::CaptureStatusUi,
    cx: &mut AsyncApp,
) {
```

to:

```rust
async fn complete_screenshot(
    path: std::path::PathBuf,
    file_ready: crate::screenshot::CaptureFileReady,
    completion: Option<crate::completion::PreviewCompletion>,
    presented: bool,
    status: crate::capture_status::CaptureStatusUi,
    tracker: MonitorTracker,
    cx: &mut AsyncApp,
) {
```

and replace:

```rust
    if let Some(completion) = completion {
        completion.announce_saved();
        if !presented {
            completion.finish(crate::completion::PreviewExit::Unavailable);
        }
    }
```

with:

```rust
    if let Some(completion) = completion {
        announce_saved_feedback(&completion, &tracker, cx);
        if !presented {
            completion.finish(crate::completion::PreviewExit::Unavailable);
        }
    }
```

Add below `complete_screenshot`:

```rust
fn announce_saved_feedback(
    completion: &crate::completion::PreviewCompletion,
    tracker: &MonitorTracker,
    cx: &mut AsyncApp,
) {
    if crate::config::load().capture.saved_feedback == crate::config::SavedFeedback::Notification {
        completion.announce_saved();
        return;
    }
    let Some(announcement) = completion.announce() else {
        return;
    };
    let toast_announcement = announcement.clone();
    let shown = cx
        .update(|cx| crate::saved_toast::show(toast_announcement, tracker, cx))
        .unwrap_or_else(|error| Err(anyhow::anyhow!("app unavailable: {error}")));
    match shown {
        Ok(()) => qol_runtime::probe!("SHOT_SAVED_TOAST", "result=shown"),
        Err(error) => {
            qol_runtime::probe!("SHOT_SAVED_TOAST", "result=fallback error={error:#}");
            eprintln!("[qol-shot] saved toast failed, falling back to notification: {error:#}");
            crate::platform::show_saved_notification(
                announcement.title,
                &announcement.message,
                8_000,
                announcement.target.clone(),
            );
        }
    }
    if announcement.open_automatically {
        announcement.reveal_automatically();
    }
}
```

- [ ] **Step 3: Build and run the full qol-shot suite**

Run: `cargo test -p qol-shot`
Expected: PASS. Then `cargo build -p qol-shot` with no warnings.

- [ ] **Step 4: Commit**

```bash
git add plugins/qol-shot/src/saved_toast.rs plugins/qol-shot/src/lib.rs plugins/qol-shot/src/daemon_app.rs
git commit -m "feat(qol-shot): show saved toast surface when configured" -- plugins/qol-shot/src/saved_toast.rs plugins/qol-shot/src/lib.rs plugins/qol-shot/src/daemon_app.rs
```

---

### Task 6: Full verification

**Files:** none (verification only).

- [ ] **Step 1: Workspace gates with real output**

Run each and confirm clean output before claiming done:

```bash
cargo fmt -p qol-gpui -p qol-shot -- --check
cargo clippy -p qol-gpui -p qol-shot --all-targets -- -D warnings
cargo test -p qol-gpui -p qol-shot
RUSTFLAGS="-D warnings" cargo check -p qol-shot --release
```

The release check catches debug-only probe locals (release builds are warning-fatal in CI).

- [ ] **Step 2: Runtime verification in a guest VM**

Host sessions must not be used for popup verification. In a `qol env up <environment> --dev-worktree <worktree>` guest:

1. Set qol-shot config `capture.saved_feedback` to `toast`.
2. Take a screenshot; confirm the toast appears bottom-right on the cursor monitor, over the focused app, without stealing focus (type into the previously focused window to confirm keystrokes land there).
3. Click the toast; confirm the containing folder opens exactly once and the toast dismisses.
4. Take another screenshot and let it sit; confirm the toast dismisses on its own at ~8s.
5. Repeat over a fullscreen window to check visibility.
6. Switch config back to `notification`; confirm the OS notification path still works.

Cold-first-show behavior should be checked on a fresh VM lane (first toast after daemon start), since first-show is where window quirks appear.

- [ ] **Step 3: Report**

Report results using the four-part completion structure with the real command output, listing any guest-VM deviations verbatim.
