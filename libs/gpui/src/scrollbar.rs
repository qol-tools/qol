use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gpui::*;
use qol_theme::Motion;

const BAR_REST_WIDTH: f32 = 3.0;
const BAR_NEAR_WIDTH: f32 = qol_theme::SPACE_INSET;
const BAR_INSET: f32 = 2.0;
const BAR_THUMB_MIN: f32 = 24.0;
const BAR_REACH: f32 = qol_theme::SPACE_PAD;

pub const OVERFLOW_FADE_HEIGHT: f32 = qol_theme::HEIGHT_HINT_BAR;
pub const OVERFLOW_STREAK_WIDTH: f32 = 180.0;
pub const OVERFLOW_STREAK_HEIGHT: f32 = qol_theme::HEIGHT_INLINE - qol_theme::SPACE_TIGHT;
pub const OVERFLOW_CUE_CENTRE: f32 = OVERFLOW_FADE_HEIGHT / 2.0;
pub const OVERFLOW_CHEVRON_WIDTH: f32 = qol_theme::SPACE_PAD;
pub const OVERFLOW_CHEVRON_RISE: f32 = qol_theme::SPACE_SNUG;
pub const OVERFLOW_CHEVRON_STROKE: f32 = 2.0;

pub type Seek = Rc<dyn Fn(&f32, &mut Window, &mut App)>;

