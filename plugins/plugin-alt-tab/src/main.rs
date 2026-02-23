mod config;
mod daemon;
mod layout;
mod monitor;
mod platform;
mod window_source;

use crate::config::{load_alt_tab_config, ActionMode, AltTabConfig};
use crate::layout::*;
use crate::platform::WindowInfo;
use crate::window_source::{load_windows_with_previews, preview_tile};
use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{
    list::{ListDelegate, ListItem, ListState},
    IndexPath,
};
use monitor::MonitorTracker;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-alt-tab/";
const DEFAULT_ESTIMATED_WINDOW_COUNT: usize = 8;
const PREWARM_REFRESH_INTERVAL_MS: u64 = 1200;
const REUSE_ORIGIN_TOLERANCE_PX: f64 = 6.0;

// ─── List delegate ────────────────────────────────────────────────────────────

struct WindowDelegate {
    windows: Vec<WindowInfo>,
    selected_index: Option<IndexPath>,
    label_config: crate::config::LabelConfig,
}

impl WindowDelegate {
    fn new(windows: Vec<WindowInfo>, label_config: crate::config::LabelConfig) -> Self {
        let selected_index = if windows.is_empty() {
            None
        } else {
            Some(IndexPath::new(0))
        };
        Self {
            windows,
            selected_index,
            label_config,
        }
    }

    fn set_windows(&mut self, windows: Vec<WindowInfo>, reset_selection: bool) {
        self.windows = windows;
        if self.windows.is_empty() {
            self.selected_index = None;
            #[cfg(debug_assertions)]
            eprintln!(
                "[alt-tab/select] set_windows reset={} next=None total=0",
                reset_selection
            );
            return;
        }

        if reset_selection {
            self.selected_index = Some(IndexPath::new(0));
            #[cfg(debug_assertions)]
            eprintln!(
                "[alt-tab/select] set_windows reset={} next=Some(0) total={}",
                reset_selection,
                self.windows.len()
            );
            return;
        }

        let selected_row = self.selected_index.map(|ix| ix.row).unwrap_or(0);
        self.selected_index = Some(IndexPath::new(selected_row.min(self.windows.len() - 1)));
        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/select] set_windows reset={} next={:?} total={}",
            reset_selection,
            self.selected_index.map(|ix| ix.row),
            self.windows.len()
        );
    }
}

impl ListDelegate for WindowDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.windows.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let is_selected = self.selected_index == Some(ix);
        let win = &self.windows[ix.row];

        let item = div().flex().px_1().py_2().child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .w(px(GRID_CARD_WIDTH))
                .p_2()
                .rounded_xl()
                .when(is_selected, |s| {
                    s.bg(rgb(0x233050)).border_1().border_color(rgb(0x4a6fa5))
                })
                .when(!is_selected, |s| s.bg(rgb(0x161a25)))
                .child(div().rounded_md().overflow_hidden().child(preview_tile(
                    &win.preview_path,
                    GRID_PREVIEW_WIDTH,
                    GRID_PREVIEW_HEIGHT,
                )))
                .child(
                    div()
                        .mt_2()
                        .w_full()
                        .text_color(if is_selected {
                            rgb(0xffffff)
                        } else {
                            rgb(0x6b7890)
                        })
                        .text_xs()
                        .text_center()
                        .text_ellipsis()
                        .child(self.label_config.format(&win.app_name, &win.title)),
                ),
        );

        Some(
            ListItem::new(gpui::SharedString::from(format!(
                "window-{}-{}-{}",
                ix.row, self.label_config.show_app_name, self.label_config.show_window_title
            )))
            .child(item),
        )
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        self.selected_index = ix;
    }

    fn confirm(
        &mut self,
        _secondary: bool,
        window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
        self.activate_selected(window, None);
    }
}

