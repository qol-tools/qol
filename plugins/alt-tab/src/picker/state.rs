use crate::actions;
use crate::config::{AltTabConfig, LabelConfig, PreviewIconPosition};
use crate::discovery::WindowInfo;
use crate::picker::create::PickerInit;
use crate::picker::{IconMap, LiveFrameMap, PreviewMap};
use futures::channel::oneshot;
use gpui::{App, Window};
use qol_gpui::image_registry::{extend_with, replace_map, retain_or_release, REGISTRY};

pub(crate) struct PickerState {
    pub(crate) windows: Vec<WindowInfo>,
    pub(crate) selected_index: Option<usize>,
    pub(crate) label_config: LabelConfig,
    pub(crate) transparent_background: bool,
    pub(crate) card_bg_color: Option<u32>,
    pub(crate) card_bg_opacity: f32,
    pub(crate) unselected_card_opacity: f32,
    pub(crate) icon_position: PreviewIconPosition,
    pub(crate) show_debug_overlay: bool,
    pub(crate) show_hotkey_hints: bool,
    pub(crate) max_columns: usize,
    pub(crate) card_scale: f32,
    pub(crate) dynamic_card_scale: bool,
    pub(crate) card_padding: f32,
    pub(crate) layout_budget: Option<(f32, f32)>,
    pub(crate) live_previews: PreviewMap,
    pub(crate) live_frames: LiveFrameMap,
    pub(crate) fresh: FreshFrame,
    pub(crate) icon_cache: IconMap,
    pub(crate) dismissed: Dismissed,
}

#[derive(Default)]
pub(crate) struct FreshFrame {
    pub(crate) awaiting: Option<u32>,
    pub(crate) landed: Option<u32>,
    pub(crate) generation: usize,
    settle: Option<oneshot::Sender<()>>,
    settled: Option<oneshot::Receiver<()>>,
}

impl FreshFrame {
    pub(crate) fn covers(&self, wid: u32) -> bool {
        self.awaiting == Some(wid) || self.landed == Some(wid)
    }

    pub(crate) fn take_settled(&mut self) -> Option<oneshot::Receiver<()>> {
        self.settled.take()
    }
}

// A background refresh can land before a closed window or quit app leaves the
// window server list, so W and Q hold their removals until the next show.
#[derive(Default)]
pub(crate) struct Dismissed {
    window_ids: std::collections::HashSet<u32>,
    app_names: std::collections::HashSet<String>,
}

impl Dismissed {
    pub(crate) fn close_window(&mut self, window_id: u32) {
        self.window_ids.insert(window_id);
    }

    pub(crate) fn quit_app(&mut self, app_name: &str) {
        self.app_names.insert(app_name.to_string());
    }

    fn keeps(&self, window: &WindowInfo) -> bool {
        !self.window_ids.contains(&window.id) && !self.app_names.contains(&window.app_name)
    }
}

impl PickerState {
    pub(crate) fn from_init(init: PickerInit) -> Self {
        let selected_index = if init.windows.is_empty() {
            None
        } else {
            Some(0)
        };
        for v in init.previews.values() {
            REGISTRY.retain(v);
        }
        for v in init.icons.values() {
            REGISTRY.retain(v);
        }
        Self {
            windows: init.windows,
            selected_index,
            label_config: init.label_config,
            transparent_background: init.transparent_bg,
            card_bg_color: init.card_color,
            card_bg_opacity: init.card_opacity,
            unselected_card_opacity: init.unselected_card_opacity.clamp(0.2, 1.0),
            icon_position: init.icon_position,
            show_debug_overlay: init.show_debug_overlay,
            show_hotkey_hints: init.show_hotkey_hints,
            max_columns: init.max_columns,
            card_scale: init.card_scale,
            dynamic_card_scale: init.dynamic_card_scale,
            card_padding: init.card_padding,
            layout_budget: init.layout_budget,
            live_previews: init.previews,
            live_frames: LiveFrameMap::new(),
            fresh: FreshFrame::default(),
            icon_cache: init.icons,
            dismissed: Dismissed::default(),
        }
    }

