use std::time::{Duration, Instant};

use gpui::*;
use qol_config::placement::Spot;

use super::super::components::{
    display_layout_ghost, display_layout_stage, display_layout_tile, placement_caption,
    placement_toast, DisplayLayoutTile, PlacementHold, SettingsHint,
};
use super::super::rows::RowControl;
use super::SettingsPanelView;
use crate::key::Key;

pub(super) const PLACEMENT_HOLD: Duration = Duration::from_secs(2);
const HOLD_JITTER: f32 = 4.0;
const STAGE_PAD: f32 = qol_theme::SPACE_INSET;
const SCREEN_INSET: f32 = qol_theme::SPACE_SNUG;
const PREVIEW_SHARE: f32 = 0.3;
const PREVIEW_MIN_WIDTH: f32 = 96.0;
const PREVIEW_RATIO: f32 = 0.3;
const FALLBACK_ASPECT: f32 = 16.0 / 9.0;
const SCREEN_LABEL: &str = "Active display";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PlacementStage {
    screen: [f32; 4],
    preview: (f32, f32),
}

impl PlacementStage {
    pub(super) fn fit(width: f32, height: f32, aspect: f32) -> Self {
        let aspect = if aspect.is_finite() && aspect > 0.0 {
            aspect
        } else {
            FALLBACK_ASPECT
        };
        let room_width = (width - 2.0 * STAGE_PAD).max(1.0);
        let room_height = (height - 2.0 * STAGE_PAD).max(1.0);
        let (screen_width, screen_height) = if room_width / room_height > aspect {
            (room_height * aspect, room_height)
        } else {
            (room_width, room_width / aspect)
        };
        let preview_width = (screen_width * PREVIEW_SHARE)
            .max(PREVIEW_MIN_WIDTH)
            .min((screen_width - 2.0 * SCREEN_INSET).max(1.0));
        Self {
            screen: [
                (width - screen_width) / 2.0,
                (height - screen_height) / 2.0,
                screen_width,
                screen_height,
            ],
            preview: (preview_width, preview_width * PREVIEW_RATIO),
        }
    }

    fn travel(self) -> (f32, f32) {
        let [_, _, width, height] = self.screen;
        (
            (width - 2.0 * SCREEN_INSET - self.preview.0).max(0.0),
            (height - 2.0 * SCREEN_INSET - self.preview.1).max(0.0),
        )
    }

    pub(super) fn preview_at(self, spot: Spot) -> [f32; 4] {
        let [left, top, _, _] = self.screen;
        let (across, down) = self.travel();
        [
            left + SCREEN_INSET + spot.x * across,
            top + SCREEN_INSET + spot.y * down,
            self.preview.0,
            self.preview.1,
        ]
    }

    pub(super) fn dragged(self, from: Spot, dx: f32, dy: f32) -> Spot {
        let (across, down) = self.travel();
        Spot::new(from.x + share(dx, across), from.y + share(dy, down))
    }

    pub(super) fn spot_under(self, x: f32, y: f32) -> Spot {
        let [left, top, _, _] = self.screen;
        let (across, down) = self.travel();
        Spot::new(
            share(x - left - SCREEN_INSET - self.preview.0 / 2.0, across),
            share(y - top - SCREEN_INSET - self.preview.1 / 2.0, down),
        )
    }
}

fn share(offset: f32, room: f32) -> f32 {
    if room > 0.0 {
        offset / room
    } else {
        0.0
    }
}

pub(super) fn snapped(spot: Spot) -> Spot {
    let half = |value: f32| (value * 2.0).round() / 2.0;
    Spot::new(half(spot.x), half(spot.y))
}

pub(super) fn stepped(spot: Spot, across: i32, down: i32) -> Spot {
    Spot::new(step_axis(spot.x, across), step_axis(spot.y, down))
}

fn step_axis(value: f32, direction: i32) -> f32 {
    const STOPS: [f32; 3] = [0.0, 0.5, 1.0];
    const EPSILON: f32 = 0.001;
    match direction.signum() {
        1 => STOPS
            .into_iter()
            .find(|stop| *stop > value + EPSILON)
            .unwrap_or(1.0),
        -1 => STOPS
            .into_iter()
            .rev()
            .find(|stop| *stop < value - EPSILON)
            .unwrap_or(0.0),
        _ => value,
    }
}