impl WindowDelegate {
    fn activate_selected(&self, window: &mut Window, tracker: Option<&MonitorTracker>) {
        if let Some(ix) = self.selected_index {
            let win = &self.windows[ix.row];
            platform::activate_window(win.id);
            // Immediately update the monitor focus so the next snapshot
            // reflects the newly-focused monitor without waiting for poll.
            if let Some(tracker) = tracker {
                tracker.force_focus_update();
            }
            platform::dismiss_picker(window);
        }
    }

    fn select_next(&mut self) {
        if self.windows.is_empty() {
            return;
        }
        let current = self.selected_index.map(|ix| ix.row).unwrap_or(0);
        let next = (current + 1) % self.windows.len();
        self.selected_index = Some(IndexPath::new(next));
    }

    fn select_prev(&mut self) {
        if self.windows.is_empty() {
            return;
        }
        let current = self.selected_index.map(|ix| ix.row).unwrap_or(0);
        let prev = if current == 0 {
            self.windows.len() - 1
        } else {
            current - 1
        };
        self.selected_index = Some(IndexPath::new(prev));
    }

    fn select_left(&mut self, columns: usize) {
        self.move_in_grid(GridDirection::Left, columns);
    }

    fn select_right(&mut self, columns: usize) {
        self.move_in_grid(GridDirection::Right, columns);
    }

    fn select_up(&mut self, columns: usize) {
        self.move_in_grid(GridDirection::Up, columns);
    }

    fn select_down(&mut self, columns: usize) {
        self.move_in_grid(GridDirection::Down, columns);
    }

    fn move_in_grid(&mut self, direction: GridDirection, columns: usize) {
        let total = self.windows.len();
        if total == 0 {
            return;
        }

        let cols = columns.max(1).min(total);
        let rows = (total + cols - 1) / cols;
        let current = self
            .selected_index
            .map(|ix| ix.row)
            .unwrap_or(0)
            .min(total.saturating_sub(1));

        let row = current / cols;
        let col = current % cols;

        let row_bounds = |r: usize| {
            let start = r * cols;
            let end = ((r + 1) * cols).min(total);
            (start, end)
        };

        let next = match direction {
            GridDirection::Left => {
                let (row_start, _) = row_bounds(row);
                if current > row_start {
                    current - 1
                } else {
                    current
                }
            }
            GridDirection::Right => {
                let (_, row_end) = row_bounds(row);
                if current + 1 < row_end {
                    current + 1
                } else {
                    current
                }
            }
            GridDirection::Up => {
                if row == 0 {
                    current
                } else {
                    let (target_start, target_end) = row_bounds(row - 1);
                    target_start + col.min(target_end - target_start - 1)
                }
            }
            GridDirection::Down => {
                if row + 1 >= rows {
                    current
                } else {
                    let (target_start, target_end) = row_bounds(row + 1);
                    target_start + col.min(target_end - target_start - 1)
                }
            }
        };

        self.selected_index = Some(IndexPath::new(next));
    }
}

enum GridDirection {
    Left,
    Right,
    Up,
    Down,
}

// ─── App view ─────────────────────────────────────────────────────────────────

struct AltTabApp {
    list_state: Entity<ListState<WindowDelegate>>,
    focus_handle: FocusHandle,
    action_mode: ActionMode,
    alt_was_held: bool,
    _alt_poll_task: Option<gpui::Task<()>>,
}