    pub(crate) fn drain_to_registry(&mut self, app: &mut App) {
        for (_, arc) in self.live_previews.drain() {
            REGISTRY.release(arc, app, None);
        }
        for (_, arc) in self.icon_cache.drain() {
            REGISTRY.release(arc, app, None);
        }
        self.live_frames.clear();
    }

    pub(crate) fn set_windows(
        &mut self,
        mut windows: Vec<WindowInfo>,
        reset_selection: bool,
        app: &mut App,
        window: Option<&mut Window>,
    ) {
        windows.retain(|w| self.dismissed.keeps(w));
        self.windows = windows;
        let active_ids: std::collections::HashSet<u32> =
            self.windows.iter().map(|w| w.id).collect();
        retain_or_release(&mut self.live_previews, app, window, |id| {
            active_ids.contains(id)
        });
        self.live_frames.retain(|id, _| active_ids.contains(id));
        self.update_selection_after_resize(reset_selection);
    }

    #[cfg(test)]
    pub(crate) fn replace_windows_for_test(
        &mut self,
        mut windows: Vec<WindowInfo>,
        reset_selection: bool,
    ) {
        windows.retain(|w| self.dismissed.keeps(w));
        self.windows = windows;
        self.update_selection_after_resize(reset_selection);
    }

    fn update_selection_after_resize(&mut self, reset_selection: bool) {
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

    pub(crate) fn insert_icons(
        &mut self,
        icons: IconMap,
        app: &mut App,
        window: Option<&mut Window>,
    ) {
        extend_with(&mut self.icon_cache, icons, app, window);
    }

    pub(crate) fn insert_previews(
        &mut self,
        previews: PreviewMap,
        app: &mut App,
        window: Option<&mut Window>,
    ) {
        extend_with(&mut self.live_previews, previews, app, window);
    }

    pub(crate) fn insert_fresh_previews(
        &mut self,
        previews: PreviewMap,
        app: &mut App,
        window: Option<&mut Window>,
    ) {
        for wid in previews.keys() {
            self.live_frames.remove(wid);
        }
        self.insert_previews(previews, app, window);
    }

    pub(crate) fn insert_live_frames(&mut self, frames: LiveFrameMap) {
        self.live_frames.extend(frames);
    }

    pub(crate) fn await_fresh_frame(&mut self, wid: Option<u32>) {
        let (settle, settled) = oneshot::channel();
        self.fresh.awaiting = wid;
        self.fresh.landed = None;
        self.fresh.generation += 1;
        self.fresh.settle = wid.map(|_| settle);
        self.fresh.settled = wid.map(|_| settled);
    }

    pub(crate) fn land_fresh_frame(&mut self, wid: u32, frames: LiveFrameMap) -> bool {
        if self.fresh.awaiting != Some(wid) {
            return false;
        }
        self.fresh.awaiting = None;
        self.fresh.settle = None;
        if frames.contains_key(&wid) {
            self.fresh.landed = Some(wid);
            self.live_frames.extend(frames);
        }
        true
    }

    pub(crate) fn clear_live_frames(&mut self) {
        self.live_frames.clear();
    }

    pub(crate) fn replace_caches(
        &mut self,
        previews: PreviewMap,
        icons: IconMap,
        app: &mut App,
        mut window: Option<&mut Window>,
    ) {
        if !previews.is_empty() {
            replace_map(
                &mut self.live_previews,
                previews,
                app,
                window.as_deref_mut(),
            );
        }
        if !icons.is_empty() {
            replace_map(&mut self.icon_cache, icons, app, window);
        }
    }

    pub(crate) fn apply_config(
        &mut self,
        config: &AltTabConfig,
        card_color: Option<u32>,
        card_opacity: f32,
        layout_budget: Option<(f32, f32)>,
    ) {
        self.layout_budget = layout_budget;
        self.label_config = config.label.clone();
        self.transparent_background = config.display.transparent_background;
        self.card_bg_color = card_color;
        self.card_bg_opacity = card_opacity;
        self.unselected_card_opacity = config.display.unselected_card_opacity.clamp(0.2, 1.0);
        self.icon_position = config.display.icon_position;
        self.show_debug_overlay = config.display.show_debug_overlay;
        self.show_hotkey_hints = config.display.show_hotkey_hints;
        self.card_scale = config.display.card_scale;
        self.dynamic_card_scale = config.display.dynamic_card_scale;
        self.card_padding = config.display.card_padding;
    }

    pub(crate) fn remove_window(
        &mut self,
        window_id: u32,
        app: &mut App,
        window: Option<&mut Window>,
    ) {
        self.dismissed.close_window(window_id);
        self.set_windows(self.windows.clone(), false, app, window);
    }

    pub(crate) fn remove_app_windows(
        &mut self,
        app_name: &str,
        app: &mut App,
        window: Option<&mut Window>,
    ) {
        self.dismissed.quit_app(app_name);
        self.set_windows(self.windows.clone(), false, app, window);
    }

    pub(crate) fn forget_dismissed(&mut self) {
        self.dismissed = Dismissed::default();
    }

    pub(crate) fn mark_minimized(
        &mut self,
        window_id: u32,
        app: &mut App,
        window: Option<&mut Window>,
    ) {
        let Some(idx) = self.windows.iter().position(|w| w.id == window_id) else {
            return;
        };
        let mut w = self.windows.remove(idx);
        w.is_minimized = true;
        self.windows.push(w);
        let reordered = std::mem::take(&mut self.windows);
        self.set_windows(reordered, false, app, window);
    }

    pub(crate) fn select_index(&mut self, index: usize) {
        if self.windows.is_empty() {
            self.selected_index = None;
            return;
        }
        self.selected_index = Some(index.min(self.windows.len() - 1));
    }

    pub(crate) fn activate_selected_target(&self) {
        let Some(ix) = self.selected_index else {
            #[cfg(debug_assertions)]
            eprintln!("[alt-tab/activate] no selection — skipping");
            return;
        };
        let Some(win) = self.windows.get(ix) else {
            eprintln!(
                "[alt-tab/activate] selection {ix} is past {} windows - skipping",
                self.windows.len()
            );
            return;
        };
        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/activate] idx={} id={} app={} title={}",
            ix, win.id, win.app_name, win.title
        );
        actions::activate_window(win.id);
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

