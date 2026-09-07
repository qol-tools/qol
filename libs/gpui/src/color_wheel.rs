use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::*;

const DISC_SIZE: f32 = 200.0;
const POPUP_SIZE: f32 = DISC_SIZE + 2.0 * (WHEEL_PADDING + WHEEL_BORDER);
const DISC_RADIUS: f64 = DISC_SIZE as f64 / 2.0 - 1.0;
const THUMB_SIZE: f32 = 16.0;
const NUDGE_STEP: f64 = 6.0;
const NUDGE_STEP_FAST: f64 = 18.0;
const WHEEL_PADDING: f32 = 8.0;
const WHEEL_BORDER: f32 = 1.0;
const WINDOW_MARGIN: f32 = 8.0;

#[derive(Clone, Copy)]
pub struct WheelStyle {
    pub bg: u32,
    pub border: u32,
    pub thumb_border: u32,
}

type WheelCallback = Box<dyn FnMut(String, &mut App)>;

pub struct WheelCallbacks {
    preview: WheelCallback,
    commit: WheelCallback,
}

impl WheelCallbacks {
    pub fn new(
        preview: impl FnMut(String, &mut App) + 'static,
        commit: impl FnMut(String, &mut App) + 'static,
    ) -> Self {
        Self {
            preview: Box::new(preview),
            commit: Box::new(commit),
        }
    }
}

pub struct ColorWheelPopup {
    wheel: ColorWheel,
    style: WheelStyle,
    on_preview: WheelCallback,
    on_commit: WheelCallback,
    parent_window: AnyWindowHandle,
    parent_focus: FocusHandle,
    focus_handle: FocusHandle,
    finished: bool,
}

pub struct ColorWheel {
    hue: f64,
    sat: f64,
    image: Arc<RenderImage>,
    disc_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    dragging: bool,
}

impl ColorWheelPopup {
    pub fn open(
        wheel: ColorWheel,
        style: WheelStyle,
        anchor: Bounds<Pixels>,
        parent_window: &mut Window,
        parent_focus: FocusHandle,
        callbacks: WheelCallbacks,
        cx: &mut App,
    ) -> Option<WindowHandle<Self>> {
        let parent_bounds = parent_window.window_bounds().get_bounds();
        let display = parent_window.display(cx);
        let display_bounds = display
            .as_ref()
            .map(|display| display.bounds())
            .unwrap_or(parent_bounds);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(popup_bounds(
                anchor,
                parent_bounds,
                display_bounds,
            ))),
            titlebar: None,
            focus: true,
            kind: WindowKind::Floating,
            is_movable: false,
            display_id: display.map(|display| display.id()),
            window_background: WindowBackgroundAppearance::Transparent,
            window_decorations: Some(WindowDecorations::Client),
            app_id: Some("qol-color-wheel".into()),
            ..Default::default()
        };
        let parent_handle = parent_window.window_handle();
        cx.open_window(options, move |window, cx| {
            let view = cx.new(|cx| Self {
                wheel,
                style,
                on_preview: callbacks.preview,
                on_commit: callbacks.commit,
                parent_window: parent_handle,
                parent_focus,
                focus_handle: cx.focus_handle(),
                finished: false,
            });
            view.update(cx, |view, cx| view.observe_blur(window, cx));
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            view
        })
        .ok()
    }

    fn observe_blur(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.on_blur(&self.focus_handle, window, Self::on_blur)
            .detach();
    }

    fn on_blur(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish(window, cx);
    }

    pub(super) fn handle_key(
        &mut self,
        key: &str,
        fast: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(intent) = wheel_intent(key) else {
            return;
        };
        match intent {
            WheelIntent::Nudge(dx, dy) => {
                self.wheel.nudge(f64::from(dx), f64::from(dy), fast);
                (self.on_preview)(self.wheel.hex(), cx);
                cx.notify();
            }
            WheelIntent::Commit => self.finish(window, cx),
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.wheel.begin_drag(event.position) {
            return;
        }
        (self.on_preview)(self.wheel.hex(), cx);
        cx.stop_propagation();
        cx.notify();
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging() || !self.wheel.drag_to(event.position) {
            return;
        }
        (self.on_preview)(self.wheel.hex(), cx);
        cx.notify();
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.wheel.finish_drag(event.position) {
            return;
        }
        cx.stop_propagation();
        self.finish(window, cx);
    }

    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.finished {
            return;
        }
        self.finished = true;
        (self.on_commit)(self.wheel.hex(), cx);
        window.remove_window();
        let parent_window = self.parent_window;
        let parent_focus = self.parent_focus.clone();
        cx.defer(move |cx| {
            let _ = parent_window.update(cx, |_, window, _| {
                window.focus(&parent_focus);
                window.activate_window();
            });
        });
    }
}