impl AltTabApp {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        last_window_count: Arc<AtomicUsize>,
        window_cache: Arc<std::sync::Mutex<Vec<WindowInfo>>>,
        action_mode: ActionMode,
        initial_windows: Vec<WindowInfo>,
        label_config: crate::config::LabelConfig,
    ) -> Self {
        let has_cached_windows = !initial_windows.is_empty();
        let delegate = WindowDelegate::new(initial_windows.clone(), label_config);
        let list_state = cx.new(|cx| ListState::new(delegate, window, cx));

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        let mut async_window = window.to_async(cx);
        let gpui_window_handle = async_window.window_handle();

        // Register the focus out subscription. We need this to exist for Sticky mode.
        // We evaluate action_mode dynamically inside the callback, so if the config
        // changes to HoldToSwitch during reuse, it stops auto-minimizing.
        let focus_handle_for_sub = focus_handle.clone();
        let _focus_out_subscription = cx.on_focus_out(
            &focus_handle_for_sub,
            window,
            |this, _event, window, _cx| {
                if this.action_mode != ActionMode::HoldToSwitch {
                    platform::dismiss_picker(window);
                }
            },
        );

        let list_state_clone = list_state.clone();
        cx.spawn(
            move |this: gpui::WeakEntity<AltTabApp>, cx: &mut gpui::AsyncApp| {
                let cx = cx.clone();
                async move {
                    let executor = cx.background_executor().clone();
                    if has_cached_windows {
                        return;
                    }
                    let cached = window_cache
                        .lock()
                        .map(|cache| cache.clone())
                        .unwrap_or_default();
                    let windows = load_windows_with_previews(&executor, cached, false).await;
                    if let Ok(mut cache) = window_cache.lock() {
                        *cache = windows.clone();
                    }
                    let missing_targets: Vec<(usize, u32)> = windows
                        .iter()
                        .enumerate()
                        .filter(|(_, win)| win.preview_path.is_none())
                        .map(|(i, win)| (i, win.id))
                        .collect();
                    let ids: Vec<u32> = windows.iter().map(|w| w.id).collect();
                    last_window_count.store(ids.len().max(1), Ordering::Relaxed);

                    let _ = cx.update(|app_cx| {
                        let _ = list_state_clone.update(app_cx, |state, cx| {
                            state.delegate_mut().set_windows(windows.clone(), false);
                            cx.notify();
                        });
                        let _ = this.update(app_cx, |_, cx: &mut gpui::Context<AltTabApp>| {
                            cx.notify();
                        });
                    });
                    let total = ids.len().max(1);
                    let _ = async_window.update(|window, _app_cx| {
                        let cols = rendered_column_count(window, total);
                        let next_height = picker_height_for(total, cols).clamp(320.0, 980.0);
                        let current = window.window_bounds().get_bounds().size;
                        if (current.height.to_f64() - next_height as f64).abs() >= 1.0 {
                            window.resize(size(current.width, px(next_height)));
                        }
                    });

                    let (tx, rx) = mpsc::channel::<(usize, Option<String>)>();
                    for (i, id) in missing_targets {
                        let tx = tx.clone();
                        executor
                            .spawn(async move {
                                let path = platform::capture_preview(
                                    id,
                                    PREVIEW_MAX_WIDTH,
                                    PREVIEW_MAX_HEIGHT,
                                );
                                let _ = tx.send((i, path));
                            })
                            .detach();
                    }
                    drop(tx);

                    let rx = Arc::new(std::sync::Mutex::new(rx));
                    loop {
                        let rx = rx.clone();
                        let next = executor
                            .spawn(async move { rx.lock().ok()?.recv().ok() })
                            .await;

                        let Some((i, path_opt)) = next else {
                            break;
                        };

                        if let Some(path) = path_opt {
                            if let Ok(mut cache) = window_cache.lock() {
                                if i < cache.len() {
                                    cache[i].preview_path = Some(path.clone());
                                }
                            }
                            let _ = cx.update(|app_cx| {
                                let _ = list_state_clone.update(app_cx, |state, cx| {
                                    if i < state.delegate().windows.len() {
                                        state.delegate_mut().windows[i].preview_path = Some(path);
                                        cx.notify();
                                    }
                                });
                                let _ =
                                    this.update(app_cx, |_, cx: &mut gpui::Context<AltTabApp>| {
                                        cx.notify();
                                    });
                            });
                        }
                    }
                }
            },
        )
        .detach();

        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/hold] AltTabApp::new: action_mode={:?}, alt_was_held=true (assumed)",
            action_mode
        );

        let mut app = Self {
            list_state,
            focus_handle,
            action_mode: action_mode.clone(),
            alt_was_held: true,
            _alt_poll_task: None,
        };

        if action_mode == ActionMode::HoldToSwitch {
            app.start_alt_poll(gpui_window_handle, cx);
        }

        app
    }

    fn apply_cached_windows(
        &mut self,
        windows: Vec<WindowInfo>,
        reset_selection: bool,
        cx: &mut Context<Self>,
    ) {
        self.list_state.update(cx, |state, cx| {
            state.delegate_mut().set_windows(windows, reset_selection);
            cx.notify();
        });
        cx.notify();
    }

    /// Start a new X11 Alt-key polling task for HoldToSwitch mode.
    /// Drops any previous task (which auto-cancels it).
    fn start_alt_poll(
        &mut self,
        window_handle: gpui::AnyWindowHandle,
        cx: &mut gpui::Context<Self>,
    ) {
        let list = self.list_state.clone();
        self.alt_was_held = true;
        self._alt_poll_task = Some(cx.spawn(
            move |this: gpui::WeakEntity<AltTabApp>, cx: &mut gpui::AsyncApp| {
                let mut cx = cx.clone();
                async move {
                    eprintln!("[alt-tab/hold] X11 modifier poll task started");
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(50))
                        .await;
                    loop {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(16))
                            .await;
                        let alt_held = platform::is_modifier_held();

                        if !alt_held {
                            eprintln!(
                                "[alt-tab/hold] X11 poll: Alt released — activating selected"
                            );
                            let list = list.clone();
                            let _ = cx.update_window(window_handle, |_root, window, cx| {
                                list.update(cx, |s, _cx| {
                                    s.delegate_mut().activate_selected(window, None);
                                });
                            });
                            break;
                        }
                    }

                    // Clear the task reference so subsequent Show requests know we're fully closed
                    let _ = cx.update(|cx| {
                        if let Some(entity) = this.upgrade() {
                            let _ = entity.update(cx, |app: &mut AltTabApp, _cx| {
                                app._alt_poll_task = None;
                            });
                        }
                    });

                    eprintln!("[alt-tab/hold] X11 modifier poll task ended");
                }
            },
        ));
    }

}

