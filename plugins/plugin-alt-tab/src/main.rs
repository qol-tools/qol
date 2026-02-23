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
use monitor::MonitorTracker;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

const LIVE_PREVIEW_INTERVAL_MS: u64 = 500;

const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-alt-tab/";
const DEFAULT_ESTIMATED_WINDOW_COUNT: usize = 8;
const PREWARM_REFRESH_INTERVAL_MS: u64 = 1200;
const ALT_POLL_INTERVAL_MS: u64 = 50;

static PICKER_VISIBLE: AtomicBool = AtomicBool::new(false);

// ─── Window delegate ─────────────────────────────────────────────────────────

struct WindowDelegate {
    windows: Vec<WindowInfo>,
    selected_index: Option<usize>,
    label_config: crate::config::LabelConfig,
    live_previews: std::collections::HashMap<u32, Arc<gpui::RenderImage>>,
}

impl WindowDelegate {
    fn new(windows: Vec<WindowInfo>, label_config: crate::config::LabelConfig) -> Self {
        let selected_index = if windows.is_empty() { None } else { Some(0) };
        Self {
            windows,
            selected_index,
            label_config,
            live_previews: std::collections::HashMap::new(),
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
            self.selected_index = Some(0);
            #[cfg(debug_assertions)]
            eprintln!(
                "[alt-tab/select] set_windows reset={} next=Some(0) total={}",
                reset_selection,
                self.windows.len()
            );
            return;
        }

        let selected_row = self.selected_index.unwrap_or(0);
        self.selected_index = Some(selected_row.min(self.windows.len() - 1));
        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/select] set_windows reset={} next={:?} total={}",
            reset_selection,
            self.selected_index,
            self.windows.len()
        );
    }
}

impl WindowDelegate {
    fn activate_selected(&self, window: &mut Window) {
        if let Some(ix) = self.selected_index {
            let win = &self.windows[ix];
            platform::activate_window(win.id);
            // Push the activated window's monitor to the runtime so the focus
            // stamp survives the AX "no focused application" gap.
            let client = qol_runtime::PlatformStateClient::from_env();
            if let Some(state) = client.get_state() {
                // Find which monitor the activated window overlaps most
                let win_cx = win.x + win.width / 2.0;
                let win_cy = win.y + win.height / 2.0;
                let idx = state.monitors.iter().enumerate()
                    .find(|(_, m)| {
                        win_cx >= m.x && win_cx < m.x + m.width &&
                        win_cy >= m.y && win_cy < m.y + m.height
                    })
                    .map(|(i, _)| i);
                if let Some(idx) = idx {
                    eprintln!("[alt-tab] SET_FOCUS idx={} (window {}x{} at {},{} → monitor {},{})",
                        idx, win.width as i32, win.height as i32, win.x as i32, win.y as i32,
                        state.monitors[idx].x as i32, state.monitors[idx].y as i32);
                    client.set_focus(idx);
                }
            }
            PICKER_VISIBLE.store(false, Ordering::Relaxed);
            platform::dismiss_picker(window);
        }
    }

    fn select_next(&mut self) {
        if self.windows.is_empty() {
            return;
        }
        let current = self.selected_index.unwrap_or(0);
        self.selected_index = Some((current + 1) % self.windows.len());
    }

    fn select_prev(&mut self) {
        if self.windows.is_empty() {
            return;
        }
        let current = self.selected_index.unwrap_or(0);
        self.selected_index = Some(if current == 0 {
            self.windows.len() - 1
        } else {
            current - 1
        });
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

        self.selected_index = Some(next);
    }
}

enum GridDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Sample ~1KB of evenly-spaced pixels for a fast content-change check.
fn fast_pixel_hash(data: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let stride = (data.len() / 256).max(1);
    let mut i = 0;
    while i < data.len() {
        let end = (i + 4).min(data.len());
        data[i..end].hash(&mut hasher);
        i += stride;
    }
    hasher.finish()
}