pub(super) fn spot_label(spot: Spot) -> String {
    let label = match spot.name() {
        Some("top-left") => "Top left",
        Some("top-center") => "Top center",
        Some("top-right") => "Top right",
        Some("center-left") => "Middle left",
        Some("center") => "Center",
        Some("center-right") => "Middle right",
        Some("bottom-left") => "Bottom left",
        Some("bottom-center") => "Bottom center",
        Some("bottom-right") => "Bottom right",
        _ => "Free",
    };
    label.to_string()
}

fn spot_caption(spot: Spot) -> String {
    match spot.name() {
        Some(_) => spot_label(spot),
        None => format!(
            "Free, {}% across and {}% down",
            (spot.x * 100.0).round(),
            (spot.y * 100.0).round()
        ),
    }
}

pub(super) fn placement_hints() -> Vec<SettingsHint> {
    vec![
        SettingsHint::new(Key::DRAG, "move"),
        SettingsHint::new(Key::ARROWS, "snap"),
        SettingsHint::new(Key::ENTER, "done"),
    ]
}

pub(super) struct PlacementDrag {
    origin: Point<Pixels>,
    from: Spot,
    pointer: Spot,
    free: bool,
    still_at: Point<Pixels>,
    still_since: Instant,
    still_epoch: u64,
    press: u64,
}

impl PlacementDrag {
    fn new(position: Point<Pixels>, from: Spot, press: u64, now: Instant) -> Self {
        Self {
            origin: position,
            from,
            pointer: from,
            free: false,
            still_at: position,
            still_since: now,
            still_epoch: 0,
            press,
        }
    }

    fn landing(&self) -> Spot {
        if self.free {
            self.pointer
        } else {
            snapped(self.pointer)
        }
    }

    fn settle(&mut self, position: Point<Pixels>, now: Instant) -> bool {
        if self.free {
            return false;
        }
        let drift_x = f32::from(position.x - self.still_at.x);
        let drift_y = f32::from(position.y - self.still_at.y);
        if drift_x.hypot(drift_y) <= HOLD_JITTER {
            return false;
        }
        self.still_at = position;
        self.still_since = now;
        self.still_epoch += 1;
        true
    }

    fn hold_left(&mut self, now: Instant) -> Option<Duration> {
        if self.free {
            return None;
        }
        let held = now.saturating_duration_since(self.still_since);
        if held >= PLACEMENT_HOLD {
            self.free = true;
            return None;
        }
        Some(PLACEMENT_HOLD - held)
    }
}

impl SettingsPanelView {
    pub(super) fn placement_spot(&self) -> Option<Spot> {
        match self
            .level()
            .rows
            .get(self.level().selected)
            .map(|row| &row.control)
        {
            Some(RowControl::Placement(spot)) => Some(*spot),
            _ => None,
        }
    }

    fn placement_geometry(&self) -> PlacementStage {
        let (width, height) = match self.placement_stage.get() {
            Some(bounds) if bounds.size.width > px(0.0) && bounds.size.height > px(0.0) => {
                (f32::from(bounds.size.width), f32::from(bounds.size.height))
            }
            _ => (
                super::super::PANEL_WIDE_WIDTH - 2.0 * qol_theme::SPACE_PAD,
                super::super::PANEL_PLACEMENT_HEIGHT,
            ),
        };
        PlacementStage::fit(width, height, self.screen_aspect)
    }