impl Focusable for AltTabApp {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AltTabApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list_state = self.list_state.clone();

        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/render] action_mode={:?} alt_was_held={}",
            self.action_mode, self.alt_was_held
        );

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .bg(rgb(0x0f111a))
            .w_full()
            .h_full()
            // HoldToSwitch modifier detection is handled by the X11 poll task,
            // not by GPUI events (which don't fire due to global hotkey grab).
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" | "esc" => platform::dismiss_picker(window),
                    "enter" => {
                        // Activate selected and close
                        let win_id =
                            this.list_state
                                .read(cx)
                                .delegate()
                                .selected_index
                                .and_then(|ix| {
                                    this.list_state
                                        .read(cx)
                                        .delegate()
                                        .windows
                                        .get(ix.row)
                                        .map(|w| w.id)
                                });
                        if let Some(_id) = win_id {
                            this.list_state.update(cx, |s, _cx| {
                                s.delegate_mut().activate_selected(window, None);
                            });
                        }
                    }
                    // Navigate — do NOT call s.focus() after, that would steal focus
                    // from our root div and break further key events.
                    "tab" => {
                        this.list_state.update(cx, |s, _cx| {
                            if event.keystroke.modifiers.shift {
                                s.delegate_mut().select_prev();
                            } else {
                                s.delegate_mut().select_next();
                            }
                        });
                        cx.notify();
                    }
                    "backtab" => {
                        this.list_state.update(cx, |s, _cx| {
                            s.delegate_mut().select_prev();
                        });
                        cx.notify();
                    }
                    "right" | "arrowright" => {
                        let total = this.list_state.read(cx).delegate().windows.len();
                        let cols = rendered_column_count(window, total);
                        this.list_state.update(cx, |s, _cx| {
                            s.delegate_mut().select_right(cols);
                        });
                        cx.notify();
                    }
                    "left" | "arrowleft" => {
                        let total = this.list_state.read(cx).delegate().windows.len();
                        let cols = rendered_column_count(window, total);
                        this.list_state.update(cx, |s, _cx| {
                            s.delegate_mut().select_left(cols);
                        });
                        cx.notify();
                    }
                    "down" | "arrowdown" => {
                        let total = this.list_state.read(cx).delegate().windows.len();
                        let cols = rendered_column_count(window, total);
                        this.list_state.update(cx, |s, _cx| {
                            s.delegate_mut().select_down(cols);
                        });
                        cx.notify();
                    }
                    "up" | "arrowup" => {
                        let total = this.list_state.read(cx).delegate().windows.len();
                        let cols = rendered_column_count(window, total);
                        this.list_state.update(cx, |s, _cx| {
                            s.delegate_mut().select_up(cols);
                        });
                        cx.notify();
                    }
                    _ => {}
                }
            }))
            .child(
                // ── Header bar ────────────────────────────────────────────────
                div()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(0x1e2333))
                    .bg(rgb(0x13151f))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_color(rgb(0x5e6a84))
                            .text_xs()
                            .child("Alt Tab  ·  Live Window Grid"),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x3a4252))
                            .text_xs()
                            .child("↑↓←→ navigate  ·  ⏎ switch  ·  esc close"),
                    ),
            )
            .child(
                // ── Content ───────────────────────────────────────────────────
                div().flex_1().w_full().min_h_0().child({
                    let delegate = list_state.read(cx).delegate();
                    let windows = delegate.windows.clone();
                    let selected_index = delegate.selected_index;
                    let label_config = delegate.label_config.clone();
                    let _ = delegate;

                    let entity = cx.weak_entity();
                    div()
                        .id("preview-grid")
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .content_start()
                        .w_full()
                        .h_full()
                        .overflow_y_scroll()
                        .px_5()
                        .py_4()
                        .gap_3()
                        .when(windows.is_empty(), |s| {
                            s.items_center().justify_center().child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x5e6a84))
                                    .child("Scanning windows..."),
                            )
                        })
                        .children(windows.into_iter().enumerate().map(|(i, win)| {
                            let is_selected = selected_index == Some(IndexPath::new(i));
                            let entity_for_click = entity.clone();
                            div()
                                .id(ElementId::Integer(i as u64))
                                .flex()
                                .flex_col()
                                .items_center()
                                .w(px(GRID_CARD_WIDTH))
                                .p_2()
                                .rounded_xl()
                                .cursor_pointer()
                                .on_click(move |_ev: &gpui::ClickEvent, window, cx| {
                                    let window_id = entity_for_click
                                        .update(cx, |this, cx| {
                                            this.list_state.update(cx, |s, _cx| {
                                                s.delegate_mut().selected_index =
                                                    Some(IndexPath::new(i));
                                            });
                                            this.list_state
                                                .read(cx)
                                                .delegate()
                                                .windows
                                                .get(i)
                                                .map(|w| w.id)
                                        })
                                        .ok()
                                        .flatten();
                                    if let Some(_id) = window_id {
                                        entity_for_click
                                            .update(cx, |this, cx| {
                                                this.list_state.update(cx, |s, _cx| {
                                                    s.delegate_mut()
                                                        .activate_selected(window, None);
                                                });
                                            })
                                            .ok();
                                    }
                                })
                                .when(is_selected, |s| {
                                    s.bg(rgb(0x233050)).border_1().border_color(rgb(0x4a6fa5))
                                })
                                .when(!is_selected, |s| {
                                    s.bg(rgb(0x1a1e2a)).hover(|mut h| {
                                        h.background = Some(rgb(0x1e2640).into());
                                        h
                                    })
                                })
                                .child(div().rounded_md().overflow_hidden().child(preview_tile(
                                    &win.preview_path,
                                    GRID_PREVIEW_WIDTH,
                                    GRID_PREVIEW_HEIGHT,
                                )))
                                .child(
                                    div()
                                        .mt_2()
                                        .w_full()
                                        .text_color(if is_selected {
                                            rgb(0xffffff)
                                        } else {
                                            rgb(0x7a849e)
                                        })
                                        .text_xs()
                                        .text_center()
                                        .text_ellipsis()
                                        .overflow_hidden()
                                        .child({
                                            let label = label_config.format(&win.app_name, &win.title);
                                            #[cfg(debug_assertions)]
                                            {
                                                format!("[{}] {}", i, label)
                                            }
                                            #[cfg(not(debug_assertions))]
                                            {
                                                label
                                            }
                                        }),
                                )
                        }))
                }),
            )
    }
}