impl Focusable for ColorWheelPopup {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ColorWheelPopup {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("qol-color-wheel-popup")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_key(
                    event.keystroke.key.as_str(),
                    event.keystroke.modifiers.shift,
                    window,
                    cx,
                );
            }))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .size_full()
            .child(
                self.wheel
                    .render(self.style, cx.listener(Self::on_mouse_down)),
            )
    }
}

impl ColorWheel {
    pub fn open(current: &str) -> Self {
        let (hue, sat) = hex_to_hue_sat(current);
        Self {
            hue,
            sat,
            image: disc_image(),
            disc_bounds: Rc::new(Cell::new(None)),
            dragging: false,
        }
    }

    fn nudge(&mut self, dx: f64, dy: f64, fast: bool) {
        let step = if fast { NUDGE_STEP_FAST } else { NUDGE_STEP };
        let (hue, sat) = nudged(self.hue, self.sat, dx * step, dy * step);
        self.hue = hue;
        self.sat = sat;
    }

    pub fn hex(&self) -> String {
        format!("#{}", hue_sat_to_hex(self.hue, self.sat))
    }

    fn begin_drag(&mut self, position: Point<Pixels>) -> bool {
        let Some((x, y)) = self.local_pointer(position) else {
            return false;
        };
        if !pointer_hits_disc(x, y) {
            return false;
        }
        self.dragging = true;
        self.set_from_pointer(x, y);
        true
    }

    fn drag_to(&mut self, position: Point<Pixels>) -> bool {
        if !self.dragging {
            return false;
        }
        let Some((x, y)) = self.local_pointer(position) else {
            return false;
        };
        self.set_from_pointer(x, y);
        true
    }

    fn finish_drag(&mut self, position: Point<Pixels>) -> bool {
        if !self.drag_to(position) {
            return false;
        }
        self.dragging = false;
        true
    }

    fn local_pointer(&self, position: Point<Pixels>) -> Option<(f64, f64)> {
        let bounds = self.disc_bounds.get()?;
        Some((
            (position.x - bounds.origin.x).to_f64(),
            (position.y - bounds.origin.y).to_f64(),
        ))
    }

    fn set_from_pointer(&mut self, x: f64, y: f64) {
        let (hue, sat) = hue_sat_from_pointer(x, y);
        self.hue = hue;
        self.sat = sat;
    }

    fn thumb_color(&self) -> u32 {
        let [r, g, b] = hue_sat_to_rgb(self.hue, self.sat);
        u32::from_be_bytes([0, r, g, b])
    }

    fn thumb_position(&self) -> (f32, f32) {
        let center = DISC_SIZE as f64 / 2.0;
        let angle = self.hue.to_radians();
        let dist = self.sat * DISC_RADIUS;
        (
            (center + dist * angle.cos()) as f32,
            (center + dist * angle.sin()) as f32,
        )
    }