    pub(crate) fn cycle(&mut self, reverse: bool) {
        if reverse {
            self.select_prev();
            return;
        }
        self.select_next();
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

fn grid_left(current: usize, _row: usize, g: &Grid) -> usize {
    if current == 0 {
        g.total.saturating_sub(1)
    } else {
        current - 1
    }
}

fn grid_right(current: usize, _row: usize, g: &Grid) -> usize {
    if current + 1 >= g.total {
        0
    } else {
        current + 1
    }
}

fn grid_up(current: usize, row: usize, col: usize, g: &Grid) -> usize {
    if row == 0 {
        let last_row = g.rows.saturating_sub(1);
        let target_start = last_row * g.cols;
        let target_end = g.row_end(last_row);
        if target_end <= target_start {
            return current;
        }
        return target_start + col.min(target_end - target_start - 1);
    }
    let target_start = (row - 1) * g.cols;
    let target_end = g.row_end(row - 1);
    target_start + col.min(target_end - target_start - 1)
}

fn grid_down(current: usize, row: usize, col: usize, g: &Grid) -> usize {
    if row + 1 >= g.rows {
        let target_end = g.row_end(0);
        if target_end == 0 {
            return current;
        }
        return col.min(target_end - 1);
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
                icon: None,
                width: 0.0,
                height: 0.0,
                is_minimized: false,
            })
            .collect();
        PickerState::from_init(PickerInit {
            picker_title: String::new(),
            shown: false,
            windows,
            label_config: LabelConfig::default(),
            transparent_bg: false,
            card_color: None,
            card_opacity: 1.0,
            unselected_card_opacity: 0.85,
            icon_position: crate::config::PreviewIconPosition::default(),
            show_debug_overlay: false,
            show_hotkey_hints: false,
            action_mode: ActionMode::HoldToSwitch,
            cycle_on_open: false,
            max_columns: 6,
            card_scale: 1.0,
            dynamic_card_scale: true,
            card_padding: crate::picker::layout::DEFAULT_CARD_PADDING,
            layout_budget: None,
            rendering: crate::rendering::RenderingFlow::gpui_snapshots(),
            preview_cache: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
            previews: HashMap::new(),
            icons: HashMap::new(),
        })
    }

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

