use std::time::Instant;

use qol_theme::Motion;

pub(super) const CARD_WIDTH: f32 = qol_theme::toast::WIDTH;
pub(super) const CARD_HEIGHT: f32 = qol_theme::toast::HEIGHT;
pub(super) const STRIP_HEIGHT: f32 = qol_theme::HEIGHT_INLINE;
const GROW: f32 = qol_theme::toast::GROW;
pub(super) const WIDTH: f32 = CARD_WIDTH * GROW;
pub(super) const EDGE_STEP: f32 = qol_theme::SPACE_SNUG;
pub(super) const EDGE_INSET: f32 = qol_theme::SPACE_CELL;
pub(super) const MAX_EDGES: usize = 3;
const EDGE_BAND: f32 = qol_theme::toast::FAN;
pub(super) const WORDS_ROW: f32 = qol_theme::toast::SHOW_ALL_ROW;
pub(super) const WORDS_RISE: f32 = qol_theme::SPACE_TIGHT;
const LIST_GAP: f32 = qol_theme::SPACE_INSET;
const LEAVE_SCALE: f32 = qol_theme::toast::LEAVE_SCALE;
const EDGE_FLOOR: f32 = qol_theme::toast::EDGE_FLOOR;
pub(super) const GLIDE: Motion = Motion::QUICK;
pub(super) const LIST_STEP: f32 = CARD_HEIGHT + LIST_GAP;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Frame {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CardFrame {
    pub frame: Frame,
    pub scale: f32,
    pub opacity: f32,
    pub content: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Pile {
    pub width: f32,
    pub height: f32,
    pub cards: Vec<CardFrame>,
    pub strip: Option<CardFrame>,
    pub band: Option<Frame>,
    pub words: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Anchored {
    right: f32,
    bottom: f32,
    width: f32,
    height: f32,
}

impl Anchored {
    fn lerp(self, to: Self, t: f32) -> Self {
        Self {
            right: lerp(self.right, to.right, t),
            bottom: lerp(self.bottom, to.bottom, t),
            width: lerp(self.width, to.width, t),
            height: lerp(self.height, to.height, t),
        }
    }

    fn scaled_about_centre(self, scale: f32) -> Self {
        let height = self.height * scale;
        Self {
            right: self.right,
            bottom: self.bottom - (height - self.height) / 2.0,
            width: self.width * scale,
            height,
        }
    }

    fn top(self) -> f32 {
        self.bottom + self.height
    }

    fn reach(self) -> f32 {
        self.right + self.width
    }

    fn placed(self, width: f32, height: f32) -> Frame {
        Frame {
            left: width - self.right - self.width,
            top: height - self.bottom - self.height,
            width: self.width,
            height: self.height,
        }
    }
}

fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

pub(super) fn edge_count(count: usize) -> usize {
    count.saturating_sub(1).min(MAX_EDGES)
}

pub(super) fn layout(heights: &[f32], grow: f32, open: f32, focus: &[f32], scroll: f32) -> Pile {
    let count = heights.len();
    let newest_height = heights.first().copied().unwrap_or(CARD_HEIGHT);
    let focus_of = |index: usize| focus.get(index).copied().unwrap_or(0.0);
    let piled = count >= 2;
    let open = if piled { open } else { 0.0 };
    let newest_scale = 1.0 + (GROW - 1.0) * (grow * (1.0 - open)).max(focus_of(0) * open);
    let strip_height = if piled {
        STRIP_HEIGHT * newest_scale
    } else {
        0.0
    };
    let strip = piled.then_some(Anchored {
        right: 0.0,
        bottom: 0.0,
        width: CARD_WIDTH * newest_scale,
        height: strip_height,
    });
    let lowered = scroll * open;
    let newest = Anchored {
        right: 0.0,
        bottom: strip_height - lowered,
        width: CARD_WIDTH * newest_scale,
        height: newest_height * newest_scale,
    };
    let rest_strip = if piled { STRIP_HEIGHT } else { 0.0 };
    let grown_top = (rest_strip + newest_height) * GROW;
    let edges = edge_count(count);
    let peek = if edges == 0 {
        0.0
    } else {
        EDGE_BAND / edges as f32
    };

    let mut anchored = vec![(newest, 1.0, newest_scale, 1.0)];
    let mut listed_bottom = rest_strip + newest_height + LIST_GAP;
    for (index, &own_height) in heights.iter().enumerate().skip(1) {
        let edge = index.min(edges.max(1)) as f32;
        let resting = Anchored {
            right: EDGE_INSET * edge,
            bottom: rest_strip + EDGE_STEP * edge,
            width: CARD_WIDTH - 2.0 * EDGE_INSET * edge,
            height: newest_height,
        };
        let grown = Anchored {
            right: 0.0,
            bottom: grown_top + peek * edge - newest_height,
            width: CARD_WIDTH * GROW,
            height: newest_height,
        };
        let listed = Anchored {
            right: 0.0,
            bottom: listed_bottom - scroll,
            width: CARD_WIDTH,
            height: own_height,
        };
        listed_bottom += own_height + LIST_GAP;
        let scale = 1.0 + (GROW - 1.0) * focus_of(index) * open;
        let place = resting
            .lerp(grown, grow)
            .lerp(listed, open)
            .scaled_about_centre(scale);
        let opacity = if index <= edges { 1.0 } else { open };
        anchored.push((place, opacity, scale, open));
    }

    let raised = grow * (1.0 - open);
    let band = (piled && raised > 0.0).then(|| {
        let top_edge = anchored[edges.max(1)].0.top();
        Anchored {
            right: 0.0,
            bottom: newest.top(),
            width: CARD_WIDTH * GROW,
            height: (top_edge - newest.top()).max(0.0) + WORDS_ROW,
        }
    });

    let width = anchored
        .iter()
        .map(|(place, opacity, _, _)| if *opacity > 0.0 { place.reach() } else { 0.0 })
        .chain(strip.map(Anchored::reach))
        .chain(band.map(Anchored::reach))
        .fold(CARD_WIDTH, f32::max);
    let height = anchored
        .iter()
        .map(|(place, opacity, _, _)| if *opacity > 0.0 { place.top() } else { 0.0 })
        .chain(band.map(Anchored::top))
        .fold(0.0, f32::max);

    Pile {
        width,
        height,
        cards: anchored
            .into_iter()
            .map(|(place, opacity, scale, content)| CardFrame {
                frame: place.placed(width, height),
                scale,
                opacity,
                content,
            })
            .collect(),
        strip: strip.map(|place| CardFrame {
            frame: place.placed(width, height),
            scale: newest_scale,
            opacity: 1.0,
            content: 1.0,
        }),
        band: band.map(|place| place.placed(width, height)),
        words: raised,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Pose {
    right: f32,
    bottom: f32,
    width: f32,
    height: f32,
    scale: f32,
    opacity: f32,
    content: f32,
}

impl Pose {
    pub(super) fn of(card: CardFrame, pile_width: f32, pile_height: f32) -> Self {
        let frame = card.frame;
        Self {
            right: pile_width - frame.left - frame.width,
            bottom: pile_height - frame.top - frame.height,
            width: frame.width,
            height: frame.height,
            scale: card.scale,
            opacity: card.opacity,
            content: card.content,
        }
    }

    pub(super) fn toward(self, to: Self, t: f32) -> Self {
        Self {
            right: lerp(self.right, to.right, t),
            bottom: lerp(self.bottom, to.bottom, t),
            width: lerp(self.width, to.width, t),
            height: lerp(self.height, to.height, t),
            scale: lerp(self.scale, to.scale, t),
            opacity: lerp(self.opacity, to.opacity, t),
            content: lerp(self.content, to.content, t),
        }
    }

    pub(super) fn leaving(self, t: f32) -> Self {
        let shrink = lerp(1.0, LEAVE_SCALE, t);
        let width = self.width * shrink;
        let height = self.height * shrink;
        Self {
            right: self.right + (self.width - width) / 2.0,
            bottom: self.bottom + (self.height - height) / 2.0,
            width,
            height,
            scale: self.scale * shrink,
            opacity: self.opacity * (1.0 - t),
            content: self.content,
        }
    }

    pub(super) fn card(self, width: f32, height: f32) -> CardFrame {
        CardFrame {
            frame: Frame {
                left: width - self.right - self.width,
                top: height - self.bottom - self.height,
                width: self.width,
                height: self.height,
            },
            scale: self.scale,
            opacity: self.opacity,
            content: self.content,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Gravity {
    across: f32,
    down: f32,
}

impl Gravity {
    pub(super) fn new(across: f32, down: f32) -> Self {
        Self {
            across: across.clamp(0.0, 1.0),
            down: down.clamp(0.0, 1.0),
        }
    }

    pub(super) fn hangs(self) -> bool {
        self.down < 0.5
    }

    pub(super) fn base(self, height: f32) -> f32 {
        let travel = (height - CARD_HEIGHT).max(0.0);
        if self.hangs() {
            self.down * travel
        } else {
            height - (1.0 - self.down) * travel
        }
    }

    pub(super) fn room(self, height: f32) -> f32 {
        if self.hangs() {
            height - self.base(height)
        } else {
            self.base(height)
        }
    }

    pub(super) fn left(self, width: f32, pile_width: f32) -> f32 {
        (width - pile_width).max(0.0) * self.across
    }

    pub(super) fn pile_top(self, height: f32, pile_height: f32) -> f32 {
        if self.hangs() {
            self.base(height)
        } else {
            self.base(height) - pile_height
        }
    }

    pub(super) fn top(self, height: f32, pile_height: f32, frame: Frame) -> f32 {
        let inside = if self.hangs() {
            pile_height - frame.top - frame.height
        } else {
            frame.top
        };
        self.pile_top(height, pile_height) + inside
    }
}

pub(super) fn edge_fade(centre: f32, room: f32, above: f32, below: f32) -> f32 {
    let fade = |inside: f32, hidden: f32| {
        lerp(
            1.0,
            lerp(EDGE_FLOOR, 1.0, (inside / LIST_STEP).clamp(0.0, 1.0)),
            (hidden / LIST_STEP).clamp(0.0, 1.0),
        )
    };
    fade(centre, above).min(fade(room - centre, below))
}

pub(super) fn overflow(heights: &[f32], room: f32) -> f32 {
    (layout(heights, 0.0, 1.0, &[], 0.0).height + LIST_GAP - room).max(0.0)
}

pub(super) fn contains(rect: [f32; 4], x: f32, y: f32) -> bool {
    let [left, top, width, height] = rect;
    x >= left && x <= left + width && y >= top && y <= top + height
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    motion: Motion,
}

impl Tween {
    pub(super) fn at(value: f32) -> Self {
        Self {
            from: value,
            to: value,
            start: Instant::now(),
            motion: Motion::SETTLE,
        }
    }

    pub(super) fn gliding(self) -> Self {
        Self {
            motion: GLIDE,
            ..self
        }
    }

    pub(super) fn value(&self, now: Instant) -> f32 {
        let progress =
            crate::motion::progress(self.motion, now.saturating_duration_since(self.start));
        lerp(self.from, self.to, progress)
    }

    pub(super) fn target(&self) -> f32 {
        self.to
    }

    pub(super) fn moving(&self, now: Instant) -> bool {
        self.from != self.to && now.saturating_duration_since(self.start) < self.motion.duration
    }

    pub(super) fn toward(&mut self, to: f32, now: Instant) {
        if self.to == to {
            return;
        }
        self.from = self.value(now);
        self.to = to;
        self.start = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    fn even(count: usize) -> Vec<f32> {
        vec![CARD_HEIGHT; count]
    }

    #[test]
    fn an_open_list_stacks_cards_of_their_own_heights_one_gap_apart() {
        let heights = [CARD_HEIGHT, 120.0, 88.0, CARD_HEIGHT];
        let pile = layout(&heights, 1.0, 1.0, &[], 0.0);
        for (card, height) in pile.cards.iter().zip(heights) {
            assert!(close(card.frame.height, height));
        }
        for pair in pile.cards.windows(2) {
            let gap = pair[0].frame.top - (pair[1].frame.top + pair[1].frame.height);
            assert!(close(gap, LIST_GAP), "{gap}");
        }
        let resting = layout(&heights, 0.0, 0.0, &[], 0.0);
        for card in &resting.cards[1..] {
            assert!(
                close(card.frame.height, CARD_HEIGHT),
                "folded cards take the newest card's height"
            );
        }
    }

    #[test]
    fn the_bottom_right_gravity_rests_the_pile_on_the_window_floor() {
        let gravity = Gravity::new(1.0, 1.0);
        let pile = layout(&even(3), 0.0, 0.0, &[], 0.0);
        assert!(!gravity.hangs());
        assert!(close(gravity.base(1000.0), 1000.0));
        assert!(close(gravity.room(1000.0), 1000.0));
        assert!(close(gravity.left(WIDTH, pile.width), WIDTH - pile.width));
        let newest = pile.cards[0].frame;
        assert!(close(
            gravity.top(1000.0, pile.height, newest),
            1000.0 - pile.height + newest.top
        ));
    }

    #[test]
    fn a_hanging_pile_mirrors_its_cards_below_the_newest() {
        let gravity = Gravity::new(0.0, 0.0);
        let height = 1000.0;
        let pile = layout(&even(3), 0.0, 0.0, &[], 0.0);
        assert!(gravity.hangs());
        assert!(close(gravity.base(height), 0.0));
        assert!(close(gravity.room(height), height));
        assert!(close(gravity.left(WIDTH, pile.width), 0.0));
        let strip = pile.strip.unwrap().frame;
        let newest = pile.cards[0].frame;
        let older = pile.cards[1].frame;
        assert!(close(gravity.top(height, pile.height, strip), 0.0));
        assert!(close(
            gravity.top(height, pile.height, newest),
            strip.height
        ));
        assert!(
            gravity.top(height, pile.height, older) + older.height
                > gravity.top(height, pile.height, newest) + newest.height,
            "the older edge peeks out below the newest card"
        );
    }

    #[test]
    fn a_lone_toast_sits_at_its_fraction_of_the_window_either_way() {
        let height = 1032.0;
        let lone = layout(&even(1), 0.0, 0.0, &[], 0.0);
        for down in [0.0, 0.2, 0.49, 0.5, 0.75, 1.0] {
            let gravity = Gravity::new(0.5, down);
            let top = gravity.top(height, lone.height, lone.cards[0].frame);
            assert!(close(top, down * (height - CARD_HEIGHT)), "{down}: {top}");
            assert!(gravity.room(height) >= CARD_HEIGHT, "{down}");
        }
        let centred = Gravity::new(0.5, 0.5);
        assert!(close(
            centred.left(WIDTH, lone.width),
            (WIDTH - lone.width) / 2.0
        ));
    }

    #[test]
    fn a_point_outside_the_card_rect_is_not_contained() {
        let rect = [10.0, 100.0, 50.0, 40.0];
        assert!(contains(rect, 10.0, 100.0));
        assert!(contains(rect, 60.0, 140.0));
        assert!(!contains(rect, 9.0, 120.0));
        assert!(!contains(rect, 30.0, 99.0));
        assert!(!contains(rect, 61.0, 120.0));
        assert!(!contains(rect, 30.0, 141.0));
    }

    #[test]
    fn a_single_toast_is_one_card_with_no_strip_or_band() {
        let pile = layout(&even(1), 0.0, 1.0, &[1.0], 0.0);
        assert_eq!(pile.cards.len(), 1);
        assert!(pile.strip.is_none());
        assert!(pile.band.is_none());
        assert!(
            close(pile.cards[0].scale, 1.0),
            "one toast never opens a list"
        );
        assert_eq!((pile.width, pile.height), (CARD_WIDTH, CARD_HEIGHT));
        let grown = layout(&even(1), 1.0, 0.0, &[], 0.0);
        assert!(close(grown.cards[0].scale, GROW));
        assert!(grown.band.is_none());
    }

    #[test]
    fn a_resting_pile_folds_the_older_toasts_into_inset_edges() {
        let pile = layout(&even(5), 0.0, 0.0, &[], 0.0);
        let newest = pile.cards[0].frame;
        assert!(close(newest.width, CARD_WIDTH));
        assert!(close(pile.strip.unwrap().frame.height, STRIP_HEIGHT));
        for (index, card) in pile.cards.iter().enumerate().skip(1) {
            let edge = index.min(3) as f32;
            assert!(close(
                card.frame.width,
                CARD_WIDTH - 2.0 * EDGE_INSET * edge
            ));
            assert!(close(newest.top - card.frame.top, EDGE_STEP * edge));
            assert!(close(card.content, 0.0), "folded toasts hide their text");
        }
        assert!(close(pile.cards[4].opacity, 0.0), "only three edges show");
        assert!(close(
            pile.height,
            STRIP_HEIGHT + CARD_HEIGHT + 3.0 * EDGE_STEP
        ));
    }

    #[test]
    fn a_grown_pile_spreads_the_edges_to_a_fixed_band_at_full_width() {
        for count in 2..=6 {
            let pile = layout(&even(count), 1.0, 0.0, &[], 0.0);
            let newest = pile.cards[0].frame;
            assert!(close(newest.width, CARD_WIDTH * GROW));
            let edges = edge_count(count);
            let top_edge = pile.cards[edges].frame;
            assert!(
                close(newest.top - top_edge.top, EDGE_BAND),
                "count={count}: edges add up to the band"
            );
            for card in &pile.cards[1..=edges] {
                assert!(close(card.frame.width, CARD_WIDTH * GROW));
            }
            let band = pile.band.unwrap();
            assert!(close(band.top, top_edge.top - WORDS_ROW));
            assert!(close(band.top + band.height, newest.top));
            assert!(
                close(band.top, 0.0),
                "the words row is the top of the window"
            );
            assert!(close(pile.words, 1.0));
        }
    }

    #[test]
    fn an_open_list_keeps_every_toast_at_rest_until_one_is_hovered() {
        let pile = layout(&even(4), 1.0, 1.0, &[], 0.0);
        assert!(pile.band.is_none());
        for (index, card) in pile.cards.iter().enumerate() {
            assert!(close(card.scale, 1.0), "index={index} stays at rest");
            assert!(close(card.frame.width, CARD_WIDTH));
            assert!(close(card.opacity, 1.0));
        }
        let gaps: Vec<f32> = pile
            .cards
            .windows(2)
            .map(|pair| pair[0].frame.top - (pair[1].frame.top + pair[1].frame.height))
            .collect();
        assert!(gaps.iter().all(|gap| close(*gap, LIST_GAP)), "{gaps:?}");
    }

    #[test]
    fn only_the_hovered_toast_grows_in_the_open_list() {
        let rest = layout(&even(4), 1.0, 1.0, &[], 0.0);
        let pile = layout(&even(4), 1.0, 1.0, &[0.0, 0.0, 1.0, 0.0], 0.0);
        for (index, card) in pile.cards.iter().enumerate() {
            let expected = if index == 2 { GROW } else { 1.0 };
            assert!(close(card.scale, expected), "index={index}");
        }
        let grown = pile.cards[2].frame;
        let before = rest.cards[2].frame;
        let centre = |frame: Frame, height: f32| height - (frame.top + frame.height / 2.0);
        assert!(close(
            centre(grown, pile.height),
            centre(before, rest.height)
        ));
        let newest = layout(&even(4), 1.0, 1.0, &[1.0], 0.0);
        assert!(close(newest.cards[0].scale, GROW));
        assert!(close(
            newest.strip.unwrap().frame.height,
            STRIP_HEIGHT * GROW
        ));
    }

    #[test]
    fn the_window_bounds_every_visible_part() {
        for (grow, open) in [(0.0, 0.0), (0.5, 0.0), (1.0, 0.0), (1.0, 0.5), (0.0, 1.0)] {
            let pile = layout(&even(5), grow, open, &[0.0, 1.0], 0.0);
            for card in pile.cards.iter().filter(|card| card.opacity > 0.0) {
                let frame = card.frame;
                assert!(frame.left >= -0.01 && frame.top >= -0.01, "{grow} {open}");
                assert!(frame.left + frame.width <= pile.width + 0.01);
                assert!(frame.top + frame.height <= pile.height + 0.01);
            }
        }
    }

    #[test]
    fn a_pose_keeps_its_corner_distance_in_any_window() {
        let pile = layout(&even(3), 0.0, 0.0, &[], 0.0);
        let card = pile.cards[1];
        let pose = Pose::of(card, pile.width, pile.height);
        let wider = pose.card(pile.width + 40.0, pile.height + 30.0);
        assert!(close(wider.frame.left, card.frame.left + 40.0));
        assert!(close(wider.frame.top, card.frame.top + 30.0));
        assert_eq!(pose.card(pile.width, pile.height), card);
    }

    #[test]
    fn the_next_toast_glides_into_the_place_of_a_leaving_newest() {
        let before = layout(&even(3), 0.0, 0.0, &[], 0.0);
        let after = layout(&even(2), 0.0, 0.0, &[], 0.0);
        let from = Pose::of(before.cards[1], before.width, before.height);
        let to = Pose::of(after.cards[0], after.width, after.height);
        assert_eq!(from.toward(to, 0.0), from);
        assert_eq!(from.toward(to, 1.0), to);
        let middle = from.toward(to, 0.5).card(after.width, after.height);
        assert!(middle.content > 0.0 && middle.content < 1.0);
    }

    #[test]
    fn a_leaving_toast_fades_and_shrinks_about_its_centre() {
        let pile = layout(&even(2), 0.0, 0.0, &[], 0.0);
        let pose = Pose::of(pile.cards[0], pile.width, pile.height);
        assert_eq!(pose.leaving(0.0), pose);
        let gone = pose.leaving(1.0).card(pile.width, pile.height);
        let start = pile.cards[0].frame;
        assert!(close(gone.opacity, 0.0));
        assert!(close(gone.frame.width, start.width * LEAVE_SCALE));
        assert!(close(
            gone.frame.left + gone.frame.width / 2.0,
            start.left + start.width / 2.0
        ));
        assert!(close(
            gone.frame.top + gone.frame.height / 2.0,
            start.top + start.height / 2.0
        ));
    }

    #[test]
    fn a_long_list_scrolls_inside_the_room_and_folds_back_unscrolled() {
        let room = 400.0;
        let extra = overflow(&even(20), room);
        let open = layout(&even(20), 1.0, 1.0, &[], 0.0);
        assert!(close(extra, open.height + LIST_GAP - room));
        assert!(close(overflow(&even(2), room), 0.0));
        let scrolled = layout(&even(20), 1.0, 1.0, &[], extra);
        for (before, after) in open.cards.iter().zip(&scrolled.cards) {
            assert!(close(
                open.height - before.frame.top - extra,
                scrolled.height - after.frame.top
            ));
        }
        assert_eq!(
            layout(&even(20), 1.0, 0.0, &[], extra),
            layout(&even(20), 1.0, 0.0, &[], 0.0)
        );
    }

    #[test]
    fn cards_dissolve_into_an_edge_only_while_it_hides_more() {
        let room = 600.0;
        assert!(close(edge_fade(0.0, room, 0.0, 0.0), 1.0));
        assert!(close(edge_fade(0.0, room, LIST_STEP, 0.0), EDGE_FLOOR));
        let half = lerp(EDGE_FLOOR, 1.0, 0.5);
        assert!(close(
            edge_fade(LIST_STEP / 2.0, room, LIST_STEP, 0.0),
            half
        ));
        assert!(close(
            edge_fade(LIST_STEP / 2.0, room, LIST_STEP / 2.0, 0.0),
            lerp(1.0, half, 0.5)
        ));
        assert!(close(edge_fade(room, room, 0.0, LIST_STEP), EDGE_FLOOR));
        assert!(close(
            edge_fade(room / 2.0, room, LIST_STEP, LIST_STEP),
            1.0
        ));
    }

    #[test]
    fn a_tween_settles_on_its_target_and_restarts_from_where_it_is() {
        crate::motion::pin(true);
        let start = Instant::now();
        let mut tween = Tween::at(0.0);
        tween.toward(1.0, start);
        assert!(tween.moving(start));
        let middle = start + Motion::SETTLE.duration / 2;
        let halfway = tween.value(middle);
        assert!(halfway > 0.0 && halfway < 1.0);
        tween.toward(0.0, middle);
        assert!(close(tween.value(middle), halfway));
        let done = middle + Motion::SETTLE.duration;
        assert!(close(tween.value(done), 0.0));
        assert!(!tween.moving(done));
    }
}