// ─── Run / daemon ─────────────────────────────────────────────────────────────

fn open_keepalive(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1.0), px(1.0)), cx);
    let _ = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: WindowKind::PopUp,
            focus: false,
            show: false,
            ..Default::default()
        },
        |_window, cx| cx.new(|_cx| KeepAlive),
    );
}

struct KeepAlive;
impl Render for KeepAlive {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn open_picker(
    config: &AltTabConfig,
    current: &std::rc::Rc<std::cell::RefCell<Option<WindowHandle<AltTabApp>>>>,
    tracker: &MonitorTracker,
    last_window_count: Arc<AtomicUsize>,
    window_cache: Arc<std::sync::Mutex<Vec<WindowInfo>>>,
    reverse: bool,
    cx: &mut App,
) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/open] show request (reverse={})", reverse);

    // Reverse only cycles within an already-open picker — never opens one.
    if reverse && current.borrow().is_none() {
        return;
    }

    let raw_windows = platform::get_open_windows();
    let mut display_windows = raw_windows;
    if let Ok(cache) = window_cache.lock() {
        for win in &mut display_windows {
            if let Some(cached) = cache.iter().find(|c| c.id == win.id) {
                win.preview_path = cached.preview_path.clone();
            }
        }
    }
    // Update the cache centrally so background processes see the current layout
    if let Ok(mut cache) = window_cache.lock() {
        *cache = display_windows.clone();
    }

    // Fast path: reuse existing picker window instead of recreating it.
    let existing_handle = current.borrow().clone();
    if let Some(handle) = existing_handle {
        let target_count = display_windows.len().max(1);
        let (target_w, target_h) = picker_dimensions(target_count, config.display.max_columns);
        let target_size = size(px(target_w), px(target_h));
        let target_bounds = if let Some(active) = tracker.snapshot() {
            active.centered_bounds(target_size)
        } else {
            Bounds::centered(None, target_size, cx)
        };
        if handle
            .update(cx, |view, window: &mut Window, cx| {
                // In HoldToSwitch mode, if the poll task is still running, the window
                // is already visible. Treat subsequent Show commands as "select next".
                let alt_held = platform::is_modifier_held();
                if view.action_mode == ActionMode::HoldToSwitch && view._alt_poll_task.is_some() && alt_held {
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "[alt-tab/hold] window already visible (alt_held={} reverse={}) — cycling",
                        alt_held, reverse
                    );
                    view.list_state.update(cx, |s, _cx| {
                        if reverse {
                            s.delegate_mut().select_prev();
                        } else {
                            s.delegate_mut().select_next();
                        }
                    });
                    cx.notify();
                    return;
                }
                #[cfg(debug_assertions)]
                eprintln!(
                    "[alt-tab/hold] reuse path (alt_held={} poll_task={}) — applying config reset={}",
                    alt_held,
                    view._alt_poll_task.is_some(),
                    config.reset_selection_on_open
                );

                let current_bounds = window.window_bounds().get_bounds();
                let dx = (current_bounds.origin.x.to_f64() - target_bounds.origin.x.to_f64()).abs();
                let dy = (current_bounds.origin.y.to_f64() - target_bounds.origin.y.to_f64()).abs();
                if dx > REUSE_ORIGIN_TOLERANCE_PX || dy > REUSE_ORIGIN_TOLERANCE_PX
                {
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "[alt-tab/open] moving window natively via X11: current_origin={:?} target_origin={:?} dx={:.1} dy={:.1}",
                        current_bounds.origin, target_bounds.origin, dx, dy
                    );
                    let x = target_bounds.origin.x.to_f64() as i32;
                    let y = target_bounds.origin.y.to_f64() as i32;
                    platform::move_app_window("qol-alt-tab-picker", x, y);
                }

                #[cfg(debug_assertions)]
                eprintln!("[alt-tab/hold] open_picker reusing window: setting action_mode={:?}", config.action_mode);

                view.action_mode = config.action_mode.clone();
                view.alt_was_held = true;

                // Update label config in delegate
                view.list_state.update(cx, |s, _cx| {
                    s.delegate_mut().label_config = config.label.clone();
                });

                // Start a new X11 modifier polling task for HoldToSwitch
                if config.action_mode == ActionMode::HoldToSwitch {
                    let wh = window.window_handle();
                    view.start_alt_poll(wh, cx);
                } else {
                    view._alt_poll_task = None; // Cancel any existing poll
                }

                view.apply_cached_windows(display_windows.clone(), config.reset_selection_on_open, cx);

                let current_size = current_bounds.size;
                let next_size = target_size;
                if (current_size.width.to_f64() - target_w as f64).abs() >= 1.0
                    || (current_size.height.to_f64() - target_h as f64).abs() >= 1.0
                {
                    window.resize(next_size);
                }
                window.focus(&view.focus_handle(cx));
                window.activate_window();
            })
            .is_ok()
        {
            #[cfg(debug_assertions)]
            eprintln!("[alt-tab/open] reused existing picker window");
            cx.activate(true);
            return;
        }

        *current.borrow_mut() = None;
    }

    let target_count = display_windows.len().max(1);
    let estimated_count = target_count
        .max(last_window_count.load(Ordering::Relaxed))
        .max(1);
    let (win_w, win_h) = picker_dimensions(estimated_count, config.display.max_columns);
    let win_size = size(px(win_w), px(win_h));

    let bounds = if let Some(active) = tracker.snapshot() {
        active.centered_bounds(win_size)
    } else {
        Bounds::centered(None, win_size, cx)
    };

    println!(
        "[alt-tab] opening at {:?} with size {:?}",
        bounds.origin, bounds.size
    );

    let last_window_count_for_init = last_window_count.clone();
    let window_cache_for_init = window_cache.clone();
    let action_mode_for_init = config.action_mode.clone();
    let display_windows_for_init = display_windows.clone();
    let config_for_init = config.clone();

    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: platform::picker_window_kind(),
            focus: true,
            ..Default::default()
        },
        move |window, cx| {
            window.set_window_title("qol-alt-tab-picker");
            let label_config = config_for_init.label.clone();
            let view = cx.new(|cx| {
                AltTabApp::new(
                    window,
                    cx,
                    last_window_count_for_init,
                    window_cache_for_init,
                    action_mode_for_init,
                    display_windows_for_init,
                    label_config,
                )
            });
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            view
        },
    );
    if let Ok(h) = handle {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/open] opened new picker window");
        *current.borrow_mut() = Some(h);
    } else {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/open] failed to open picker window");
    }
    cx.activate(true);
}