    #[test]
    fn right_at_row_end_wraps_to_next_row_start() {
        let mut s = picker(8);
        s.selected_index = Some(2);
        s.select_right(3);
        assert_eq!(s.selected_index, Some(3));
    }

    #[test]
    fn right_at_grid_end_wraps_to_first() {
        let mut s = picker(8);
        s.selected_index = Some(7);
        s.select_right(3);
        assert_eq!(s.selected_index, Some(0));
    }

    #[test]
    fn left_at_row_start_wraps_to_prev_row_end() {
        let mut s = picker(8);
        s.selected_index = Some(3);
        s.select_left(3);
        assert_eq!(s.selected_index, Some(2));
    }

    #[test]
    fn left_at_grid_start_wraps_to_last() {
        let mut s = picker(8);
        s.selected_index = Some(0);
        s.select_left(3);
        assert_eq!(s.selected_index, Some(7));
    }

    #[test]
    fn up_at_top_row_wraps_to_bottom_row_same_column() {
        let mut s = picker(8);
        s.selected_index = Some(1);
        s.select_up(3);
        assert_eq!(s.selected_index, Some(7));
    }

    #[test]
    fn up_at_top_row_clamps_to_last_populated_when_short_row() {
        let mut s = picker(8);
        s.selected_index = Some(2);
        s.select_up(3);
        assert_eq!(s.selected_index, Some(7));
    }

    #[test]
    fn down_at_bottom_row_wraps_to_top_row_same_column() {
        let mut s = picker(8);
        s.selected_index = Some(7);
        s.select_down(3);
        assert_eq!(s.selected_index, Some(1));
    }
}

#[cfg(test)]
mod set_windows_tests {
    use super::PickerState;
    use crate::config::{ActionMode, LabelConfig};
    use crate::discovery::WindowInfo;
    use crate::picker::create::PickerInit;
    use std::collections::HashMap;

    fn windows(n: usize) -> Vec<WindowInfo> {
        (0..n)
            .map(|i| WindowInfo {
                id: i as u32,
                title: String::new(),
                app_name: String::new(),
                icon: None,
                width: 0.0,
                height: 0.0,
                is_minimized: false,
            })
            .collect()
    }

    fn picker_at(count: usize, selected: Option<usize>) -> PickerState {
        let mut s = PickerState::from_init(PickerInit {
            picker_title: String::new(),
            shown: false,
            windows: windows(count),
            label_config: LabelConfig::default(),
            transparent_bg: false,
            card_color: None,
            card_opacity: 1.0,
            unselected_card_opacity: 0.85,
            icon_position: crate::config::PreviewIconPosition::default(),
            show_debug_overlay: false,
            show_hotkey_hints: false,
            action_mode: ActionMode::HoldToSwitch,
            cycle_on_open: false,
            max_columns: 6,
            card_scale: 1.0,
            dynamic_card_scale: true,
            card_padding: crate::picker::layout::DEFAULT_CARD_PADDING,
            layout_budget: None,
            rendering: crate::rendering::RenderingFlow::gpui_snapshots(),
            preview_cache: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
            previews: HashMap::new(),
            icons: HashMap::new(),
        });
        s.selected_index = selected;
        s
    }