#[derive(Clone)]
pub enum ScrollSource {
    Handle {
        handle: ScrollHandle,
        children: usize,
    },
    Window {
        first: usize,
        shown: usize,
        total: usize,
        seek: Seek,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CueState {
    top: bool,
    bottom: bool,
    share: f32,
    at: f32,
}

impl ScrollSource {
    pub fn window(
        first: usize,
        shown: usize,
        total: usize,
        seek: impl Fn(&f32, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self::Window {
            first,
            shown,
            total,
            seek: Rc::new(seek),
        }
    }

    fn state(&self) -> Option<CueState> {
        match self {
            Self::Handle { handle, children } => {
                let max = handle.max_offset().height;
                if max <= px(0.) || *children == 0 {
                    return None;
                }
                let first = handle.bounds_for_item(0)?;
                let last = handle.bounds_for_item(children - 1)?;
                let viewport = handle.bounds();
                let (top, bottom) = overflow_edges(viewport, first, last, handle.offset().y);
                let view = viewport.size.height;
                Some(CueState {
                    top,
                    bottom,
                    share: view / (view + max),
                    at: (-handle.offset().y / max).clamp(0.0, 1.0),
                })
            }
            Self::Window {
                first,
                shown,
                total,
                ..
            } => window_state(*first, *shown, *total),
        }
    }

    fn seek(&self, at: f32, window: &mut Window, cx: &mut App) {
        match self {
            Self::Handle { handle, .. } => {
                let max = handle.max_offset().height;
                handle.set_offset(point(handle.offset().x, -max * at));
                window.refresh();
            }
            Self::Window { seek, .. } => seek(&at, window, cx),
        }
    }
}

fn window_state(first: usize, shown: usize, total: usize) -> Option<CueState> {
    if shown == 0 || shown >= total {
        return None;
    }
    let hidden = total - shown;
    Some(CueState {
        top: first > 0,
        bottom: first + shown < total,
        share: shown as f32 / total as f32,
        at: (first.min(hidden) as f32 / hidden as f32).clamp(0.0, 1.0),
    })
}

pub(crate) fn window_first(at: f32, shown: usize, total: usize) -> usize {
    let hidden = total.saturating_sub(shown);
    ((at.clamp(0.0, 1.0) * hidden as f32).round() as usize).min(hidden)
}

#[derive(Clone, Copy, Debug)]
struct CueInk {
    surface: u32,
    chevron: u32,
    streak: u32,
    track: u32,
    thumb: u32,
}

impl CueInk {
    fn of(ground: qol_theme::Ground) -> Self {
        Self {
            surface: ground.bg,
            chevron: qol_theme::translucent(ground.ink, qol_theme::Alpha::Strong),
            streak: qol_theme::translucent(ground.ink, qol_theme::Alpha::Wash),
            track: ground.edge.packed(),
            thumb: qol_theme::translucent(ground.ink, qol_theme::Alpha::Strong),
        }
    }
}

#[derive(IntoElement)]
pub struct ScrollCue {
    id: ElementId,
    source: ScrollSource,
    ink: CueInk,
}

pub fn scroll_cue(
    id: impl Into<ElementId>,
    source: ScrollSource,
    ground: qol_theme::Ground,
) -> ScrollCue {
    ScrollCue {
        id: id.into(),
        source,
        ink: CueInk::of(ground),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Bar {
    held: Option<f32>,
    near: bool,
    from: f32,
    since: Option<Instant>,
}

impl Bar {
    fn wide(&self) -> bool {
        self.near || self.held.is_some()
    }

    fn width(&self, now: Instant, moving: bool) -> (f32, bool) {
        let target = if self.wide() {
            BAR_NEAR_WIDTH
        } else {
            BAR_REST_WIDTH
        };
        let Some(since) = self.since else {
            return (target, false);
        };
        let motion = if self.wide() {
            Motion::QUICK
        } else {
            Motion::SETTLE
        };
        let elapsed = now.saturating_duration_since(since);
        let delta = (elapsed.as_secs_f32() / motion.duration.as_secs_f32()).min(1.0);
        let eased = crate::motion::ease_when(motion, delta, moving);
        (
            self.from + (target - self.from) * eased,
            delta < 1.0 && moving,
        )
    }

    fn shift(&mut self, now: Instant, moving: bool, change: impl FnOnce(&mut Self)) {
        let was = self.wide();
        let (width, _) = self.width(now, moving);
        change(self);
        if self.wide() != was {
            self.from = width;
            self.since = Some(now);
        }
    }
}

fn shift_bar(bar: &Entity<Bar>, cx: &mut App, change: impl FnOnce(&mut Bar)) {
    let now = Instant::now();
    let moving = crate::motion::moving();
    bar.update(cx, |bar, cx| {
        let before = *bar;
        bar.shift(now, moving, change);
        if *bar != before {
            cx.notify();
        }
    });
}

impl RenderOnce for ScrollCue {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let bar = window.use_keyed_state(self.id, cx, |_, _| Bar::default());
        let Some(state) = self.source.state() else {
            return div().into_any_element();
        };
        let ink = self.ink;
        let strip: Rc<Cell<Option<Bounds<Pixels>>>> = Rc::default();
        let measured = strip.clone();
        let painted = bar.clone();
        let dragged = self.source.clone();
        let pressed = self.source;
        div()
            .absolute()
            .inset_0()
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        if state.top {
                            paint_overflow_edge(window, bounds, OverflowEdge::Top, ink);
                        }
                        if state.bottom {
                            paint_overflow_edge(window, bounds, OverflowEdge::Bottom, ink);
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right_0()
                    .w(px(BAR_REACH))
                    .block_mouse_except_scroll()
                    .child(
                        canvas(
                            move |bounds, _, _| measured.set(Some(bounds)),
                            move |bounds, _, window, cx| {
                                let (width, moving) = painted
                                    .read(cx)
                                    .width(Instant::now(), crate::motion::moving());
                                paint_track(window, bounds, state, ink, width);
                                if moving {
                                    window.request_animation_frame();
                                }
                                watch_bar(window, bounds, state, painted, dragged);
                            },
                        )
                        .size_full(),
                    )
                    .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                        cx.stop_propagation();
                        let Some(bounds) = strip.get() else {
                            return;
                        };
                        let thumb = Thumb::of(f32::from(bounds.size.height), state);
                        let y = f32::from(event.position.y - bounds.top());
                        let grip = thumb.hold(y);
                        shift_bar(&bar, cx, |bar| bar.held = Some(grip));
                        pressed.seek(thumb.seek(y, grip), window, cx);
                    }),
            )
            .into_any_element()
    }
}

fn paint_track(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    state: CueState,
    ink: CueInk,
    width: f32,
) {
    let view_h = bounds.size.height;
    let track_x = bounds.right() - px(BAR_INSET + width);
    window.paint_quad(fill(
        Bounds::new(point(track_x, bounds.top()), size(px(width), view_h)),
        rgba(ink.track),
    ));
    let thumb = Thumb::of(f32::from(view_h), state);
    window.paint_quad(fill(
        Bounds::new(
            point(track_x, bounds.top() + px(thumb.top)),
            size(px(width), px(thumb.height)),
        ),
        rgba(ink.thumb),
    ));
}

fn watch_bar(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    state: CueState,
    bar: Entity<Bar>,
    source: ScrollSource,
) {
    let released = bar.clone();
    let left = bar.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
        if phase != DispatchPhase::Capture {
            return;
        }
        let near = bounds.contains(&event.position);
        let Some(grip) = bar.read(cx).held else {
            if bar.read(cx).near != near {
                shift_bar(&bar, cx, |bar| bar.near = near);
            }
            return;
        };
        if !event.dragging() {
            shift_bar(&bar, cx, |bar| {
                bar.held = None;
                bar.near = near;
            });
            return;
        }
        let thumb = Thumb::of(f32::from(bounds.size.height), state);
        source.seek(
            thumb.seek(f32::from(event.position.y - bounds.top()), grip),
            window,
            cx,
        );
    });
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
            let near = bounds.contains(&event.position);
            shift_bar(&released, cx, |bar| {
                bar.held = None;
                bar.near = near;
            });
        }
    });
    window.on_mouse_event(move |_: &MouseExitEvent, phase, _, cx| {
        if phase == DispatchPhase::Capture && left.read(cx).held.is_none() {
            shift_bar(&left, cx, |bar| bar.near = false);
        }
    });
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Thumb {
    top: f32,
    height: f32,
    travel: f32,
}