    pub(super) fn on_placement_card_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(spot) = self.placement_spot() else {
            return false;
        };
        let step = match key {
            "left" => Some((-1, 0)),
            "right" => Some((1, 0)),
            "up" => Some((0, -1)),
            "down" => Some((0, 1)),
            _ => None,
        };
        if let Some((across, down)) = step {
            self.placement_drag = None;
            self.place_notifications(stepped(spot, across, down));
            cx.notify();
            return true;
        }
        match key {
            "escape" if self.placement_drag.is_some() => {
                self.placement_drag = None;
                cx.notify();
                true
            }
            "enter" | "return" | "space" => {
                self.placement_drag = None;
                self.pop_card(cx);
                true
            }
            _ => false,
        }
    }

    fn begin_placement_drag(
        &mut self,
        position: Point<Pixels>,
        jump: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(spot) = self.placement_spot() else {
            return;
        };
        let from = match (jump, self.placement_stage.get()) {
            (true, Some(bounds)) => self.placement_geometry().spot_under(
                f32::from(position.x - bounds.origin.x),
                f32::from(position.y - bounds.origin.y),
            ),
            _ => spot,
        };
        self.placement_presses += 1;
        self.placement_drag = Some(PlacementDrag::new(
            position,
            from,
            self.placement_presses,
            Instant::now(),
        ));
        self.arm_placement_hold(PLACEMENT_HOLD, cx);
        cx.notify();
    }

    fn arm_placement_hold(&self, wait: Duration, cx: &mut Context<Self>) {
        let Some(press) = self.placement_drag.as_ref().map(|drag| drag.press) else {
            return;
        };
        cx.spawn(async move |view, cx| {
            cx.background_executor().timer(wait).await;
            let _ = view.update(cx, |view, cx| view.placement_hold_elapsed(press, cx));
        })
        .detach();
    }

    fn placement_hold_elapsed(&mut self, press: u64, cx: &mut Context<Self>) {
        let Some(drag) = self
            .placement_drag
            .as_mut()
            .filter(|drag| drag.press == press && !drag.free)
        else {
            return;
        };
        match drag.hold_left(Instant::now()) {
            Some(wait) => self.arm_placement_hold(wait, cx),
            None => cx.notify(),
        }
    }

    fn drag_placement_to(&mut self, position: Point<Pixels>) -> bool {
        let geometry = self.placement_geometry();
        let Some(drag) = self.placement_drag.as_mut() else {
            return false;
        };
        let mut changed = drag.settle(position, Instant::now());
        let pointer = geometry.dragged(
            drag.from,
            f32::from(position.x - drag.origin.x),
            f32::from(position.y - drag.origin.y),
        );
        changed |= pointer != drag.pointer;
        drag.pointer = pointer;
        changed
    }

    fn finish_placement_drag(&mut self) -> bool {
        let Some(drag) = self.placement_drag.take() else {
            return false;
        };
        self.place_notifications(drag.landing());
        true
    }

    fn place_notifications(&mut self, spot: Spot) {
        let selected = self.level().selected;
        let Some(RowControl::Placement(current)) = self
            .level_mut()
            .rows
            .get_mut(selected)
            .map(|row| &mut row.control)
        else {
            return;
        };
        if *current == spot {
            return;
        }
        *current = spot;
        if self.stack.len() > 1 && self.level().live_card {
            self.sync_live_card_to_root();
        }
        self.persist();
    }

    pub(super) fn render_placement(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let Some(RowControl::Placement(spot)) =
            self.level().rows.get(index).map(|row| &row.control)
        else {
            return div();
        };
        let spot = *spot;
        let kit = self.kit;
        let geometry = self.placement_geometry();
        let drag = self.placement_drag.as_ref();
        let landing = drag.map_or(spot, PlacementDrag::landing);
        let shown = drag.map_or(spot, |drag| drag.pointer);
        let free = drag.is_some_and(|drag| drag.free);
        let measured = std::rc::Rc::clone(&self.placement_stage);
        let entity = cx.weak_entity();
        let drag_entity = entity.clone();
        let stage_bounds = canvas(
            move |bounds, window, _| {
                if measured.get() == Some(bounds) {
                    return;
                }
                measured.set(Some(bounds));
                let entity = entity.clone();
                window.on_next_frame(move |_, cx| {
                    let _ = entity.update(cx, |_, cx| cx.notify());
                });
            },
            move |_, _, window, _| watch_placement_drag(window, drag_entity.clone()),
        )
        .absolute()
        .inset_0();
        let [left, top, width, height] = geometry.screen;
        let screen = DisplayLayoutTile {
            connector: SCREEN_LABEL.to_string(),
            resolution: String::new(),
            left,
            top,
            width,
            height,
            selected: false,
            primary: false,
            conflicted: false,
        };
        let mut stage = display_layout_stage(kit, super::super::PANEL_PLACEMENT_HEIGHT)
            .child(stage_bounds)
            .child(
                display_layout_tile(0, &screen, kit)
                    .cursor(CursorStyle::PointingHand)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.begin_placement_drag(event.position, true, cx);
                        }),
                    ),
            );
        if drag.is_some() && !free {
            let [left, top, width, height] = geometry.preview_at(landing);
            stage = stage.child(display_layout_ghost(left, top, width, height, kit));
        }
        let hold = match drag {
            None => PlacementHold::Resting,
            Some(drag) if drag.free => PlacementHold::Free,
            Some(drag) => PlacementHold::Charging {
                press: drag.press,
                epoch: drag.still_epoch,
                hold: PLACEMENT_HOLD,
            },
        };
        let [left, top, width, height] = geometry.preview_at(shown);
        stage = stage.child(
            placement_toast(hold, kit)
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width))
                .h(px(height))
                .cursor(if drag.is_some() {
                    CursorStyle::ClosedHand
                } else {
                    CursorStyle::OpenHand
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.begin_placement_drag(event.position, false, cx);
                    }),
                ),
        );
        let caption = match drag {
            None => format!(
                "{}. Drags snap to the corners, edges and center. Hold still for two seconds to place it freely.",
                spot_caption(spot)
            ),
            Some(drag) if drag.free => {
                format!("{}. Release to place it here.", spot_caption(landing))
            }
            Some(_) => format!(
                "Snaps to {}. Hold still to place it freely.",
                spot_label(landing).to_lowercase()
            ),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(qol_theme::SPACE_TIGHT))
            .w_full()
            .child(stage)
            .child(placement_caption(caption, kit))
    }
}