    #[test]
    fn selection_tracks_window_list_changes() {
        struct Case {
            initial: usize,
            start_idx: Option<usize>,
            new_count: usize,
            reset: bool,
            want: Option<usize>,
            label: &'static str,
        }
        let cases = [
            Case {
                initial: 3,
                start_idx: Some(1),
                new_count: 3,
                reset: false,
                want: Some(1),
                label: "stable: same size, no reset",
            },
            Case {
                initial: 3,
                start_idx: Some(1),
                new_count: 3,
                reset: true,
                want: Some(0),
                label: "reset clears to 0",
            },
            Case {
                initial: 3,
                start_idx: Some(2),
                new_count: 1,
                reset: false,
                want: Some(0),
                label: "shrink: clamp to new len-1",
            },
            Case {
                initial: 3,
                start_idx: Some(1),
                new_count: 0,
                reset: false,
                want: None,
                label: "empty list clears selection",
            },
            Case {
                initial: 3,
                start_idx: Some(1),
                new_count: 0,
                reset: true,
                want: None,
                label: "empty list overrides reset=true",
            },
            Case {
                initial: 0,
                start_idx: None,
                new_count: 3,
                reset: true,
                want: Some(0),
                label: "from-empty with reset",
            },
            Case {
                initial: 0,
                start_idx: None,
                new_count: 3,
                reset: false,
                want: Some(0),
                label: "from-empty no-reset defaults to 0",
            },
            Case {
                initial: 1,
                start_idx: Some(0),
                new_count: 4,
                reset: false,
                want: Some(0),
                label: "grow preserves selection",
            },
        ];
        for c in &cases {
            let mut s = picker_at(c.initial, c.start_idx);
            s.replace_windows_for_test(windows(c.new_count), c.reset);
            assert_eq!(s.selected_index, c.want, "{}", c.label);
        }
    }

    #[test]
    fn a_stale_card_index_clamps_instead_of_panicking() {
        let mut picker = picker_at(6, Some(5));
        picker.replace_windows_for_test(windows(2), false);

        picker.select_index(5);
        assert_eq!(picker.selected_index, Some(1));

        let _serial = crate::actions::ACTIVATOR_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        picker.activate_selected_target();
    }

    #[test]
    fn a_refresh_does_not_bring_back_a_window_closed_while_the_picker_is_open() {
        let mut picker = picker_at(3, Some(1));
        picker.dismissed.close_window(1);

        picker.replace_windows_for_test(windows(3), false);

        let ids: Vec<u32> = picker.windows.iter().map(|w| w.id).collect();
        assert_eq!(ids, vec![0, 2]);
    }

    #[test]
    fn a_refresh_does_not_bring_back_windows_of_an_app_quit_while_the_picker_is_open() {
        let mut picker = picker_at(0, None);
        picker.dismissed.quit_app("Notes");
        let mut refreshed = windows(3);
        refreshed[0].app_name = "Notes".into();
        refreshed[2].app_name = "Notes".into();

        picker.replace_windows_for_test(refreshed, false);

        let ids: Vec<u32> = picker.windows.iter().map(|w| w.id).collect();
        assert_eq!(ids, vec![1]);
    }

    #[test]
    fn a_new_show_forgets_what_the_last_one_closed() {
        let mut picker = picker_at(3, Some(0));
        picker.dismissed.close_window(1);
        picker.dismissed.quit_app("");

        picker.forget_dismissed();
        picker.replace_windows_for_test(windows(3), false);

        assert_eq!(picker.windows.len(), 3);
    }

    #[test]
    fn selecting_a_card_when_every_window_closed_clears_the_selection() {
        let mut picker = picker_at(3, Some(2));
        picker.replace_windows_for_test(Vec::new(), false);

        picker.select_index(2);
        assert_eq!(picker.selected_index, None);
    }
}