impl Thumb {
    fn of(view: f32, state: CueState) -> Self {
        let height = (view * state.share).max(BAR_THUMB_MIN).min(view);
        let travel = view - height;
        Self {
            top: travel * state.at,
            height,
            travel,
        }
    }

    fn hold(self, y: f32) -> f32 {
        if (self.top..=self.top + self.height).contains(&y) {
            y - self.top
        } else {
            self.height / 2.0
        }
    }

    fn seek(self, y: f32, held: f32) -> f32 {
        if self.travel <= 0.0 {
            return 0.0;
        }
        ((y - held) / self.travel).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy)]
pub enum OverflowEdge {
    Top,
    Bottom,
}

pub fn overflow_fade(edge: OverflowEdge, edge_ink: u32, clear: u32) -> Background {
    let (start, end) = match edge {
        OverflowEdge::Top => (edge_ink, clear),
        OverflowEdge::Bottom => (clear, edge_ink),
    };
    linear_gradient(
        180.0,
        linear_color_stop(rgba(start), 0.0),
        linear_color_stop(rgba(end), 1.0),
    )
}

pub fn overflow_edges(
    viewport: Bounds<Pixels>,
    first: Bounds<Pixels>,
    last: Bounds<Pixels>,
    offset: Pixels,
) -> (bool, bool) {
    let top = first.top() + offset < viewport.top() - px(1.);
    let bottom = last.bottom() + offset > viewport.bottom() + px(1.);
    (top, bottom)
}

fn paint_overflow_edge(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    edge: OverflowEdge,
    ink: CueInk,
) {
    let fade_h = px(OVERFLOW_FADE_HEIGHT).min(bounds.size.height);
    let rise = px(OVERFLOW_CHEVRON_RISE);
    let (band_top, centre_y, tip_y, wing_y) = match edge {
        OverflowEdge::Top => {
            let centre_y = bounds.top() + px(OVERFLOW_CUE_CENTRE);
            (
                bounds.top(),
                centre_y,
                centre_y - rise * 0.5,
                centre_y + rise * 0.5,
            )
        }
        OverflowEdge::Bottom => {
            let centre_y = bounds.bottom() - px(OVERFLOW_CUE_CENTRE);
            (
                bounds.bottom() - fade_h,
                centre_y,
                centre_y + rise * 0.5,
                centre_y - rise * 0.5,
            )
        }
    };
    window.paint_quad(fill(
        Bounds::new(
            point(bounds.left(), band_top),
            size(bounds.size.width, fade_h),
        ),
        overflow_fade(
            edge,
            qol_theme::solid(ink.surface),
            qol_theme::clear(ink.surface),
        ),
    ));
    let clear = ink.streak & 0xffff_ff00;
    let half_streak = px(OVERFLOW_STREAK_WIDTH / 2.0);
    let streak_size = size(half_streak, px(OVERFLOW_STREAK_HEIGHT));
    let streak_top = centre_y - px(OVERFLOW_STREAK_HEIGHT / 2.0);
    let center_x = bounds.center().x;
    window.paint_quad(fill(
        Bounds::new(point(center_x - half_streak, streak_top), streak_size),
        linear_gradient(
            90.0,
            linear_color_stop(rgba(clear), 0.0),
            linear_color_stop(rgba(ink.streak), 1.0),
        ),
    ));
    window.paint_quad(fill(
        Bounds::new(point(center_x, streak_top), streak_size),
        linear_gradient(
            90.0,
            linear_color_stop(rgba(ink.streak), 0.0),
            linear_color_stop(rgba(clear), 1.0),
        ),
    ));
    paint_chevron(window, center_x, tip_y, wing_y, rgba(ink.chevron));
}

