use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rgb, rgba, App, Context, Div, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Window,
};

const STRIP_WIDTH: f32 = 12.0;
const PICKUP_RADIUS: f32 = 1.0 / 32.0;
const OVERVIEW_BG: u32 = 0x0e1117;
const VIEWPORT_BAND: u32 = 0xffffff26;
const ACCENT_DIM: u32 = 0x57401a;
const ACCENT_BRIGHT: u32 = 0xffb03c;
const TICK_MIN_H: f32 = 2.0;
const TICK_MAX_H: f32 = 12.0;
const TICK_WIDTH: f32 = 6.0;
const TICK_WIDTH_HOVERED: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HunkMarker {
    pub offset_ratio: f32,
    pub weight: u32,
}

impl HunkMarker {
    pub fn new(offset_ratio: f32, weight: u32) -> Self {
        Self {
            offset_ratio: offset_ratio.clamp(0.0, 1.0),
            weight,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverviewState {
    pub markers: Vec<HunkMarker>,
    pub hovered: Option<usize>,
    pub viewport: Option<(f32, f32)>,
    pub selection: Option<usize>,
}

impl Default for OverviewState {
    fn default() -> Self {
        Self::new()
    }
}

impl OverviewState {
    pub fn new() -> Self {
        Self {
            markers: Vec::new(),
            hovered: None,
            viewport: None,
            selection: None,
        }
    }

    pub fn set_markers(&mut self, markers: Vec<HunkMarker>) {
        self.markers = markers
            .into_iter()
            .map(|marker| HunkMarker::new(marker.offset_ratio, marker.weight))
            .collect();
        self.hovered = self.hovered.filter(|index| *index < self.markers.len());
        self.selection = self.selection.filter(|index| *index < self.markers.len());
    }

    pub fn hover_at(&self, y_ratio: f32) -> Option<usize> {
        self.markers
            .iter()
            .enumerate()
            .filter(|(_, marker)| (marker.offset_ratio - y_ratio).abs() <= PICKUP_RADIUS)
            .min_by(|(left_index, left), (right_index, right)| {
                let left_distance = (left.offset_ratio - y_ratio).abs();
                let right_distance = (right.offset_ratio - y_ratio).abs();
                left_distance
                    .total_cmp(&right_distance)
                    .then_with(|| left_index.cmp(right_index))
            })
            .map(|(index, _)| index)
    }

    pub fn select_at(&mut self, y_ratio: f32) -> Option<usize> {
        let index = self.hover_at(y_ratio);
        self.selection = index;
        index
    }

    pub fn jump_to(&mut self, index: usize) -> f32 {
        let Some(marker) = self.markers.get(index) else {
            return 0.0;
        };
        self.selection = Some(index);
        let height = self.viewport_height();
        if height >= 1.0 {
            return 0.0;
        }
        match self.viewport {
            None => marker.offset_ratio,
            Some(_) => (marker.offset_ratio - height / 2.0).clamp(0.0, 1.0 - height),
        }
    }

    pub fn viewport_to(&mut self, region: (f32, f32)) -> (f32, f32) {
        let (start, end) = region;
        let start = start.clamp(0.0, 1.0);
        let end = end.clamp(0.0, 1.0);
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        self.viewport = Some((start, end));
        (start, end)
    }

    pub fn viewport_height(&self) -> f32 {
        self.viewport
            .map(|(start, end)| (end - start).clamp(0.0, 1.0))
            .unwrap_or(0.0)
    }

    pub fn max_weight(&self) -> u32 {
        self.markers
            .iter()
            .map(|marker| marker.weight)
            .max()
            .unwrap_or(0)
    }
}

pub fn marker_color(weight: u32, max_weight: u32) -> u32 {
    let ratio = if max_weight == 0 {
        0.0
    } else {
        (weight as f32 / max_weight as f32).clamp(0.0, 1.0)
    };
    mix(ACCENT_DIM, ACCENT_BRIGHT, ratio)
}

fn mix(from: u32, to: u32, ratio: f32) -> u32 {
    let channel = |shift: u32| -> u32 {
        let from_channel = (from >> shift) & 0xff;
        let to_channel = (to >> shift) & 0xff;
        (from_channel as f32 + (to_channel as f32 - from_channel as f32) * ratio).round() as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

type JumpCallback = Rc<dyn Fn(f32, &mut App)>;

pub struct OverviewView {
    state: OverviewState,
    on_jump: JumpCallback,
    pressed: bool,
}

impl OverviewView {
    pub fn new(on_jump: impl Fn(f32, &mut App) + 'static) -> Self {
        Self {
            state: OverviewState::new(),
            on_jump: Rc::new(on_jump),
            pressed: false,
        }
    }

    pub fn state(&self) -> &OverviewState {
        &self.state
    }

    pub fn set_markers(&mut self, markers: Vec<HunkMarker>) {
        self.state.set_markers(markers);
    }

    pub fn set_viewport(&mut self, region: (f32, f32)) {
        self.state.viewport_to(region);
    }

    fn strip_height(window: &Window) -> f32 {
        window.viewport_size().height.to_f64() as f32
    }

    fn y_ratio(y: f32, height: f32) -> f32 {
        (y / height).clamp(0.0, 1.0)
    }

    fn fire_jump(&mut self, index: usize, cx: &mut Context<Self>) {
        let ratio = self.state.jump_to(index);
        (self.on_jump)(ratio, cx);
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let height = Self::strip_height(window);
        let ratio = Self::y_ratio(event.position.y.to_f64() as f32, height);
        if let Some(index) = self.state.select_at(ratio) {
            self.pressed = true;
            self.fire_jump(index, cx);
        }
        cx.notify();
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let height = Self::strip_height(window);
        let ratio = Self::y_ratio(event.position.y.to_f64() as f32, height);
        let hovered = self.state.hover_at(ratio);
        let mut changed = hovered != self.state.hovered;
        self.state.hovered = hovered;
        if event.dragging() && self.pressed {
            if let Some(index) = self.state.select_at(ratio) {
                self.fire_jump(index, cx);
                changed = true;
            } else if self.state.selection.is_some() {
                self.state.selection = None;
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }

    fn on_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.pressed = false;
    }

    fn on_hover(&mut self, hovered: &bool, _window: &mut Window, cx: &mut Context<Self>) {
        if !hovered && self.state.hovered.is_some() {
            self.state.hovered = None;
            cx.notify();
        }
    }

    fn tick(&self, marker: &HunkMarker, index: usize, max_weight: u32, height: f32) -> Div {
        let weight_ratio = if max_weight == 0 {
            0.0
        } else {
            marker.weight as f32 / max_weight as f32
        };
        let tick_height = TICK_MIN_H + weight_ratio * (TICK_MAX_H - TICK_MIN_H);
        let top = (marker.offset_ratio * height - tick_height / 2.0)
            .clamp(0.0, (height - tick_height).max(0.0));
        let highlighted = self.state.hovered == Some(index);
        let width = if highlighted {
            TICK_WIDTH_HOVERED
        } else {
            TICK_WIDTH
        };
        let color = if highlighted {
            ACCENT_BRIGHT
        } else {
            marker_color(marker.weight, max_weight)
        };
        div()
            .absolute()
            .top(px(top))
            .left(px((STRIP_WIDTH - width) / 2.0))
            .w(px(width))
            .h(px(tick_height))
            .rounded_sm()
            .bg(rgb(color))
    }
}

impl Render for OverviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = Self::strip_height(window);
        let max_weight = self.state.max_weight();
        let mut strip = div()
            .id("overview-strip")
            .relative()
            .flex_none()
            .w(px(STRIP_WIDTH))
            .h_full()
            .bg(rgb(OVERVIEW_BG))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_hover(cx.listener(Self::on_hover));
        if let Some((start, end)) = self.state.viewport {
            strip = strip.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(start * height))
                    .h(px((end - start) * height))
                    .bg(rgba(VIEWPORT_BAND)),
            );
        }
        for (index, marker) in self.state.markers.iter().enumerate() {
            strip = strip.child(self.tick(marker, index, max_weight, height));
        }
        strip
    }
}

#[cfg(test)]
mod tests {
    use super::{
        marker_color, HunkMarker, OverviewState, ACCENT_BRIGHT, ACCENT_DIM, PICKUP_RADIUS,
    };

    fn marker(offset_ratio: f32, weight: u32) -> HunkMarker {
        HunkMarker::new(offset_ratio, weight)
    }

    fn assert_ratio(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "ratio {actual} != {expected}"
        );
    }

    fn assert_band(viewport: Option<(f32, f32)>, start: f32, end: f32) {
        match viewport {
            Some((actual_start, actual_end)) => {
                assert_ratio(actual_start, start);
                assert_ratio(actual_end, end);
            }
            None => panic!("viewport band is None, expected ({start}, {end})"),
        }
    }

    fn luminance(hex: u32) -> f32 {
        let red = ((hex >> 16) & 0xff) as f32 / 255.0;
        let green = ((hex >> 8) & 0xff) as f32 / 255.0;
        let blue = (hex & 0xff) as f32 / 255.0;
        0.2126 * red + 0.7152 * green + 0.0722 * blue
    }

    #[test]
    fn set_markers_clamps_offsets_and_drops_stale_indices() {
        let mut state = OverviewState::new();
        state.hovered = Some(2);
        state.selection = Some(2);
        state.set_markers(vec![
            HunkMarker {
                offset_ratio: -0.5,
                weight: 1,
            },
            HunkMarker {
                offset_ratio: 1.7,
                weight: 2,
            },
        ]);
        assert_ratio(state.markers[0].offset_ratio, 0.0);
        assert_ratio(state.markers[1].offset_ratio, 1.0);
        assert_eq!(state.hovered, None);
        assert_eq!(state.selection, None);
        state.hovered = Some(1);
        state.selection = Some(1);
        state.set_markers(vec![marker(0.1, 1), marker(0.9, 2)]);
        assert_eq!(state.hovered, Some(1));
        assert_eq!(state.selection, Some(1));
    }

    #[test]
    fn hover_at_picks_the_nearest_marker_within_radius() {
        let mut state = OverviewState::new();
        state.set_markers(vec![marker(0.1, 1), marker(0.5, 2), marker(0.9, 3)]);
        assert_eq!(state.hover_at(0.5), Some(1));
        assert_eq!(
            state.hover_at(0.46875),
            Some(1),
            "radius boundary is inclusive"
        );
        assert_eq!(state.hover_at(0.4375), None, "past the radius misses");
        assert_eq!(
            state.hover_at(0.53125),
            Some(1),
            "radius boundary is inclusive"
        );
        assert_eq!(state.hover_at(0.55), None, "between markers misses");
    }

    #[test]
    fn hover_at_out_of_range_y_and_empty_markers_return_none() {
        let mut state = OverviewState::new();
        state.set_markers(vec![marker(0.25, 1), marker(0.75, 2)]);
        assert_eq!(state.hover_at(2.0), None);
        assert_eq!(state.hover_at(-0.5), None);
        assert_eq!(state.hover_at(0.25), Some(0));
        assert_eq!(OverviewState::new().hover_at(0.5), None);
    }

    #[test]
    fn hover_at_ties_resolve_to_the_lower_index() {
        let mut state = OverviewState::new();
        state.set_markers(vec![marker(0.5, 1), marker(0.5, 2)]);
        assert_eq!(state.hover_at(0.5), Some(0));
    }

    #[test]
    fn select_at_sets_selection_only_within_radius() {
        let mut state = OverviewState::new();
        state.set_markers(vec![marker(0.25, 1), marker(0.75, 2)]);
        assert_eq!(state.select_at(0.75), Some(1));
        assert_eq!(state.selection, Some(1));
        assert_eq!(state.select_at(0.5), None);
        assert_eq!(state.selection, None);
    }

    #[test]
    fn jump_to_centers_the_marker_within_the_viewport() {
        let mut state = OverviewState::new();
        state.set_markers(vec![marker(0.0625, 1), marker(0.5, 2), marker(0.9375, 3)]);
        state.viewport_to((0.25, 0.5));
        assert_ratio(state.jump_to(1), 0.375);
        assert_eq!(state.selection, Some(1));
        assert_ratio(state.jump_to(0), 0.0);
        assert_ratio(state.jump_to(2), 0.75);
    }

    #[test]
    fn jump_to_without_viewport_returns_the_marker_offset() {
        let mut state = OverviewState::new();
        state.set_markers(vec![marker(0.25, 1)]);
        assert_ratio(state.jump_to(0), 0.25);
        assert_eq!(state.selection, Some(0));
    }

    #[test]
    fn jump_to_full_file_viewport_requests_no_movement() {
        let mut state = OverviewState::new();
        state.set_markers(vec![marker(0.5, 1)]);
        state.viewport_to((1.5, -0.3));
        assert_ratio(state.jump_to(0), 0.0);
    }

    #[test]
    fn jump_to_out_of_range_index_requests_no_movement() {
        let mut state = OverviewState::new();
        state.set_markers(vec![marker(0.5, 1)]);
        state.selection = Some(0);
        assert_ratio(state.jump_to(7), 0.0);
        assert_eq!(
            state.selection,
            Some(0),
            "invalid jumps leave selection alone"
        );
        assert_ratio(state.jump_to(0), 0.5);
        assert_eq!(state.selection, Some(0));
    }

    #[test]
    fn viewport_to_clamps_orders_and_stores_the_band() {
        let mut state = OverviewState::new();
        let (start, end) = state.viewport_to((1.5, -0.3));
        assert_ratio(start, 0.0);
        assert_ratio(end, 1.0);
        assert_band(state.viewport, 0.0, 1.0);
        let (start, end) = state.viewport_to((-0.5, 0.25));
        assert_ratio(start, 0.0);
        assert_ratio(end, 0.25);
        let (start, end) = state.viewport_to((0.75, 0.25));
        assert_ratio(start, 0.25);
        assert_ratio(end, 0.75);
        assert_band(state.viewport, 0.25, 0.75);
    }

    #[test]
    fn viewport_height_tracks_the_stored_band() {
        let mut state = OverviewState::new();
        assert_ratio(state.viewport_height(), 0.0);
        state.viewport_to((0.25, 0.5));
        assert_ratio(state.viewport_height(), 0.25);
    }

    #[test]
    fn marker_offsets_clamp_at_construction() {
        assert_ratio(marker(-0.25, 1).offset_ratio, 0.0);
        assert_ratio(marker(1.25, 1).offset_ratio, 1.0);
    }

    #[test]
    fn marker_color_ramps_from_dim_to_bright_with_weight() {
        assert_eq!(marker_color(0, 0), ACCENT_DIM);
        assert_eq!(marker_color(0, 5), ACCENT_DIM);
        assert_eq!(marker_color(5, 5), ACCENT_BRIGHT);
        assert_eq!(marker_color(9, 5), ACCENT_BRIGHT, "weight above max clamps");
        let mut previous = 0.0;
        for weight in 0..=5 {
            let brightness = luminance(marker_color(weight, 5));
            assert!(brightness >= previous, "weight {weight} got dimmer");
            previous = brightness;
        }
    }

    #[test]
    fn pickup_radius_is_a_friendly_fraction() {
        assert_eq!(PICKUP_RADIUS, 0.03125);
    }
}
