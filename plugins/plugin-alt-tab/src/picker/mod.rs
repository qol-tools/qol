pub(crate) mod create;
pub(crate) mod gather;
pub(crate) mod platform;
mod reuse;
pub(crate) mod run;

pub use platform::{dismiss_picker, is_modifier_held};
pub(crate) use reuse::ReuseRequest;

use crate::app::{AltTabApp, PICKER_VISIBLE};
use crate::config::{parse_hex_color, ActionMode, AltTabConfig, DisplayConfig};
use crate::{PickerWindowState, SharedIconCache};
use gather::{
    gather, spawn_icon_fill, spawn_preview_fill, GatheredWindows, IconFillRequest,
    PreviewFillRequest,
};
use gpui::*;
use qol_plugin_api::monitor::MonitorTracker;
use qol_plugin_api::window::{MonitorKey, PopupPlacement};
use run::{SharedPreviewCache, WindowCache};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const DEFAULT_ESTIMATED_WINDOW_COUNT: usize = 8;

/// Sentinel MonitorKey slot that holds the pre-created keep-alive picker on macOS before
/// it has ever been shown on a real monitor. Chosen to not collide with any real monitor
/// (negative width/height) nor with `MonitorKey::fallback()` (all zeroes).
#[cfg(target_os = "macos")]
pub(crate) const BOOTSTRAP_KEY: MonitorKey = MonitorKey {
    x: i32::MIN,
    y: i32::MIN,
    width: -1,
    height: -1,
};

pub(crate) fn default_estimated_window_count() -> usize {
    DEFAULT_ESTIMATED_WINDOW_COUNT
}

pub(crate) struct OpenPickerRequest<'a> {
    pub config: &'a AltTabConfig,
    pub current: &'a PickerWindowState,
    pub tracker: &'a MonitorTracker,
    pub last_window_count: Arc<AtomicUsize>,
    pub icon_cache: SharedIconCache,
    pub window_cache: WindowCache,
    pub preview_cache: SharedPreviewCache,
    pub reverse: bool,
}

pub(crate) fn open_picker(req: &OpenPickerRequest, cx: &mut App) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/open] show request (reverse={})", req.reverse);

    if req.reverse && req.current.borrow().is_empty() {
        return;
    }
    if try_cycle_existing(req, cx) {
        return;
    }

    let placement = PopupPlacement::from_tracker(req.tracker);
    let gathered = gather(
        req.config,
        &req.icon_cache,
        &req.window_cache,
        &req.preview_cache,
    );
    if try_reuse_existing(req, &placement, &gathered, cx) {
        return;
    }
    // macOS keeps the pre-created picker alive across opens — the reuse path above should
    // always succeed. The fallback below exists for first-ever open before bootstrap completes
    // and for Linux's create-on-demand lifecycle.
    destroy_non_target_windows(req, &placement, cx);
    create_from_request(req, placement, gathered, cx);
}

fn try_cycle_existing(req: &OpenPickerRequest, cx: &mut App) -> bool {
    let handle = match any_existing(req.current) {
        Some((_, h)) => h,
        None => return false,
    };
    if !try_cycle_selection(&handle, req.reverse, cx) {
        return false;
    }
    PICKER_VISIBLE.store(true, Ordering::Relaxed);
    cx.activate(true);
    true
}

fn try_reuse_existing(
    req: &OpenPickerRequest,
    placement: &PopupPlacement,
    gathered: &GatheredWindows,
    cx: &mut App,
) -> bool {
    let target = placement.target();
    let (handle, source_key) = match req.current.borrow().existing(target) {
        Some(h) => (h, target),
        None => match any_existing(req.current) {
            Some((key, h)) => (h, key),
            None => return false,
        },
    };
    let input = reuse::LayoutInput {
        config: req.config,
        window_count: gathered.windows.len(),
        placement,
    };
    let layout = reuse::compute_layout(&input, cx);
    let reuse_req = reuse::ReuseRequest {
        handle: &handle,
        layout: &layout,
        config: req.config,
        gathered,
        reverse: req.reverse,
    };
    if reuse::try_reuse(&reuse_req, cx) {
        if source_key != target {
            req.current.borrow_mut().remove(source_key);
            req.current.borrow_mut().insert(target, handle);
        }
        finalize_reuse(handle, gathered, &req.icon_cache, &req.preview_cache, cx);
        return true;
    }
    discard_old_window(req, source_key, handle, cx);
    false
}

fn any_existing(current: &PickerWindowState) -> Option<(MonitorKey, WindowHandle<AltTabApp>)> {
    current.borrow().iter().into_iter().next()
}

fn destroy_non_target_windows(req: &OpenPickerRequest, placement: &PopupPlacement, cx: &mut App) {
    platform::destroy_non_target_windows(req.current, placement.target(), cx);
}