fn paint_chevron(window: &mut Window, center_x: Pixels, tip_y: Pixels, wing_y: Pixels, ink: Rgba) {
    let half_chevron = px(OVERFLOW_CHEVRON_WIDTH / 2.0);
    let mut path = PathBuilder::stroke(px(OVERFLOW_CHEVRON_STROKE));
    path.move_to(point(center_x - half_chevron, wing_y));
    path.line_to(point(center_x, tip_y));
    path.line_to(point(center_x + half_chevron, wing_y));
    if let Ok(path) = path.build() {
        window.paint_path(path, ink);
    }
}

pub fn chevron(edge: OverflowEdge, ink: u32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let centre_y = bounds.center().y;
            let half_rise = px(OVERFLOW_CHEVRON_RISE / 2.0);
            let (tip_y, wing_y) = match edge {
                OverflowEdge::Top => (centre_y - half_rise, centre_y + half_rise),
                OverflowEdge::Bottom => (centre_y + half_rise, centre_y - half_rise),
            };
            paint_chevron(window, bounds.center().x, tip_y, wing_y, rgb(ink));
        },
    )
    .flex_none()
    .w(px(OVERFLOW_CHEVRON_WIDTH))
    .h(px(OVERFLOW_CHEVRON_RISE + OVERFLOW_CHEVRON_STROKE))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{
        overflow_edges, window_first, window_state, Bar, Thumb, BAR_NEAR_WIDTH, BAR_REACH,
        BAR_REST_WIDTH, BAR_THUMB_MIN, OVERFLOW_CHEVRON_RISE, OVERFLOW_CHEVRON_STROKE,
        OVERFLOW_CUE_CENTRE, OVERFLOW_FADE_HEIGHT, OVERFLOW_STREAK_HEIGHT, OVERFLOW_STREAK_WIDTH,
    };
    use gpui::{point, px, size, Bounds, Pixels};
    use qol_theme::Motion;

    fn viewport() -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(0.)), size(px(300.), px(400.)))
    }

    fn child(top: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(top)), size(px(300.), px(height)))
    }

    #[test]
    fn overflow_edges_stay_dark_at_rest() {
        assert_eq!(
            overflow_edges(viewport(), child(0., 40.), child(900., 52.), px(0.)),
            (false, true),
            "only the last child past the bottom lights"
        );
        assert_eq!(
            overflow_edges(viewport(), child(0., 40.), child(300., 52.), px(0.)),
            (false, false),
            "both edges stay dark when every child fits"
        );
    }

    #[test]
    fn overflow_edges_ignore_bottom_padding() {
        assert_eq!(
            overflow_edges(viewport(), child(0., 40.), child(332., 52.), px(0.)),
            (false, false),
            "padding below the last child does not light the bottom"
        );
        assert_eq!(
            overflow_edges(viewport(), child(0., 40.), child(348., 52.), px(0.)),
            (false, false),
            "a last child ending exactly at the viewport bottom stays dark"
        );
        assert_eq!(
            overflow_edges(viewport(), child(0., 40.), child(348., 52.), px(2.)),
            (false, true),
            "two pixels of the last child cut off light the bottom"
        );
    }

    #[test]
    fn overflow_edges_light_the_top_once_the_first_child_is_cut() {
        assert_eq!(
            overflow_edges(viewport(), child(0., 40.), child(300., 52.), px(-1.)),
            (false, false),
            "one pixel of overlap stays dark"
        );
        assert_eq!(
            overflow_edges(viewport(), child(0., 40.), child(300., 52.), px(-2.)),
            (true, false),
            "two pixels of the first child cut off light the top"
        );
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn the_cue_fits_inside_its_band() {
        assert!(OVERFLOW_STREAK_HEIGHT <= OVERFLOW_FADE_HEIGHT);
        assert_eq!(OVERFLOW_CUE_CENTRE, OVERFLOW_FADE_HEIGHT / 2.0);
        assert!(OVERFLOW_CUE_CENTRE + OVERFLOW_STREAK_HEIGHT / 2.0 <= OVERFLOW_FADE_HEIGHT);
        assert!(OVERFLOW_CHEVRON_RISE + OVERFLOW_CHEVRON_STROKE <= OVERFLOW_STREAK_HEIGHT);
        assert_eq!((OVERFLOW_STREAK_WIDTH / 2.0).fract(), 0.0);
    }

    #[test]
    fn a_windowed_list_shows_more_on_each_hidden_side() {
        assert_eq!(window_state(0, 9, 9), None);
        let first = window_state(0, 9, 109).expect("more below");
        assert!(!first.top && first.bottom);
        let middle = window_state(50, 9, 109).expect("more both ways");
        assert!(middle.top && middle.bottom);
        let last = window_state(100, 9, 109).expect("more above");
        assert!(last.top && !last.bottom);
        assert_eq!(last.at, 1.0);
    }
    #[test]
    fn a_press_on_the_thumb_drags_it_from_where_it_was_held() {
        let state = window_state(50, 10, 110).expect("more both ways");
        let thumb = Thumb::of(400.0, state);
        assert_eq!(thumb.height, 400.0 * 10.0 / 110.0);
        let inside = thumb.top + 5.0;
        let held = thumb.hold(inside);
        assert_eq!(held, 5.0);
        assert!((thumb.seek(inside, held) - state.at).abs() < 1e-6);
        assert_eq!(thumb.seek(-100.0, held), 0.0);
        assert_eq!(thumb.seek(900.0, held), 1.0);
    }

    #[test]
    fn a_press_on_the_track_centres_the_thumb_under_the_pointer() {
        let state = window_state(0, 10, 1000).expect("more below");
        let thumb = Thumb::of(400.0, state);
        assert_eq!(thumb.height, BAR_THUMB_MIN);
        let held = thumb.hold(200.0);
        assert_eq!(held, BAR_THUMB_MIN / 2.0);
        let at = thumb.seek(200.0, held);
        assert!((at - (200.0 - BAR_THUMB_MIN / 2.0) / (400.0 - BAR_THUMB_MIN)).abs() < 1e-6);
        let moved = Thumb::of(400.0, super::CueState { at, ..state });
        assert!((moved.top + moved.height / 2.0 - 200.0).abs() < 1e-3);
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn the_bar_widens_inside_its_reach_and_stays_on_the_ladder() {
        assert_eq!(BAR_REST_WIDTH, 3.0);
        assert_eq!(BAR_NEAR_WIDTH, qol_theme::SPACE_INSET);
        assert_eq!(BAR_REACH, qol_theme::SPACE_PAD);
        assert!(BAR_NEAR_WIDTH < BAR_REACH);
    }

    #[test]
    fn the_bar_grows_quickly_near_the_pointer_and_settles_back_when_it_leaves() {
        let start = Instant::now();
        let mut bar = Bar::default();
        assert_eq!(bar.width(start, true), (BAR_REST_WIDTH, false));
        bar.shift(start, true, |bar| bar.near = true);
        assert_eq!(bar.width(start, true), (BAR_REST_WIDTH, true));
        let (halfway, moving) = bar.width(start + Motion::QUICK.duration / 2, true);
        assert!(moving && halfway > BAR_REST_WIDTH && halfway < BAR_NEAR_WIDTH);
        let grown = start + Motion::QUICK.duration;
        assert_eq!(bar.width(grown, true), (BAR_NEAR_WIDTH, false));
        bar.shift(grown, true, |bar| bar.held = Some(4.0));
        bar.shift(grown, true, |bar| bar.near = false);
        assert_eq!(
            bar.width(grown + Duration::from_secs(1), true),
            (BAR_NEAR_WIDTH, false),
            "a held bar stays wide when the pointer leaves"
        );
        let let_go = grown + Duration::from_secs(1);
        bar.shift(let_go, true, |bar| bar.held = None);
        assert_eq!(bar.width(let_go, true), (BAR_NEAR_WIDTH, true));
        assert_eq!(
            bar.width(let_go + Motion::SETTLE.duration, true),
            (BAR_REST_WIDTH, false)
        );
    }

    #[test]
    fn the_bar_jumps_when_animations_are_off() {
        let start = Instant::now();
        let mut bar = Bar::default();
        bar.shift(start, false, |bar| bar.near = true);
        assert_eq!(bar.width(start, false), (BAR_NEAR_WIDTH, false));
        bar.shift(start, false, |bar| bar.near = false);
        assert_eq!(bar.width(start, false), (BAR_REST_WIDTH, false));
    }

    #[test]
    fn a_seek_lands_a_window_on_its_nearest_first_row() {
        assert_eq!(window_first(0.0, 10, 110), 0);
        assert_eq!(window_first(1.0, 10, 110), 100);
        assert_eq!(window_first(0.5, 10, 110), 50);
        assert_eq!(window_first(2.0, 10, 110), 100);
        assert_eq!(window_first(-1.0, 10, 110), 0);
        assert_eq!(window_first(0.7, 10, 5), 0);
    }
}
