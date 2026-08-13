use gpui::prelude::*;
use gpui::{
    div, px, rgb, App, Context, Div, FocusHandle, Focusable, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Window,
};

const STRIP_HEIGHT: f32 = 28.0;
const SLOT_PX: f32 = 18.0;
const DOT_PX: f32 = 8.0;
const SELECTED_DOT_PX: f32 = 12.0;
const DOT_MAX_PX: f32 = 16.0;
const MAGNITUDE_CLAMP: u64 = 5_000;

const STRIP_BG: u32 = 0x11141a;
const STRIP_BORDER: u32 = 0x2f3644;
const ACCENT: u32 = 0xff8c42;
const FOCUS_RING: u32 = 0x5fd0e8;
const DOT_OLDEST: u32 = 0x3a435c;
const HOT_WHITE: u32 = 0xfff2e0;
const EMPTY_TEXT: u32 = 0x67748f;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub subject: String,
    pub magnitude: u64,
}

impl Commit {
    pub fn new(sha: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            sha: sha.into(),
            subject: subject.into(),
            magnitude: 0,
        }
    }

    pub fn with_magnitude(
        sha: impl Into<String>,
        subject: impl Into<String>,
        magnitude: u64,
    ) -> Self {
        Self {
            sha: sha.into(),
            subject: subject.into(),
            magnitude,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScrubberState {
    commits: Vec<Commit>,
    selected: usize,
    focus: usize,
    focus_enabled: bool,
}

impl ScrubberState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.commits.is_empty()
    }

    pub fn len(&self) -> usize {
        self.commits.len()
    }

    pub fn commits(&self) -> &[Commit] {
        &self.commits
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn focus(&self) -> usize {
        self.focus
    }

    pub fn focus_enabled(&self) -> bool {
        self.focus_enabled
    }

    pub fn set_commits(&mut self, commits: Vec<Commit>) {
        let selected_sha = self.selected_sha().map(str::to_owned);
        let focus_sha = self.focus_sha().map(str::to_owned);
        let old_selected = self.selected;
        let old_focus = self.focus;
        self.commits = commits;
        self.selected = 0;
        self.focus = 0;
        if let Some(sha) = selected_sha {
            if let Some(index) = self.commits.iter().position(|commit| commit.sha == sha) {
                self.selected = index;
            } else {
                self.selected = old_selected.min(self.commits.len().saturating_sub(1));
            }
        }
        if let Some(sha) = focus_sha {
            if let Some(index) = self.commits.iter().position(|commit| commit.sha == sha) {
                self.focus = index;
            } else {
                self.focus = old_focus.min(self.commits.len().saturating_sub(1));
            }
        }
    }

    pub fn next(&mut self) {
        if !self.commits.is_empty() {
            self.selected = (self.selected + 1) % self.commits.len();
        }
    }

    pub fn prev(&mut self) {
        if !self.commits.is_empty() {
            self.selected = (self.selected + self.commits.len() - 1) % self.commits.len();
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.commits.is_empty() {
            return;
        }
        let target = self.selected as isize + delta;
        self.selected = target.clamp(0, self.commits.len() as isize - 1) as usize;
    }

    pub fn select(&mut self, index: usize) {
        if !self.commits.is_empty() {
            self.selected = index.min(self.commits.len() - 1);
        }
    }

    pub fn set_focus(&mut self, index: usize) {
        if !self.commits.is_empty() {
            self.focus = index.min(self.commits.len() - 1);
        }
    }

    pub fn toggle_focus_ring(&mut self) {
        self.focus_enabled = !self.focus_enabled;
    }

    pub fn selected_commit(&self) -> Option<&Commit> {
        self.commits.get(self.selected)
    }

    pub fn selected_sha(&self) -> Option<&str> {
        self.selected_commit().map(|commit| commit.sha.as_str())
    }

    pub fn focus_sha(&self) -> Option<&str> {
        self.commits
            .get(self.focus)
            .map(|commit| commit.sha.as_str())
    }
}

fn age_color(index: usize, len: usize) -> u32 {
    if len == 0 {
        return DOT_OLDEST;
    }
    let t = if len == 1 {
        1.0
    } else {
        1.0 - index as f32 / (len - 1) as f32
    };
    heat_color(t)
}

fn heat_color(t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        lerp(DOT_OLDEST, ACCENT, t * 2.0)
    } else {
        lerp(ACCENT, HOT_WHITE, (t - 0.5) * 2.0)
    }
}

fn dot_size(magnitude: u64) -> f32 {
    if magnitude == 0 {
        return DOT_PX;
    }
    let clamped = magnitude.min(MAGNITUDE_CLAMP) as f32;
    let t = clamped.ln_1p() / (MAGNITUDE_CLAMP as f32).ln_1p();
    DOT_PX + (DOT_MAX_PX - DOT_PX) * t
}

fn lerp(from: u32, to: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let channel = |shift: u32| {
        let a = (from >> shift) & 0xff;
        let b = (to >> shift) & 0xff;
        let value = a as f32 + (b as f32 - a as f32) * t;
        ((value.round() as u32) & 0xff) << shift
    };
    channel(16) | channel(8) | channel(0)
}

fn drag_target(anchor_index: usize, anchor_x: f32, x: f32, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let delta = ((x - anchor_x) / SLOT_PX).round() as isize;
    let target = anchor_index as isize + delta;
    Some(target.clamp(0, len as isize - 1) as usize)
}

#[derive(Debug, Clone, Copy)]
struct DragAnchor {
    index: usize,
    x: f32,
}

pub type ConfirmCallback = Box<dyn Fn(&mut Context<ScrubberView>)>;
pub type SelectCallback = Box<dyn Fn(usize, &mut Context<ScrubberView>)>;

pub struct ScrubberView {
    state: ScrubberState,
    focus_handle: FocusHandle,
    drag: Option<DragAnchor>,
    on_confirm: Option<ConfirmCallback>,
    on_select: Option<SelectCallback>,
}

impl ScrubberView {
    pub fn new(
        cx: &mut Context<Self>,
        on_confirm: Option<ConfirmCallback>,
        on_select: Option<SelectCallback>,
    ) -> Self {
        Self {
            state: ScrubberState::new(),
            focus_handle: cx.focus_handle(),
            drag: None,
            on_confirm,
            on_select,
        }
    }

    pub fn set_commits(&mut self, commits: Vec<Commit>, cx: &mut Context<Self>) {
        self.state.set_commits(commits);
        cx.notify();
    }

    pub fn state(&self) -> &ScrubberState {
        &self.state
    }

    pub fn selected_sha(&self) -> Option<&str> {
        self.state.selected_sha()
    }

    pub fn set_focus(&mut self, index: usize, cx: &mut Context<Self>) {
        self.state.set_focus(index);
        cx.notify();
    }

    pub fn toggle_focus_ring(&mut self, cx: &mut Context<Self>) {
        self.state.toggle_focus_ring();
        cx.notify();
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(callback) = &self.on_confirm {
            callback(cx);
        }
    }

    fn fire_select(&self, cx: &mut Context<Self>) {
        if let Some(callback) = &self.on_select {
            callback(self.state.selected(), cx);
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        match key {
            "left" if !self.state.is_empty() => {
                let before = self.state.selected();
                self.state.prev();
                if self.state.selected() != before {
                    self.fire_select(cx);
                }
            }
            "right" if !self.state.is_empty() => {
                let before = self.state.selected();
                self.state.next();
                if self.state.selected() != before {
                    self.fire_select(cx);
                }
            }
            "enter" | "return" => {
                self.confirm(cx);
                return;
            }
            _ => return,
        }
        cx.notify();
    }

    fn begin_drag(&mut self, index: usize, x: f32, cx: &mut Context<Self>) {
        let changed = index != self.state.selected();
        self.state.select(index);
        if changed {
            self.fire_select(cx);
        }
        self.drag = Some(DragAnchor { index, x });
        cx.stop_propagation();
        cx.notify();
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging() {
            return;
        }
        let Some(anchor) = self.drag else {
            return;
        };
        let Some(target) = drag_target(
            anchor.index,
            anchor.x,
            event.position.x.into(),
            self.state.len(),
        ) else {
            return;
        };
        if target != self.state.selected {
            self.state.select(target);
            self.fire_select(cx);
            cx.notify();
        }
    }

    fn on_mouse_up(&mut self, _event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.stop_propagation();
        }
    }

    fn dot(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let selected = index == self.state.selected;
        let focused = self.state.focus_enabled && index == self.state.focus;
        let mut slot = div()
            .w(px(SLOT_PX))
            .h(px(SLOT_PX))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    this.begin_drag(index, event.position.x.into(), cx);
                }),
            );
        if focused {
            slot = slot.border_1().border_color(rgb(FOCUS_RING));
        }
        let magnitude = self.state.commits[index].magnitude;
        let size = if selected {
            dot_size(magnitude).max(SELECTED_DOT_PX)
        } else {
            dot_size(magnitude)
        };
        let mut dot = div()
            .w(px(size))
            .h(px(size))
            .rounded_full()
            .bg(rgb(age_color(index, self.state.len())));
        if selected {
            dot = dot.border_2().border_color(rgb(ACCENT));
        }
        slot.child(dot)
    }
}