fn run_app(config: AltTabConfig, rx: mpsc::Receiver<daemon::Command>, show_on_start: bool) {
    let app = Application::new();

    app.run(move |cx: &mut App| {
        gpui_component::init(cx);

        let any_visible = Arc::new(AtomicBool::new(false));
        let tracker = MonitorTracker::start(cx, any_visible);

        // Keepalive: prevents GPUI from quitting when the picker window is removed
        open_keepalive(cx);

        // Track the single picker window; shared with the daemon poll loop
        let current: std::rc::Rc<std::cell::RefCell<Option<WindowHandle<AltTabApp>>>> =
            std::rc::Rc::new(std::cell::RefCell::new(None));
        let last_window_count = Arc::new(AtomicUsize::new(DEFAULT_ESTIMATED_WINDOW_COUNT));
        let window_cache: Arc<std::sync::Mutex<Vec<WindowInfo>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));

        // Prewarm window + preview cache while daemon is alive so first user open is hot.
        let warm_cache = window_cache.clone();
        let warm_count = last_window_count.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let executor = cx.background_executor().clone();
            loop {
                let cached = warm_cache
                    .lock()
                    .map(|cache| cache.clone())
                    .unwrap_or_default();
                let refreshed = load_windows_with_previews(&executor, cached, false).await;
                warm_count.store(refreshed.len().max(1), Ordering::Relaxed);
                if let Ok(mut cache) = warm_cache.lock() {
                    *cache = refreshed;
                }
                executor
                    .timer(Duration::from_millis(PREWARM_REFRESH_INTERVAL_MS))
                    .await;
            }
        })
        .detach();

        if show_on_start {
            open_picker(
                &config,
                &current,
                &tracker,
                last_window_count.clone(),
                window_cache.clone(),
                false,
                cx,
            );
        }

        // Poll the daemon channel for Show/Kill commands
        let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
        let tracker_clone = tracker.clone();
        cx.spawn(async move |cx: &mut AsyncApp| loop {
            let rx2 = rx.clone();
            let cmd = cx
                .background_executor()
                .spawn(async move { rx2.lock().ok()?.recv().ok() })
                .await;

            match cmd {
                Some(daemon::Command::Show) | Some(daemon::Command::ShowReverse) => {
                    let reverse = matches!(cmd, Some(daemon::Command::ShowReverse));
                    #[cfg(debug_assertions)]
                    eprintln!("[alt-tab/daemon] received Show (reverse={})", reverse);
                    let current2 = current.clone();
                    let tracker2 = tracker_clone.clone();
                    let last_window_count2 = last_window_count.clone();
                    let window_cache2 = window_cache.clone();
                    let _ = cx.update(|app_cx| {
                        let reloaded_config = crate::config::load_alt_tab_config();
                        open_picker(
                            &reloaded_config,
                            &current2,
                            &tracker2,
                            last_window_count2,
                            window_cache2,
                            reverse,
                            app_cx,
                        );
                    });
                }
                Some(daemon::Command::Kill) | None => {
                    #[cfg(debug_assertions)]
                    eprintln!("[alt-tab/daemon] shutting down");
                    cx.update(|cx| cx.quit()).ok();
                    break;
                }
            }
        })
        .detach();
    });
}