fn discard_old_window(
    req: &OpenPickerRequest,
    target: qol_plugin_api::window::MonitorKey,
    handle: WindowHandle<AltTabApp>,
    cx: &mut App,
) {
    platform::discard_old_window(req.current, target, handle, cx);
}

fn create_from_request(
    req: &OpenPickerRequest,
    placement: PopupPlacement,
    gathered: GatheredWindows,
    cx: &mut App,
) {
    let create_req = create::CreateRequest {
        config: req.config,
        placement,
        last_window_count: req.last_window_count.clone(),
        icon_cache: req.icon_cache.clone(),
        preview_cache: req.preview_cache.clone(),
        current: req.current,
    };
    create::create_new(&create_req, gathered, cx);
}

fn try_cycle_selection(handle: &WindowHandle<AltTabApp>, reverse: bool, cx: &mut App) -> bool {
    handle
        .update(cx, |view, window: &mut Window, cx| -> bool {
            if !PICKER_VISIBLE.load(Ordering::Relaxed) {
                return false;
            }
            if view.action_mode != ActionMode::HoldToSwitch {
                return false;
            }
            view.ensure_live_preview(cx);
            if view._alt_poll_task.is_none() {
                view.start_alt_poll(window.window_handle(), cx);
            }
            #[cfg(debug_assertions)]
            eprintln!(
                "[alt-tab/hold] window already visible (reverse={}) — cycling",
                reverse
            );
            view.delegate.update(cx, |s, _| match reverse {
                true => s.select_prev(),
                false => s.select_next(),
            });
            cx.notify();
            true
        })
        .unwrap_or(false)
}

fn finalize_reuse(
    handle: WindowHandle<AltTabApp>,
    gathered: &GatheredWindows,
    icon_cache: &SharedIconCache,
    preview_cache: &SharedPreviewCache,
    cx: &mut App,
) {
    let previews = gathered.previews.clone();
    PICKER_VISIBLE.store(true, Ordering::Relaxed);
    let _ = handle.update(cx, |view, _window, cx| {
        view.ensure_live_preview(cx);
        view.delegate
            .update(cx, |state, _| state.insert_previews(previews));
        cx.notify();
    });
    let icon_req = IconFillRequest {
        handle,
        windows: gathered.windows.clone(),
        icon_cache: icon_cache.clone(),
    };
    spawn_icon_fill(icon_req, &gathered.icons, cx);
    let preview_req = PreviewFillRequest {
        handle,
        windows: gathered.windows.clone(),
        preview_cache: preview_cache.clone(),
    };
    spawn_preview_fill(preview_req, cx);
    cx.activate(true);
}

pub(crate) fn resolve_card_bg(display: &DisplayConfig) -> (u32, f32) {
    let (r, g, b) = parse_hex_color(&display.card_background_color).unwrap_or((0x1a, 0x1e, 0x2a));
    let color = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
    (color, display.card_background_opacity.clamp(0.0, 1.0))
}

pub(crate) mod state {
    use crate::actions;
    use crate::config::{AltTabConfig, LabelConfig};
    use crate::discovery::WindowInfo;
    use crate::picker::create::PickerInit;
    use crate::{IconMap, PreviewMap};

    pub(crate) struct PickerState {
        pub(crate) windows: Vec<WindowInfo>,
        pub(crate) selected_index: Option<usize>,
        pub(crate) label_config: LabelConfig,
        pub(crate) transparent_background: bool,
        pub(crate) card_bg_color: u32,
        pub(crate) card_bg_opacity: f32,
        pub(crate) show_debug_overlay: bool,
        pub(crate) show_hotkey_hints: bool,
        pub(crate) live_previews: PreviewMap,
        pub(crate) icon_cache: IconMap,
    }

    impl PickerState {
        pub(crate) fn from_init(init: PickerInit) -> Self {
            let selected_index = if init.windows.is_empty() {
                None
            } else {
                Some(0)
            };
            Self {
                windows: init.windows,
                selected_index,
                label_config: init.label_config,
                transparent_background: init.transparent_bg,
                card_bg_color: init.card_color,
                card_bg_opacity: init.card_opacity,
                show_debug_overlay: init.show_debug_overlay,
                show_hotkey_hints: init.show_hotkey_hints,
                live_previews: init.previews,
                icon_cache: init.icons,
            }
        }

        pub(crate) fn set_windows(&mut self, windows: Vec<WindowInfo>, reset_selection: bool) {
            self.windows = windows;
            let active_ids: std::collections::HashSet<u32> =
                self.windows.iter().map(|w| w.id).collect();
            self.live_previews.retain(|id, _| active_ids.contains(id));

            self.selected_index = match (self.windows.is_empty(), reset_selection) {
                (true, _) => None,
                (_, true) => Some(0),
                _ => Some(self.selected_index.unwrap_or(0).min(self.windows.len() - 1)),
            };

            #[cfg(debug_assertions)]
            eprintln!(
                "[alt-tab/select] set_windows reset={} next={:?} total={}",
                reset_selection,
                self.selected_index,
                self.windows.len()
            );
        }