impl Focusable for ScrubberView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ScrubberView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut strip = div()
            .id("scrubber")
            .track_focus(&self.focus_handle)
            .w_full()
            .h(px(STRIP_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(STRIP_BG))
            .border_b_1()
            .border_color(rgb(STRIP_BORDER))
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up));
        if self.state.is_empty() {
            strip = strip.child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(EMPTY_TEXT))
                    .child("No commits"),
            );
        } else {
            let dots: Vec<Div> = (0..self.state.len())
                .map(|index| self.dot(index, cx))
                .collect();
            strip = strip.children(dots);
        }
        strip
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(sha: &str, subject: &str) -> Commit {
        Commit::new(sha, subject)
    }

    fn history() -> Vec<Commit> {
        vec![
            commit("aaa", "newest"),
            commit("bbb", "middle"),
            commit("ccc", "oldest"),
        ]
    }

    #[test]
    fn set_commits_preserves_selection_by_sha() {
        let mut state = ScrubberState::new();
        state.set_commits(history());
        state.next();
        assert_eq!(state.selected_sha(), Some("bbb"));
        state.set_commits(vec![
            commit("zzz", "rewritten head"),
            commit("aaa", "a"),
            commit("bbb", "b"),
            commit("ccc", "c"),
        ]);
        assert_eq!(state.selected_sha(), Some("bbb"), "sha survives a rewrite");
        assert_eq!(state.selected(), 2);
    }

    #[test]
    fn set_commits_clamps_when_the_selected_sha_disappears() {
        let mut state = ScrubberState::new();
        state.set_commits(vec![
            commit("aaa", "a"),
            commit("bbb", "b"),
            commit("ccc", "c"),
            commit("ddd", "d"),
            commit("eee", "e"),
        ]);
        state.select(3);
        state.set_commits(vec![commit("fff", "f"), commit("ggg", "g")]);
        assert_eq!(state.selected(), 1, "clamped to the last slot");
        assert_eq!(state.selected_sha(), Some("ggg"));
    }

    #[test]
    fn set_commits_resets_everything_on_an_empty_list() {
        let mut state = ScrubberState::new();
        state.set_commits(history());
        state.next();
        state.set_focus(2);
        state.set_commits(Vec::new());
        assert_eq!(state.selected_commit(), None);
        assert_eq!(state.focus_sha(), None);
        assert_eq!((state.selected(), state.focus()), (0, 0));
        assert!(state.is_empty());
    }

    #[test]
    fn next_and_prev_wrap_around() {
        let mut state = ScrubberState::new();
        state.set_commits(history());
        state.next();
        state.next();
        state.next();
        assert_eq!(
            state.selected_sha(),
            Some("aaa"),
            "next wraps to the newest"
        );
        state.prev();
        assert_eq!(
            state.selected_sha(),
            Some("ccc"),
            "prev wraps to the oldest"
        );
        state.prev();
        assert_eq!(state.selected_sha(), Some("bbb"));
    }

    #[test]
    fn navigation_is_a_noop_on_an_empty_state() {
        let mut state = ScrubberState::new();
        state.next();
        state.prev();
        state.move_by(5);
        state.select(7);
        state.set_focus(3);
        assert_eq!(state.selected(), 0);
        assert_eq!(state.focus(), 0);
    }

    #[test]
    fn move_by_clamps_at_both_ends() {
        let mut state = ScrubberState::new();
        state.set_commits(history());
        state.move_by(10);
        assert_eq!(
            state.selected_sha(),
            Some("ccc"),
            "overflow clamps at the oldest"
        );
        state.move_by(-10);
        assert_eq!(
            state.selected_sha(),
            Some("aaa"),
            "underflow clamps at the newest"
        );
        state.move_by(1);
        assert_eq!(
            state.selected_sha(),
            Some("bbb"),
            "mid-range moves normally"
        );
    }

    #[test]
    fn select_clamps_to_the_last_commit() {
        let mut state = ScrubberState::new();
        state.set_commits(history());
        state.select(99);
        assert_eq!(state.selected(), 2);
    }

    #[test]
    fn focus_ring_is_independent_of_selection() {
        let mut state = ScrubberState::new();
        state.set_commits(history());
        state.set_focus(2);
        state.toggle_focus_ring();
        assert!(state.focus_enabled());
        state.next();
        state.next();
        assert_eq!(state.selected_sha(), Some("ccc"));
        assert_eq!(state.focus_sha(), Some("ccc"));
        assert_eq!(state.focus(), 2, "selection moves, focus stays");
        state.toggle_focus_ring();
        assert!(!state.focus_enabled(), "ring toggles off");
        assert_eq!(state.focus(), 2, "toggling never moves the focus index");
        assert_eq!(state.selected(), 2, "toggling never moves the selection");
    }

    #[test]
    fn focus_defaults_to_the_newest_commit() {
        let mut state = ScrubberState::new();
        state.set_commits(history());
        assert_eq!(state.focus(), 0);
        assert_eq!(state.focus_sha(), Some("aaa"));
    }

    #[test]
    fn focus_survives_a_refresh_by_sha() {
        let mut state = ScrubberState::new();
        state.set_commits(history());
        state.set_focus(2);
        state.set_commits(vec![
            commit("zzz", "z"),
            commit("ccc", "c"),
            commit("aaa", "a"),
            commit("bbb", "b"),
        ]);
        assert_eq!(state.focus_sha(), Some("ccc"));
    }

    #[test]
    fn age_color_is_white_hot_for_the_newest_commit() {
        assert_eq!(age_color(0, 5), HOT_WHITE);
        assert_eq!(age_color(4, 5), DOT_OLDEST);
        assert_eq!(age_color(0, 1), HOT_WHITE, "a single commit is the newest");
        assert_eq!(age_color(0, 0), DOT_OLDEST, "an empty strip renders dim");
        assert_eq!(age_color(2, 5), ACCENT, "the strip midpoint burns ember");
        let luminance = |color: u32| {
            let red = (color >> 16) & 0xff;
            let green = (color >> 8) & 0xff;
            let blue = color & 0xff;
            (0.2126 * red as f32 + 0.7152 * green as f32 + 0.0722 * blue as f32) as u32
        };
        for index in 1..5 {
            let newer = age_color(index - 1, 5);
            let older = age_color(index, 5);
            assert!(
                (newer >> 16) & 0xff >= (older >> 16) & 0xff,
                "red dims with age at index {index}"
            );
            assert!(
                luminance(newer) >= luminance(older),
                "luminance dims with age at index {index}"
            );
        }
    }

    #[test]
    fn heat_color_walks_blue_to_ember_to_white() {
        assert_eq!(heat_color(0.0), DOT_OLDEST);
        assert_eq!(heat_color(0.5), ACCENT);
        assert_eq!(heat_color(1.0), HOT_WHITE);
        assert_eq!(heat_color(-1.0), DOT_OLDEST, "cold clamps");
        assert_eq!(heat_color(2.0), HOT_WHITE, "hot clamps");
        let blue = heat_color(0.25);
        assert!(
            (blue >> 16) & 0xff < (ACCENT >> 16) & 0xff,
            "the cool half stays below ember red"
        );
        let hot = heat_color(0.75);
        assert!(hot >= ACCENT, "the hot half burns at or past ember");
        assert_eq!(hot, lerp(ACCENT, HOT_WHITE, 0.5));
    }

    #[test]
    fn dot_size_follows_a_clamped_log_curve() {
        assert_eq!(
            dot_size(0),
            DOT_PX,
            "unknown magnitude renders the base dot"
        );
        assert!(dot_size(1) > DOT_PX);
        assert!(dot_size(10) > dot_size(5));
        assert!(dot_size(100) > dot_size(10));
        assert!(dot_size(1_000) > dot_size(100));
        assert_eq!(
            dot_size(MAGNITUDE_CLAMP),
            DOT_MAX_PX,
            "the clamp reaches the largest dot"
        );
        assert_eq!(
            dot_size(MAGNITUDE_CLAMP * 10),
            DOT_MAX_PX,
            "oversized commits stay clamped"
        );
        assert!(dot_size(10) - dot_size(5) < dot_size(500) - dot_size(250));
    }

    #[test]
    fn commit_magnitude_defaults_to_zero() {
        let plain = Commit::new("aaa", "plain");
        assert_eq!(plain.magnitude, 0, "legacy construction stays cold");
        let hot = Commit::with_magnitude("aaa", "hot", 42);
        assert_eq!(hot.magnitude, 42);
        assert_eq!(hot.sha, "aaa");
        assert_eq!(hot.subject, "hot");
    }

    #[test]
    fn drag_target_follows_the_pointer_by_slots() {
        assert_eq!(drag_target(1, 100.0, 100.0 + SLOT_PX, 3), Some(2));
        assert_eq!(drag_target(1, 100.0, 100.0 - SLOT_PX, 3), Some(0));
        assert_eq!(
            drag_target(1, 100.0, 100.0 - 3.0 * SLOT_PX, 3),
            Some(0),
            "clamps at the newest end"
        );
        assert_eq!(
            drag_target(1, 100.0, 100.0 + 50.0 * SLOT_PX, 3),
            Some(2),
            "clamps at the oldest end"
        );
        assert_eq!(
            drag_target(0, 0.0, 0.0 + SLOT_PX / 3.0, 3),
            Some(0),
            "sub-slot jitter stays put"
        );
        assert_eq!(
            drag_target(0, 0.0, 0.0, 0),
            None,
            "empty strip has no target"
        );
    }
}