fn maybe_open_settings(args: &[String]) -> bool {
    if !args.iter().any(|arg| arg == "--settings") {
        return false;
    }
    if let Err(error) = open::that(SETTINGS_URL) {
        eprintln!("Failed to open settings page: {}", error);
    }
    true
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if maybe_open_settings(&args) {
        return;
    }

    let is_show = args.iter().any(|a| a == "--show");
    let is_show_reverse = args.iter().any(|a| a == "--show-reverse");
    let is_kill = args.iter().any(|a| a == "--kill");

    if is_kill {
        daemon::send_kill();
        return;
    }

    // If daemon is alive, forward command and exit
    if is_show_reverse && daemon::send_show_reverse() {
        return;
    }
    if is_show && daemon::send_show() {
        return;
    }

    // Otherwise start as daemon
    let config = load_alt_tab_config();
    let (tx, rx) = mpsc::channel();

    if !daemon::start_listener(tx) {
        if is_show_reverse {
            daemon::send_show_reverse();
        } else if is_show {
            daemon::send_show();
        }
        return;
    }

    run_app(config, rx, is_show || is_show_reverse);
    daemon::cleanup();
}

#[cfg(test)]
mod tests {
    use qol_tray::plugins::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        let manifest_str =
            std::fs::read_to_string("plugin.toml").expect("Failed to read plugin.toml");
        let manifest: PluginManifest =
            toml::from_str(&manifest_str).expect("Failed to parse plugin.toml");
        manifest.validate().expect("Manifest validation failed");

        println!("Plugin contract passed successfully!");
    }
}