fn watch_placement_drag(window: &mut Window, view: WeakEntity<SettingsPanelView>) {
    let move_view = view.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase != DispatchPhase::Capture {
            return;
        }
        let _ = move_view.update(cx, |panel, cx| {
            if panel.placement_drag.is_none() {
                return;
            }
            if event.dragging() {
                if panel.drag_placement_to(event.position) {
                    cx.notify();
                }
            } else {
                panel.finish_placement_drag();
                cx.notify();
            }
        });
    });
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
            return;
        }
        let _ = view.update(cx, |panel, cx| {
            if panel.finish_placement_drag() {
                cx.notify();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use qol_config::placement::Spot;

    use super::{
        snapped, spot_label, stepped, PlacementDrag, PlacementStage, PLACEMENT_HOLD, SCREEN_INSET,
    };

    fn stage() -> PlacementStage {
        PlacementStage::fit(560.0, 280.0, 16.0 / 9.0)
    }

    #[test]
    fn the_screen_keeps_the_display_shape_inside_the_stage() {
        let stage = stage();
        let [left, top, width, height] = stage.screen;
        assert!((width / height - 16.0 / 9.0).abs() < 0.01);
        assert!(left >= 0.0 && top >= 0.0);
        assert!(left + width <= 560.0 && top + height <= 280.0);
        let wide = PlacementStage::fit(560.0, 280.0, 0.0);
        assert!((wide.screen[2] / wide.screen[3] - 16.0 / 9.0).abs() < 0.01);
    }

    #[test]
    fn the_preview_reaches_every_corner_of_the_screen() {
        let stage = stage();
        let [left, top, width, height] = stage.screen;
        let [x, y, _, _] = stage.preview_at(Spot::new(0.0, 0.0));
        assert_eq!((x, y), (left + SCREEN_INSET, top + SCREEN_INSET));
        let [x, y, w, h] = stage.preview_at(Spot::new(1.0, 1.0));
        assert!((x + w - (left + width - SCREEN_INSET)).abs() < 0.01);
        assert!((y + h - (top + height - SCREEN_INSET)).abs() < 0.01);
    }

    #[test]
    fn dragging_moves_the_spot_by_the_share_of_the_room() {
        let stage = stage();
        let start = Spot::new(1.0, 0.0);
        let [x0, y0, _, _] = stage.preview_at(start);
        let [x1, y1, _, _] = stage.preview_at(Spot::new(0.5, 0.5));
        let moved = stage.dragged(start, x1 - x0, y1 - y0);
        assert!((moved.x - 0.5).abs() < 0.002 && (moved.y - 0.5).abs() < 0.002);
        assert_eq!(
            stage.dragged(start, 10_000.0, -10_000.0),
            Spot::new(1.0, 0.0)
        );
        let [x, y, w, h] = stage.preview_at(Spot::new(0.25, 0.75));
        let under = stage.spot_under(x + w / 2.0, y + h / 2.0);
        assert!((under.x - 0.25).abs() < 0.002 && (under.y - 0.75).abs() < 0.002);
    }

    #[test]
    fn a_drag_snaps_to_the_nearest_of_nine_spots_until_it_is_released() {
        let cases = [
            ((0.1, 0.2), "top-left"),
            ((0.3, 0.1), "top-center"),
            ((0.9, 0.24), "top-right"),
            ((0.2, 0.5), "center-left"),
            ((0.6, 0.4), "center"),
            ((0.8, 0.6), "center-right"),
            ((0.0, 0.9), "bottom-left"),
            ((0.55, 0.8), "bottom-center"),
            ((0.76, 0.76), "bottom-right"),
        ];
        for ((x, y), name) in cases {
            assert_eq!(snapped(Spot::new(x, y)).name(), Some(name), "{x},{y}");
        }
        let start = gpui::point(gpui::px(0.0), gpui::px(0.0));
        let mut drag = PlacementDrag::new(start, Spot::TOP_RIGHT, 1, Instant::now());
        assert_eq!(drag.landing(), Spot::TOP_RIGHT);
        drag.pointer = Spot::new(0.37, 0.82);
        assert_eq!(drag.landing().name(), Some("bottom-center"));
        drag.free = true;
        assert_eq!(drag.landing(), Spot::new(0.37, 0.82));
    }

    #[test]
    fn holding_still_for_two_seconds_releases_the_snap() {
        let at = |x: f32, y: f32| gpui::point(gpui::px(x), gpui::px(y));
        let start = Instant::now();
        let after = |millis: u64| start + Duration::from_millis(millis);
        let mut drag = PlacementDrag::new(at(10.0, 10.0), Spot::TOP_RIGHT, 1, start);
        assert_eq!(
            drag.hold_left(after(500)),
            Some(Duration::from_millis(1500))
        );
        assert!(!drag.settle(at(12.0, 12.0), after(1000)));
        assert!(drag.settle(at(40.0, 10.0), after(1500)));
        assert_eq!(
            drag.hold_left(after(2500)),
            Some(Duration::from_millis(1000))
        );
        assert!(!drag.free);
        assert_eq!(drag.hold_left(after(3500)), None);
        assert!(drag.free);
        assert!(!drag.settle(at(400.0, 10.0), after(4000)));
        assert!(drag.free);
        assert_eq!(PLACEMENT_HOLD, Duration::from_secs(2));
    }

    #[test]
    fn arrows_step_to_the_next_snapping_spot() {
        let top_right = Spot::TOP_RIGHT;
        assert_eq!(stepped(top_right, 1, 0), top_right);
        assert_eq!(stepped(top_right, -1, 0).name(), Some("top-center"));
        assert_eq!(stepped(top_right, 0, 1).name(), Some("center-right"));
        assert_eq!(stepped(top_right, 0, -1), top_right);
        let free = Spot::new(0.37, 0.82);
        assert_eq!(stepped(free, -1, 0), Spot::new(0.0, 0.82));
        assert_eq!(stepped(free, 1, 0), Spot::new(0.5, 0.82));
        assert_eq!(stepped(free, 0, 1), Spot::new(0.37, 1.0));
        assert_eq!(stepped(free, 0, -1), Spot::new(0.37, 0.5));
    }

    #[test]
    fn every_snapping_spot_has_a_name_and_the_rest_read_as_free() {
        let labels: Vec<String> = Spot::snaps().map(spot_label).collect();
        assert_eq!(labels.len(), 9);
        assert!(labels.iter().all(|label| label != "Free"));
        let mut unique = labels.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 9);
        assert_eq!(spot_label(Spot::new(0.37, 0.82)), "Free");
    }
}