    fn render(
        &self,
        style: WheelStyle,
        on_mouse_down: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        let (tx, ty) = self.thumb_position();
        let disc_bounds = Rc::clone(&self.disc_bounds);
        div()
            .id("qol-color-wheel")
            .p(px(WHEEL_PADDING))
            .rounded_none()
            .border_1()
            .border_color(rgb(style.border))
            .shadow(crate::kit::float_shadow(style.thumb_border))
            .bg(rgb(style.bg))
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("qol-color-wheel-disc")
                    .relative()
                    .w(px(DISC_SIZE))
                    .h(px(DISC_SIZE))
                    .cursor(CursorStyle::Crosshair)
                    .on_mouse_down(MouseButton::Left, on_mouse_down)
                    .child(img(self.image.clone()).w(px(DISC_SIZE)).h(px(DISC_SIZE)))
                    .child(
                        canvas(
                            move |bounds, _, _| disc_bounds.set(Some(bounds)),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(px(tx - THUMB_SIZE / 2.0))
                            .top(px(ty - THUMB_SIZE / 2.0))
                            .w(px(THUMB_SIZE))
                            .h(px(THUMB_SIZE))
                            .rounded_none()
                            .border_2()
                            .border_color(rgb(style.thumb_border))
                            .bg(rgb(self.thumb_color())),
                    ),
            )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WheelIntent {
    Nudge(i8, i8),
    Commit,
}

fn wheel_intent(key: &str) -> Option<WheelIntent> {
    match key {
        "left" => Some(WheelIntent::Nudge(-1, 0)),
        "right" => Some(WheelIntent::Nudge(1, 0)),
        "up" => Some(WheelIntent::Nudge(0, -1)),
        "down" => Some(WheelIntent::Nudge(0, 1)),
        "enter" | "return" | "escape" => Some(WheelIntent::Commit),
        _ => None,
    }
}

fn popup_bounds(
    anchor: Bounds<Pixels>,
    parent: Bounds<Pixels>,
    display: Bounds<Pixels>,
) -> Bounds<Pixels> {
    let width = POPUP_SIZE;
    let height = POPUP_SIZE;
    let anchor_left = (parent.origin.x + anchor.origin.x).to_f64() as f32;
    let anchor_top = (parent.origin.y + anchor.origin.y).to_f64() as f32;
    let anchor_bottom = anchor_top + anchor.size.height.to_f64() as f32;
    let display_left = display.origin.x.to_f64() as f32 + WINDOW_MARGIN;
    let display_top = display.origin.y.to_f64() as f32 + WINDOW_MARGIN;
    let display_right = (display.origin.x + display.size.width).to_f64() as f32 - WINDOW_MARGIN;
    let display_bottom = (display.origin.y + display.size.height).to_f64() as f32 - WINDOW_MARGIN;
    let max_x = (display_right - width).max(display_left);
    let max_y = (display_bottom - height).max(display_top);
    let preferred_y = if anchor_bottom + height <= display_bottom {
        anchor_bottom
    } else {
        anchor_top - height
    };
    Bounds::new(
        point(
            px(anchor_left.clamp(display_left, max_x)),
            px(preferred_y.clamp(display_top, max_y)),
        ),
        size(px(width), px(height)),
    )
}

fn nudged(hue: f64, sat: f64, dx: f64, dy: f64) -> (f64, f64) {
    let angle = hue.to_radians();
    let dist = sat * DISC_RADIUS;
    let x = dist * angle.cos() + dx;
    let y = dist * angle.sin() + dy;
    hue_sat_from_cartesian(x, y)
}

fn pointer_hits_disc(x: f64, y: f64) -> bool {
    let center = DISC_SIZE as f64 / 2.0;
    let dx = x - center;
    let dy = y - center;
    (dx * dx + dy * dy).sqrt() <= center
}

fn hue_sat_from_pointer(x: f64, y: f64) -> (f64, f64) {
    let center = DISC_SIZE as f64 / 2.0;
    hue_sat_from_cartesian(x - center, y - center)
}

fn hue_sat_from_cartesian(x: f64, y: f64) -> (f64, f64) {
    let dist = (x * x + y * y).sqrt().min(DISC_RADIUS);
    let hue = (y.atan2(x).to_degrees() + 360.0) % 360.0;
    (hue, dist / DISC_RADIUS)
}

fn hue_components(h: f64) -> [f64; 3] {
    let x = 1.0 - (((h / 60.0) % 2.0) - 1.0).abs();
    if h < 60.0 {
        [1.0, x, 0.0]
    } else if h < 120.0 {
        [x, 1.0, 0.0]
    } else if h < 180.0 {
        [0.0, 1.0, x]
    } else if h < 240.0 {
        [0.0, x, 1.0]
    } else if h < 300.0 {
        [x, 0.0, 1.0]
    } else {
        [1.0, 0.0, x]
    }
}

fn hue_sat_to_rgb(h: f64, s: f64) -> [u8; 3] {
    hue_components(h).map(|c| ((1.0 - s + s * c) * 255.0).round() as u8)
}

fn hue_sat_to_hex(h: f64, s: f64) -> String {
    let [r, g, b] = hue_sat_to_rgb(h, s);
    format!("{r:02x}{g:02x}{b:02x}")
}

fn hex_to_hue_sat(hex: &str) -> (f64, f64) {
    let mut clean: String = hex.trim().trim_start_matches('#').chars().take(6).collect();
    clean.extend(std::iter::repeat_n('0', 6usize.saturating_sub(clean.len())));
    let channel = |i: usize| -> f64 {
        let pair = clean.get(i..i + 2).unwrap_or("00");
        u8::from_str_radix(pair, 16).unwrap_or(0) as f64 / 255.0
    };
    let (r, g, b) = (channel(0), channel(2), channel(4));
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let mut h = 0.0;
    if delta > 0.0 {
        if max == r {
            h = 60.0 * (((g - b) / delta + 6.0) % 6.0);
        } else if max == g {
            h = 60.0 * ((b - r) / delta + 2.0);
        } else {
            h = 60.0 * ((r - g) / delta + 4.0);
        }
    }
    let s = if max == 0.0 { 0.0 } else { delta / max };
    (h, s)
}

fn disc_bgra() -> Vec<u8> {
    let size = DISC_SIZE as usize;
    let center = size as f64 / 2.0;
    let radius = center - 1.0;
    let mut data = vec![0u8; size * size * 4];
    for py in 0..size {
        for px in 0..size {
            let dx = px as f64 - center;
            let dy = py as f64 - center;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist > radius {
                continue;
            }
            let h = (dy.atan2(dx).to_degrees() + 360.0) % 360.0;
            let [r, g, b] = hue_sat_to_rgb(h, dist / radius);
            let i = (py * size + px) * 4;
            data[i] = b;
            data[i + 1] = g;
            data[i + 2] = r;
            data[i + 3] = 255;
        }
    }
    data
}

fn disc_image() -> Arc<RenderImage> {
    let size = DISC_SIZE as u32;
    let buffer = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(size, size, disc_bgra())
        .expect("disc buffer dimensions are static");
    Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
        buffer
    )]))
}