        pub(crate) fn insert_icons(&mut self, icons: IconMap) {
            self.icon_cache.extend(icons);
        }

        pub(crate) fn insert_previews(&mut self, previews: PreviewMap) {
            self.live_previews.extend(previews);
        }

        pub(crate) fn apply_config(
            &mut self,
            config: &AltTabConfig,
            card_color: u32,
            card_opacity: f32,
        ) {
            self.label_config = config.label.clone();
            self.transparent_background = config.display.transparent_background;
            self.card_bg_color = card_color;
            self.card_bg_opacity = card_opacity;
            self.show_debug_overlay = config.display.show_debug_overlay;
            self.show_hotkey_hints = config.display.show_hotkey_hints;
        }

        pub(crate) fn remove_window(&mut self, window_id: u32) {
            let remaining: Vec<_> = self
                .windows
                .iter()
                .filter(|w| w.id != window_id)
                .cloned()
                .collect();
            self.set_windows(remaining, false);
        }

        pub(crate) fn remove_app_windows(&mut self, app_name: &str) {
            let remaining: Vec<_> = self
                .windows
                .iter()
                .filter(|w| w.app_name != app_name)
                .cloned()
                .collect();
            self.set_windows(remaining, false);
        }

        pub(crate) fn mark_minimized(&mut self, window_id: u32) {
            let Some(idx) = self.windows.iter().position(|w| w.id == window_id) else {
                return;
            };
            let mut w = self.windows.remove(idx);
            w.is_minimized = true;
            self.windows.push(w);
            let reordered = std::mem::take(&mut self.windows);
            self.set_windows(reordered, false);
        }

        // Caller is responsible for dismissing the picker after this returns.
        pub(crate) fn activate_selected_target(&self) {
            let Some(ix) = self.selected_index else {
                #[cfg(debug_assertions)]
                eprintln!("[alt-tab/activate] no selection — skipping");
                return;
            };
            let win = &self.windows[ix];
            #[cfg(debug_assertions)]
            eprintln!(
                "[alt-tab/activate] idx={} id={} app={} title={}",
                ix, win.id, win.app_name, win.title
            );
            actions::activate_window(win.id);
            push_focus_hint(win);
        }

        pub(crate) fn select_next(&mut self) {
            if self.windows.is_empty() {
                return;
            }
            let current = self.selected_index.unwrap_or(0);
            self.selected_index = Some((current + 1) % self.windows.len());
        }

        pub(crate) fn select_prev(&mut self) {
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

        pub(crate) fn select_left(&mut self, columns: usize) {
            self.move_in_grid(GridDirection::Left, columns);
        }

        pub(crate) fn select_right(&mut self, columns: usize) {
            self.move_in_grid(GridDirection::Right, columns);
        }

        pub(crate) fn select_up(&mut self, columns: usize) {
            self.move_in_grid(GridDirection::Up, columns);
        }

        pub(crate) fn select_down(&mut self, columns: usize) {
            self.move_in_grid(GridDirection::Down, columns);
        }

        fn move_in_grid(&mut self, direction: GridDirection, columns: usize) {
            let total = self.windows.len();
            if total == 0 {
                return;
            }
            let current = self
                .selected_index
                .unwrap_or(0)
                .min(total.saturating_sub(1));
            self.selected_index = Some(grid_move(current, direction, columns, total));
        }
    }

    pub(crate) fn push_focus_hint(win: &WindowInfo) {
        let client = qol_plugin_api::PlatformStateClient::from_env();
        let Some(state) = client.get_state() else {
            return;
        };
        let win_cx = win.x + win.width / 2.0;
        let win_cy = win.y + win.height / 2.0;
        let Some(idx) = find_containing_monitor(&state.monitors, win_cx, win_cy) else {
            return;
        };
        eprintln!(
            "[alt-tab] SET_FOCUS idx={} (window {}x{} at {},{} → monitor {},{})",
            idx,
            win.width as i32,
            win.height as i32,
            win.x as i32,
            win.y as i32,
            state.monitors[idx].x as i32,
            state.monitors[idx].y as i32,
        );
        client.set_focus(idx);
    }

    pub(crate) fn find_containing_monitor(
        monitors: &[qol_plugin_api::MonitorBounds],
        x: f32,
        y: f32,
    ) -> Option<usize> {
        monitors
            .iter()
            .enumerate()
            .find(|(_, m)| x >= m.x && x < m.x + m.width && y >= m.y && y < m.y + m.height)
            .map(|(i, _)| i)
    }