fn rgba_to_render_image(data: &[u8], w: usize, h: usize) -> Option<Arc<gpui::RenderImage>> {
    let mut bgra = data.to_vec();
    // gpui's Metal renderer expects BGRA byte order
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let buf = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(w as u32, h as u32, bgra)?;
    let frame = image::Frame::new(buf);
    Some(Arc::new(gpui::RenderImage::new(smallvec::smallvec![frame])))
}

// ─── App view ─────────────────────────────────────────────────────────────────

struct AltTabApp {
    delegate: Entity<WindowDelegate>,
    focus_handle: FocusHandle,
    action_mode: ActionMode,
    alt_was_held: bool,
    _alt_poll_task: Option<gpui::Task<()>>,
    _live_preview_task: Option<gpui::Task<()>>,
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
        let win_delegate = WindowDelegate::new(initial_windows.clone(), label_config);
        let delegate = cx.new(|_cx| win_delegate);

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
                    PICKER_VISIBLE.store(false, Ordering::Relaxed);
                    platform::dismiss_picker(window);
                }
            },
        );

        let delegate_clone = delegate.clone();
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
                    let ids: Vec<u32> = windows.iter().map(|w| w.id).collect();
                    last_window_count.store(ids.len().max(1), Ordering::Relaxed);

                    let _ = cx.update(|app_cx| {
                        let _ = delegate_clone.update(app_cx, |state, cx| {
                            state.set_windows(windows.clone(), false);
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

                    // Live preview task handles ongoing captures — no per-window
                    // file-based capture loop needed here.
                }
            },
        )
        .detach();

        // Spawn live preview refresh task
        let delegate_for_preview = delegate.clone();
        let live_preview_task = cx.spawn(
            move |this: gpui::WeakEntity<AltTabApp>, cx: &mut gpui::AsyncApp| {
                let mut cx = cx.clone();
                async move {
                    let executor = cx.background_executor().clone();
                    // Track pixel data hashes to avoid re-uploading unchanged textures
                    let mut prev_hashes: std::collections::HashMap<u32, u64> =
                        std::collections::HashMap::new();
                    loop {
                        executor
                            .timer(Duration::from_millis(LIVE_PREVIEW_INTERVAL_MS))
                            .await;
                        if !PICKER_VISIBLE.load(Ordering::Relaxed) {
                            prev_hashes.clear();
                            continue;
                        }
                        // Collect (index, window_id) pairs
                        let window_ids: Vec<(usize, u32)> = cx
                            .update(|app_cx| {
                                delegate_for_preview
                                    .read(app_cx)
                                    .windows
                                    .iter()
                                    .enumerate()
                                    .map(|(i, w)| (i, w.id))
                                    .collect()
                            })
                            .unwrap_or_default();
                        if window_ids.is_empty() {
                            continue;
                        }
                        let id_map: Vec<(usize, u32)> = window_ids.clone();
                        let captured = executor
                            .spawn(async move {
                                platform::capture_previews_batch_rgba(
                                    &window_ids,
                                    PREVIEW_MAX_WIDTH,
                                    PREVIEW_MAX_HEIGHT,
                                )
                            })
                            .await;
                        let mut changed = false;
                        let list = delegate_for_preview.clone();
                        for (idx, rgba_opt) in captured {
                            let Some(rgba) = rgba_opt else { continue };
                            let Some(&(_, wid)) = id_map.iter().find(|(i, _)| *i == idx) else {
                                continue;
                            };
                            // Fast hash: sample a few cache lines instead of hashing all pixels
                            let hash = fast_pixel_hash(&rgba.data);
                            if prev_hashes.get(&wid) == Some(&hash) {
                                continue; // unchanged — skip atlas re-upload
                            }
                            prev_hashes.insert(wid, hash);
                            if let Some(render_img) =
                                rgba_to_render_image(&rgba.data, rgba.width, rgba.height)
                            {
                                let _ = cx.update(|app_cx| {
                                    let _ = list.update(app_cx, |state, cx| {
                                        state.live_previews.insert(wid, render_img);
                                        cx.notify();
                                    });
                                });
                                changed = true;
                            }
                        }
                        if changed {
                            let _ = cx.update(|app_cx| {
                                let _ = this.update(
                                    app_cx,
                                    |_, cx: &mut gpui::Context<AltTabApp>| {
                                        cx.notify();
                                    },
                                );
                            });
                        }
                    }
                }
            },
        );

        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/hold] AltTabApp::new: action_mode={:?}, alt_was_held=true (assumed)",
            action_mode
        );

        let mut app = Self {
            delegate,
            focus_handle,
            action_mode: action_mode.clone(),
            alt_was_held: true,
            _alt_poll_task: None,
            _live_preview_task: Some(live_preview_task),
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
        self.delegate.update(cx, |state, cx| {
            state.set_windows(windows, reset_selection);
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
        let list = self.delegate.clone();
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
                            .timer(std::time::Duration::from_millis(ALT_POLL_INTERVAL_MS))
                            .await;
                        let alt_held = platform::is_modifier_held();

                        if !alt_held {
                            eprintln!(
                                "[alt-tab/hold] X11 poll: Alt released — activating selected"
                            );
                            let list = list.clone();
                            let _ = cx.update_window(window_handle, |_root, window, cx| {
                                list.update(cx, |s, _cx| {
                                    s.activate_selected(window);
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
        let delegate = self.delegate.clone();

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
                    "escape" | "esc" => {
                        PICKER_VISIBLE.store(false, Ordering::Relaxed);
                        platform::dismiss_picker(window);
                    }
                    "enter" => {
                        // Activate selected and close
                        let win_id =
                            this.delegate
                                .read(cx)
                                .selected_index
                                .and_then(|ix| {
                                    this.delegate
                                        .read(cx)
                                        .windows
                                        .get(ix)
                                        .map(|w| w.id)
                                });
                        if let Some(_id) = win_id {
                            this.delegate.update(cx, |s, _cx| {
                                s.activate_selected(window);
                            });
                        }
                    }
                    // Navigate — do NOT call s.focus() after, that would steal focus
                    // from our root div and break further key events.
                    "tab" => {
                        this.delegate.update(cx, |s, _cx| {
                            if event.keystroke.modifiers.shift {
                                s.select_prev();
                            } else {
                                s.select_next();
                            }
                        });
                        cx.notify();
                    }
                    "backtab" => {
                        this.delegate.update(cx, |s, _cx| {
                            s.select_prev();
                        });
                        cx.notify();
                    }
                    "right" | "arrowright" => {
                        let total = this.delegate.read(cx).windows.len();
                        let cols = rendered_column_count(window, total);
                        this.delegate.update(cx, |s, _cx| {
                            s.select_right(cols);
                        });
                        cx.notify();
                    }
                    "left" | "arrowleft" => {
                        let total = this.delegate.read(cx).windows.len();
                        let cols = rendered_column_count(window, total);
                        this.delegate.update(cx, |s, _cx| {
                            s.select_left(cols);
                        });
                        cx.notify();
                    }
                    "down" | "arrowdown" => {
                        let total = this.delegate.read(cx).windows.len();
                        let cols = rendered_column_count(window, total);
                        this.delegate.update(cx, |s, _cx| {
                            s.select_down(cols);
                        });
                        cx.notify();
                    }
                    "up" | "arrowup" => {
                        let total = this.delegate.read(cx).windows.len();
                        let cols = rendered_column_count(window, total);
                        this.delegate.update(cx, |s, _cx| {
                            s.select_up(cols);
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
                    let d = delegate.read(cx);
                    let windows = d.windows.clone();
                    let selected_index = d.selected_index;
                    let label_config = d.label_config.clone();
                    let live_previews = d.live_previews.clone();

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
                            let is_selected = selected_index == Some(i);
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
                                            this.delegate.update(cx, |s, _cx| {
                                                s.selected_index = Some(i);
                                            });
                                            this.delegate
                                                .read(cx)
                                                .windows
                                                .get(i)
                                                .map(|w| w.id)
                                        })
                                        .ok()
                                        .flatten();
                                    if let Some(_id) = window_id {
                                        entity_for_click
                                            .update(cx, |this, cx| {
                                                this.delegate.update(cx, |s, _cx| {
                                                    s.activate_selected(window);
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
                                    live_previews.get(&win.id),
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
    current: &std::rc::Rc<std::cell::RefCell<Option<(WindowHandle<AltTabApp>, Point<Pixels>)>>>,
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
    let existing = current.borrow().clone();
    if let Some((handle, created_on_origin)) = existing {
        let target_count = display_windows.len().max(1);
        let target_monitor = tracker.snapshot().map(|(m, _)| m);
        let monitor_size = target_monitor.as_ref().map(|m| m.size());
        let (target_w, target_h) = picker_dimensions(target_count, config.display.max_columns, monitor_size);
        let target_size = size(px(target_w), px(target_h));
        let target_bounds = if let Some(ref active) = target_monitor {
            active.centered_bounds(target_size)
        } else {
            Bounds::centered(None, target_size, cx)
        };

        // Determine if the target monitor differs from the one the window was created on.
        let target_origin = target_monitor
            .as_ref()
            .map(|m| m.bounds().origin)
            .unwrap_or(point(px(0.0), px(0.0)));
        const MONITOR_TOLERANCE_PX: f64 = 6.0;
        let monitor_changed = {
            let dx = (created_on_origin.x.to_f64() - target_origin.x.to_f64()).abs();
            let dy = (created_on_origin.y.to_f64() - target_origin.y.to_f64()).abs();
            dx > MONITOR_TOLERANCE_PX || dy > MONITOR_TOLERANCE_PX
        };

        let reuse_ok = handle
            .update(cx, |view, window: &mut Window, cx| -> bool {
                // In HoldToSwitch mode, if the poll task is still running, the window
                // is already visible. Treat subsequent Show commands as "select next".
                let alt_held = platform::is_modifier_held();
                if view.action_mode == ActionMode::HoldToSwitch && view._alt_poll_task.is_some() && alt_held {
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "[alt-tab/hold] window already visible (alt_held={} reverse={}) — cycling",
                        alt_held, reverse
                    );
                    view.delegate.update(cx, |s, _cx| {
                        if reverse {
                            s.select_prev();
                        } else {
                            s.select_next();
                        }
                    });
                    cx.notify();
                    return true;
                }
                #[cfg(debug_assertions)]
                eprintln!(
                    "[alt-tab/hold] reuse path (alt_held={} poll_task={}) — applying config reset={} monitor_changed={}",
                    alt_held,
                    view._alt_poll_task.is_some(),
                    config.reset_selection_on_open,
                    monitor_changed,
                );

                if monitor_changed {
                    let x = target_bounds.origin.x.to_f64() as i32;
                    let y = target_bounds.origin.y.to_f64() as i32;
                    if !platform::move_app_window("qol-alt-tab-picker", x, y) {
                        // Can't move — signal caller to close and reopen on correct monitor
                        return false;
                    }
                }

                #[cfg(debug_assertions)]
                eprintln!("[alt-tab/hold] open_picker reusing window: setting action_mode={:?}", config.action_mode);

                view.action_mode = config.action_mode.clone();
                view.alt_was_held = true;

                // Update label config and cache active monitor from tracker
                view.delegate.update(cx, |s, _cx| {
                    s.label_config = config.label.clone();
                });

                // Start a new X11 modifier polling task for HoldToSwitch
                if config.action_mode == ActionMode::HoldToSwitch {
                    let wh = window.window_handle();
                    view.start_alt_poll(wh, cx);
                } else {
                    view._alt_poll_task = None; // Cancel any existing poll
                }

                view.apply_cached_windows(display_windows.clone(), config.reset_selection_on_open, cx);

                let current_bounds = window.window_bounds().get_bounds();
                let current_size = current_bounds.size;
                let next_size = target_size;
                if (current_size.width.to_f64() - target_w as f64).abs() >= 1.0
                    || (current_size.height.to_f64() - target_h as f64).abs() >= 1.0
                {
                    window.resize(next_size);
                }
                window.focus(&view.focus_handle(cx));
                window.activate_window();
                true
            })
            .unwrap_or(false);

        if reuse_ok {
            #[cfg(debug_assertions)]
            eprintln!("[alt-tab/open] reused existing picker window");
            cx.activate(true);
            return;
        }

        // Close the old window so we don't leak orphaned windows
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/open] closing old window — will recreate on correct monitor");
        let _ = handle.update(cx, |_view, window, _cx| {
            window.remove_window();
        });
        *current.borrow_mut() = None;
    }

    let target_count = display_windows.len().max(1);
    let estimated_count = target_count
        .max(last_window_count.load(Ordering::Relaxed))
        .max(1);
    let create_monitor = tracker.snapshot().map(|(m, _)| m);
    let monitor_size = create_monitor.as_ref().map(|m| m.size());
    let (win_w, win_h) = picker_dimensions(estimated_count, config.display.max_columns, monitor_size);
    let win_size = size(px(win_w), px(win_h));
    let bounds = if let Some(ref active) = create_monitor {
        active.centered_bounds(win_size)
    } else {
        Bounds::centered(None, win_size, cx)
    };
    let create_origin = create_monitor
        .as_ref()
        .map(|m| m.bounds().origin)
        .unwrap_or(point(px(0.0), px(0.0)));

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
        *current.borrow_mut() = Some((h, create_origin));
    } else {
        #[cfg(debug_assertions)]
        eprintln!("[alt-tab/open] failed to open picker window");
    }
    PICKER_VISIBLE.store(true, Ordering::Relaxed);
    cx.activate(true);

    #[cfg(target_os = "macos")]
    set_macos_accessory_policy();
}

#[cfg(target_os = "macos")]
fn set_macos_accessory_policy() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

fn run_app(config: AltTabConfig, rx: mpsc::Receiver<daemon::Command>, show_on_start: bool) {
    let app = Application::new();

    app.run(move |cx: &mut App| {
        let tracker = MonitorTracker::start(cx);

        open_keepalive(cx);

        #[cfg(target_os = "macos")]
        set_macos_accessory_policy();

        // Track the single picker window + the monitor origin it was placed on
        let current: std::rc::Rc<std::cell::RefCell<Option<(WindowHandle<AltTabApp>, Point<Pixels>)>>> =
            std::rc::Rc::new(std::cell::RefCell::new(None));
        let last_window_count = Arc::new(AtomicUsize::new(DEFAULT_ESTIMATED_WINDOW_COUNT));
        let window_cache: Arc<std::sync::Mutex<Vec<WindowInfo>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let warm_cache = window_cache.clone();
        let warm_count = last_window_count.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let executor = cx.background_executor().clone();
            loop {
                executor
                    .timer(Duration::from_millis(PREWARM_REFRESH_INTERVAL_MS))
                    .await;
                if PICKER_VISIBLE.load(Ordering::Relaxed) {
                    continue;
                }
                let cached = warm_cache
                    .lock()
                    .map(|cache| cache.clone())
                    .unwrap_or_default();
                let refreshed = load_windows_with_previews(&executor, cached, false).await;
                warm_count.store(refreshed.len().max(1), Ordering::Relaxed);
                if let Ok(mut cache) = warm_cache.lock() {
                    *cache = refreshed;
                }
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