#[cfg(test)]
mod tests {
    use gpui::{point, px, size, Bounds, Pixels};
    use proptest::prelude::*;

    use super::{
        disc_bgra, hex_to_hue_sat, hue_sat_from_pointer, hue_sat_to_hex, nudged, pointer_hits_disc,
        popup_bounds, wheel_intent, ColorWheel, WheelIntent, DISC_RADIUS, DISC_SIZE, NUDGE_STEP,
        NUDGE_STEP_FAST, POPUP_SIZE,
    };

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }

    #[test]
    fn hue_sat_hex_round_trips_primary_colors() {
        let cases = [
            (0.0, 1.0, "ff0000"),
            (120.0, 1.0, "00ff00"),
            (240.0, 1.0, "0000ff"),
            (0.0, 0.0, "ffffff"),
            (180.0, 0.5, "80ffff"),
        ];
        for (hue, sat, expected) in cases {
            assert_eq!(hue_sat_to_hex(hue, sat), expected, "hue {hue} sat {sat}");
            let (parsed_hue, parsed_sat) = hex_to_hue_sat(expected);
            let round = hue_sat_to_hex(parsed_hue, parsed_sat);
            assert_eq!(round, expected, "round trip for {expected}");
        }
    }

    #[test]
    fn hex_parse_tolerates_hash_short_and_invalid_input() {
        let cases = [
            ("#ff0000", (0.0, 1.0)),
            ("ff0000", (0.0, 1.0)),
            ("f", (0.0, 1.0)),
            ("zz", (0.0, 0.0)),
            ("", (0.0, 0.0)),
        ];
        for (hex, expected) in cases {
            assert_eq!(hex_to_hue_sat(hex), expected, "hex {hex:?}");
        }
    }

    #[test]
    fn dark_colors_open_as_the_web_wheels_bright_equivalent() {
        let cases = [
            ("202322", "e9fff8"),
            ("abcdef", "b6dbff"),
            ("000000", "ffffff"),
        ];
        for (stored, expected) in cases {
            let (hue, sat) = hex_to_hue_sat(stored);
            assert_eq!(hue_sat_to_hex(hue, sat), expected, "stored {stored}");
        }
    }

    #[test]
    fn nudged_moves_in_cartesian_space_and_clamps_to_disc() {
        let (hue, sat) = nudged(0.0, 0.0, 10.0, 0.0);
        assert_eq!(hue, 0.0, "moving right from center points at hue 0");
        assert!((sat - 10.0 / DISC_RADIUS).abs() < 1e-9);

        let (_, clamped) = nudged(0.0, 1.0, 1000.0, 0.0);
        assert_eq!(clamped, 1.0, "distance clamps to the disc radius");

        let (down_hue, _) = nudged(0.0, 0.0, 0.0, 10.0);
        assert_eq!(down_hue, 90.0, "moving down from center points at hue 90");
    }

    #[test]
    fn wheel_nudge_uses_the_shift_acceleration_step() {
        let mut normal = ColorWheel::open("ffffff");
        let mut fast = ColorWheel::open("ffffff");
        normal.nudge(1.0, 0.0, false);
        fast.nudge(1.0, 0.0, true);
        assert!((normal.sat - NUDGE_STEP / DISC_RADIUS).abs() < 1e-9);
        assert!((fast.sat - NUDGE_STEP_FAST / DISC_RADIUS).abs() < 1e-9);
    }

    #[test]
    fn wheel_intent_maps_cartesian_movement_and_commit_keys() {
        let cases = [
            ("left", Some(WheelIntent::Nudge(-1, 0))),
            ("right", Some(WheelIntent::Nudge(1, 0))),
            ("up", Some(WheelIntent::Nudge(0, -1))),
            ("down", Some(WheelIntent::Nudge(0, 1))),
            ("enter", Some(WheelIntent::Commit)),
            ("return", Some(WheelIntent::Commit)),
            ("escape", Some(WheelIntent::Commit)),
            ("space", None),
        ];
        for (key, expected) in cases {
            assert_eq!(wheel_intent(key), expected, "key {key}");
        }
    }

    #[test]
    fn popup_placement_anchors_flips_and_clamps_to_the_display() {
        let anchor = bounds(16.0, 80.0, 488.0, 36.0);
        let display = bounds(0.0, 0.0, 1000.0, 1080.0);
        let cases = [
            (bounds(200.0, 200.0, 520.0, 170.0), (216.0, 316.0)),
            (bounds(200.0, 800.0, 520.0, 170.0), (216.0, 662.0)),
            (bounds(900.0, 100.0, 520.0, 170.0), (774.0, 216.0)),
        ];
        for (parent, expected) in cases {
            let placed = popup_bounds(anchor, parent, display);
            assert_eq!(
                (placed.origin.x.to_f64(), placed.origin.y.to_f64()),
                (expected.0, expected.1),
                "parent {parent:?}"
            );
            assert_eq!(placed.size, size(px(POPUP_SIZE), px(POPUP_SIZE)));
        }
    }

    #[test]
    fn pointer_position_maps_center_cardinals_and_outside_to_the_disc() {
        let cases = [
            ((100.0, 100.0), (0.0, 0.0)),
            ((199.0, 100.0), (0.0, 1.0)),
            ((100.0, 199.0), (90.0, 1.0)),
            ((1.0, 100.0), (180.0, 1.0)),
            ((100.0, 1.0), (270.0, 1.0)),
            ((500.0, 100.0), (0.0, 1.0)),
        ];
        for ((x, y), (expected_hue, expected_sat)) in cases {
            let (hue, sat) = hue_sat_from_pointer(x, y);
            assert!((hue - expected_hue).abs() < 1e-9, "point {x},{y}");
            assert!((sat - expected_sat).abs() < 1e-9, "point {x},{y}");
        }
    }

    #[test]
    fn pointer_hit_test_accepts_the_disc_and_rejects_transparent_corners() {
        let cases = [
            ((100.0, 100.0), true),
            ((200.0, 100.0), true),
            ((0.0, 100.0), true),
            ((0.0, 0.0), false),
            ((200.0, 200.0), false),
        ];
        for ((x, y), expected) in cases {
            assert_eq!(pointer_hits_disc(x, y), expected, "point {x},{y}");
        }
    }

    #[test]
    fn disc_pixels_are_white_center_red_edge_transparent_corner() {
        let data = disc_bgra();
        let size = DISC_SIZE as usize;
        let at = |x: usize, y: usize| {
            let i = (y * size + x) * 4;
            (data[i], data[i + 1], data[i + 2], data[i + 3])
        };
        let center = at(size / 2, size / 2);
        assert_eq!(center, (255, 255, 255, 255), "center is white");
        let (b, g, r, a) = at(size - 3, size / 2);
        assert_eq!((r, a), (255, 255), "right edge is fully red");
        assert!(
            g < 20 && b < 20,
            "right edge has no green/blue: g={g} b={b}"
        );
        assert_eq!(at(0, 0).3, 0, "corner outside the disc is transparent");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn nudged_position_stays_on_the_disc(
            hue in 0.0f64..360.0,
            sat in 0.0f64..=1.0,
            dx in -1000.0f64..1000.0,
            dy in -1000.0f64..1000.0,
        ) {
            let (next_hue, next_sat) = nudged(hue, sat, dx, dy);
            prop_assert!((0.0..360.0).contains(&next_hue));
            prop_assert!((0.0..=1.0).contains(&next_sat));
        }
    }
}
