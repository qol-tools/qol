mod config;
mod daemon;
mod layout;
mod monitor;
mod platform;
mod window_source;

use crate::config::{load_alt_tab_config, AltTabConfig};
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
}

impl WindowDelegate {
    fn new(windows: Vec<WindowInfo>) -> Self {
        let selected_index = if windows.is_empty() {
            None
        } else {
            Some(IndexPath::new(0))
        };
        Self {
            windows,
            selected_index,
        }
    }

    fn set_windows(&mut self, windows: Vec<WindowInfo>) {
        self.windows = windows;
        if self.windows.is_empty() {
            self.selected_index = None;
            return;
        }

        let selected_row = self.selected_index.map(|ix| ix.row).unwrap_or(0);
        self.selected_index = Some(IndexPath::new(selected_row.min(self.windows.len() - 1)));
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
                        .child(win.title.clone()),
                ),
        );

        Some(ListItem::new(("window", ix.row)).child(item))
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
        self.activate_selected(window);
    }
}

impl WindowDelegate {
    fn activate_selected(&self, window: &mut Window) {
        if let Some(ix) = self.selected_index {
            let win = &self.windows[ix.row];
            platform::activate_window(win.id);
            window.minimize_window();
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
    _focus_out_subscription: Subscription,
}

impl AltTabApp {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        last_window_count: Arc<AtomicUsize>,
        window_cache: Arc<std::sync::Mutex<Vec<WindowInfo>>>,
    ) -> Self {
        let cached_windows = window_cache
            .lock()
            .map(|cache| cache.clone())
            .unwrap_or_default();
        if !cached_windows.is_empty() {
            last_window_count.store(cached_windows.len().max(1), Ordering::Relaxed);
        }
        let has_cached_windows = !cached_windows.is_empty();
        let delegate = WindowDelegate::new(cached_windows);

        let list_state = cx.new(|cx| ListState::new(delegate, window, cx));
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        let mut async_window = window.to_async(cx);
        let focus_handle_for_sub = focus_handle.clone();
        let focus_out_subscription = cx.on_focus_out(
            &focus_handle_for_sub,
            window,
            |_this, _event, window, _cx| {
                window.minimize_window();
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
                            state.delegate_mut().set_windows(windows.clone());
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

        Self {
            list_state,
            focus_handle,
            _focus_out_subscription: focus_out_subscription,
        }
    }

    fn apply_cached_windows(&mut self, windows: Vec<WindowInfo>, cx: &mut Context<Self>) {
        self.list_state.update(cx, |state, cx| {
            state.delegate_mut().set_windows(windows);
            cx.notify();
        });
        cx.notify();
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

        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .bg(rgb(0x0f111a))
            .w_full()
            .h_full()
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" | "esc" => window.minimize_window(),
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
                        if let Some(id) = win_id {
                            platform::activate_window(id);
                            window.minimize_window();
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
                                    if let Some(id) = window_id {
                                        platform::activate_window(id);
                                        window.minimize_window();
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
                                        .child(win.title.clone()),
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
    _config: &AltTabConfig,
    current: &std::rc::Rc<std::cell::RefCell<Option<WindowHandle<AltTabApp>>>>,
    tracker: &MonitorTracker,
    last_window_count: Arc<AtomicUsize>,
    window_cache: Arc<std::sync::Mutex<Vec<WindowInfo>>>,
    cx: &mut App,
) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/open] show request");
    let mut reopen_bounds: Option<Bounds<Pixels>> = None;

    // Fast path: reuse existing picker window instead of recreating it.
    let existing_handle = current.borrow().clone();
    if let Some(handle) = existing_handle {
        let cached_windows = window_cache
            .lock()
            .map(|cache| cache.clone())
            .unwrap_or_default();
        let target_count = cached_windows.len().max(1);
        let (target_w, target_h) = picker_dimensions(target_count);
        let target_size = size(px(target_w), px(target_h));
        let target_bounds = if let Some(active) = tracker.snapshot() {
            active.centered_bounds(target_size)
        } else {
            Bounds::centered(None, target_size, cx)
        };
        let needs_reopen = std::cell::Cell::new(false);
        if handle
            .update(cx, |view, window: &mut Window, cx| {
                let current_bounds = window.window_bounds().get_bounds();
                let dx = (current_bounds.origin.x.to_f64() - target_bounds.origin.x.to_f64()).abs();
                let dy = (current_bounds.origin.y.to_f64() - target_bounds.origin.y.to_f64()).abs();
                if dx > REUSE_ORIGIN_TOLERANCE_PX || dy > REUSE_ORIGIN_TOLERANCE_PX
                {
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "[alt-tab/open] reopen required for monitor switch: current_origin={:?} target_origin={:?} dx={:.1} dy={:.1}",
                        current_bounds.origin, target_bounds.origin, dx, dy
                    );
                    needs_reopen.set(true);
                    return;
                }

                view.apply_cached_windows(cached_windows.clone(), cx);

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
            && !needs_reopen.get()
        {
            #[cfg(debug_assertions)]
            eprintln!("[alt-tab/open] reused existing picker window");
            cx.activate(true);
            return;
        }

        if needs_reopen.get() {
            reopen_bounds = Some(target_bounds);
            let _ = handle.update(cx, |_view, window: &mut Window, _cx| {
                window.remove_window();
            });
        }
        *current.borrow_mut() = None;
    }

    let cached_count = window_cache.lock().map(|cache| cache.len()).unwrap_or(0);
    let estimated_count = cached_count
        .max(last_window_count.load(Ordering::Relaxed))
        .max(1);
    let (win_w, win_h) = picker_dimensions(estimated_count);
    let win_size = size(px(win_w), px(win_h));

    let bounds = reopen_bounds.unwrap_or_else(|| {
        if let Some(active) = tracker.snapshot() {
            active.centered_bounds(win_size)
        } else {
            Bounds::centered(None, win_size, cx)
        }
    });

    println!(
        "[alt-tab] opening at {:?} with size {:?}",
        bounds.origin, bounds.size
    );

    let last_window_count2 = last_window_count.clone();
    let window_cache2 = window_cache.clone();
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            kind: WindowKind::PopUp,
            focus: true,
            ..Default::default()
        },
        move |window, cx| {
            let view = cx.new(|cx| {
                AltTabApp::new(
                    window,
                    cx,
                    last_window_count2.clone(),
                    window_cache2.clone(),
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
                Some(daemon::Command::Show) => {
                    #[cfg(debug_assertions)]
                    eprintln!("[alt-tab/daemon] received Show");
                    let current2 = current.clone();
                    let tracker2 = tracker_clone.clone();
                    let last_window_count2 = last_window_count.clone();
                    let window_cache2 = window_cache.clone();
                    let _ = cx.update(|app_cx| {
                        open_picker(
                            &config,
                            &current2,
                            &tracker2,
                            last_window_count2,
                            window_cache2,
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
    let is_kill = args.iter().any(|a| a == "--kill");

    if is_kill {
        daemon::send_kill();
        return;
    }

    // If --show and daemon is alive, forward show and exit
    if is_show && daemon::send_show() {
        return;
    }

    // Otherwise start as daemon
    let config = load_alt_tab_config();
    let (tx, rx) = mpsc::channel();

    if !daemon::start_listener(tx) {
        // Another instance is alive; just signal it if needed
        if is_show {
            daemon::send_show();
        }
        return;
    }

    run_app(config, rx, is_show);
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