    enum GridDirection {
        Left,
        Right,
        Up,
        Down,
    }

    struct Grid {
        cols: usize,
        rows: usize,
        total: usize,
    }

    impl Grid {
        fn new(columns: usize, total: usize) -> Self {
            let cols = columns.max(1).min(total);
            Self {
                cols,
                rows: total.div_ceil(cols),
                total,
            }
        }

        fn row_end(&self, row: usize) -> usize {
            ((row + 1) * self.cols).min(self.total)
        }
    }

    fn grid_move(current: usize, direction: GridDirection, columns: usize, total: usize) -> usize {
        let g = Grid::new(columns, total);
        let row = current / g.cols;
        let col = current % g.cols;

        match direction {
            GridDirection::Left => grid_left(current, row, &g),
            GridDirection::Right => grid_right(current, row, &g),
            GridDirection::Up => grid_up(current, row, col, &g),
            GridDirection::Down => grid_down(current, row, col, &g),
        }
    }

    fn grid_left(current: usize, row: usize, g: &Grid) -> usize {
        let row_start = row * g.cols;
        if current > row_start {
            current - 1
        } else {
            current
        }
    }

    fn grid_right(current: usize, row: usize, g: &Grid) -> usize {
        if current + 1 < g.row_end(row) {
            current + 1
        } else {
            current
        }
    }

    fn grid_up(current: usize, row: usize, col: usize, g: &Grid) -> usize {
        if row == 0 {
            return current;
        }
        let target_start = (row - 1) * g.cols;
        let target_end = g.row_end(row - 1);
        target_start + col.min(target_end - target_start - 1)
    }

    fn grid_down(current: usize, row: usize, col: usize, g: &Grid) -> usize {
        if row + 1 >= g.rows {
            return current;
        }
        let target_start = (row + 1) * g.cols;
        let target_end = g.row_end(row + 1);
        target_start + col.min(target_end - target_start - 1)
    }

    #[cfg(test)]
    mod cycle_direction_tests {
        use super::PickerState;
        use crate::config::{ActionMode, LabelConfig};
        use crate::discovery::WindowInfo;
        use crate::picker::create::PickerInit;
        use std::collections::HashMap;

        fn picker(window_count: usize) -> PickerState {
            let windows: Vec<WindowInfo> = (0..window_count)
                .map(|i| WindowInfo {
                    id: i as u32,
                    title: String::new(),
                    app_name: String::new(),
                    preview_path: None,
                    icon: None,
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 0.0,
                    is_minimized: false,
                })
                .collect();
            PickerState::from_init(PickerInit {
                windows,
                label_config: LabelConfig::default(),
                transparent_bg: false,
                card_color: 0,
                card_opacity: 1.0,
                show_debug_overlay: false,
                show_hotkey_hints: false,
                action_mode: ActionMode::HoldToSwitch,
                cycle_on_open: false,
                previews: HashMap::new(),
                icons: HashMap::new(),
            })
        }

        // After fresh open with reset=Some(0), forward cycle picks idx 1 and reverse picks idx N-1.
        // Regression: reuse path used to always select_next; reverse=true now correctly select_prev.
        #[test]
        fn first_cycle_after_reset_picks_idx_1_forward() {
            for n in 2..=8 {
                let mut s = picker(n);
                assert_eq!(s.selected_index, Some(0));
                s.select_next();
                assert_eq!(s.selected_index, Some(1), "n={n}: forward must hit idx 1");
            }
        }

        #[test]
        fn first_cycle_after_reset_picks_last_idx_reverse() {
            for n in 2..=8 {
                let mut s = picker(n);
                assert_eq!(s.selected_index, Some(0));
                s.select_prev();
                assert_eq!(
                    s.selected_index,
                    Some(n - 1),
                    "n={n}: reverse must wrap to idx N-1"
                );
            }
        }

        #[test]
        fn reverse_then_forward_returns_to_origin() {
            for n in 2..=8 {
                let mut s = picker(n);
                s.select_prev();
                s.select_next();
                assert_eq!(
                    s.selected_index,
                    Some(0),
                    "n={n}: prev+next must round-trip"
                );
            }
        }

        #[test]
        fn n_forwards_equals_one_backward_for_two_windows() {
            // Two-window edge: pressing prev once must equal pressing next once.
            let mut a = picker(2);
            let mut b = picker(2);
            a.select_prev();
            b.select_next();
            assert_eq!(a.selected_index, b.selected_index);
            assert_eq!(a.selected_index, Some(1));
        }

        #[test]
        fn cycling_with_no_windows_is_noop() {
            let mut s = picker(0);
            s.select_prev();
            assert_eq!(s.selected_index, None);
            s.select_next();
            assert_eq!(s.selected_index, None);
        }
    }
}
