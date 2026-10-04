use std::time::{Duration, Instant};

use qol_theme::Motion;

pub(super) const CARD_WIDTH: f32 = 440.0;
pub(super) const CARD_HEIGHT: f32 = 84.0;
pub(super) const STRIP_HEIGHT: f32 = qol_theme::HEIGHT_INLINE;
pub(super) const GROW: f32 = 1.2;
const EDGE_STEP: f32 = qol_theme::SPACE_SNUG;
const EDGE_INSET: f32 = qol_theme::SPACE_CELL;
const MAX_EDGES: usize = 3;
const EDGE_BAND: f32 = 32.0;
pub(super) const WORDS_ROW: f32 = 24.0;
pub(super) const WORDS_RISE: f32 = qol_theme::SPACE_TIGHT;
const LIST_GAP: f32 = qol_theme::SPACE_INSET;

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

pub(super) fn layout(count: usize, grow: f32, open: f32, focus: &[f32]) -> Pile {
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
    let newest = Anchored {
        right: 0.0,
        bottom: strip_height,
        width: CARD_WIDTH * newest_scale,
        height: CARD_HEIGHT * newest_scale,
    };
    let rest_strip = if piled { STRIP_HEIGHT } else { 0.0 };
    let grown_top = (rest_strip + CARD_HEIGHT) * GROW;
    let edges = edge_count(count);
    let peek = if edges == 0 {
        0.0
    } else {
        EDGE_BAND / edges as f32
    };

    let mut anchored = vec![(newest, 1.0, newest_scale, 1.0)];
    for index in 1..count {
        let edge = index.min(edges.max(1)) as f32;
        let resting = Anchored {
            right: EDGE_INSET * edge,
            bottom: rest_strip + EDGE_STEP * edge,
            width: CARD_WIDTH - 2.0 * EDGE_INSET * edge,
            height: CARD_HEIGHT,
        };
        let grown = Anchored {
            right: 0.0,
            bottom: grown_top + peek * edge - CARD_HEIGHT,
            width: CARD_WIDTH * GROW,
            height: CARD_HEIGHT,
        };
        let listed = Anchored {
            right: 0.0,
            bottom: rest_strip + (CARD_HEIGHT + LIST_GAP) * index as f32,
            width: CARD_WIDTH,
            height: CARD_HEIGHT,
        };
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

pub(super) fn footprint(count: usize) -> (f32, f32) {
    let mut states = vec![layout(count, 0.0, 0.0, &[]), layout(count, 1.0, 0.0, &[])];
    for index in 0..count {
        let mut focus = vec![0.0; count];
        focus[index] = 1.0;
        states.push(layout(count, 0.0, 1.0, &focus));
    }
    states.iter().fold((0.0, 0.0), |(width, height), pile| {
        (width.max(pile.width), height.max(pile.height))
    })
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Tween {
    from: f32,
    to: f32,
    start: Instant,
}

impl Tween {
    pub(super) fn at(value: f32) -> Self {
        Self {
            from: value,
            to: value,
            start: Instant::now(),
        }
    }

    pub(super) fn value(&self, now: Instant) -> f32 {
        let progress = Motion::SETTLE.progress(now.saturating_duration_since(self.start));
        lerp(self.from, self.to, progress)
    }

    pub(super) fn target(&self) -> f32 {
        self.to
    }

    pub(super) fn moving(&self, now: Instant) -> bool {
        self.from != self.to && now.saturating_duration_since(self.start) < Motion::SETTLE.duration
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

pub(super) fn age_label(age: Duration) -> String {
    let minutes = age.as_secs() / 60;
    match minutes {
        0 => "now".to_string(),
        1..=59 => format!("{minutes} min"),
        _ => format!("{} h", minutes / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn a_single_toast_is_one_card_with_no_strip_or_band() {
        let pile = layout(1, 0.0, 1.0, &[1.0]);
        assert_eq!(pile.cards.len(), 1);
        assert!(pile.strip.is_none());
        assert!(pile.band.is_none());
        assert!(
            close(pile.cards[0].scale, 1.0),
            "one toast never opens a list"
        );
        assert_eq!((pile.width, pile.height), (CARD_WIDTH, CARD_HEIGHT));
        let grown = layout(1, 1.0, 0.0, &[]);
        assert!(close(grown.cards[0].scale, GROW));
        assert!(grown.band.is_none());
    }

    #[test]
    fn a_resting_pile_folds_the_older_toasts_into_inset_edges() {
        let pile = layout(5, 0.0, 0.0, &[]);
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
            let pile = layout(count, 1.0, 0.0, &[]);
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
        let pile = layout(4, 1.0, 1.0, &[]);
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
        let rest = layout(4, 1.0, 1.0, &[]);
        let pile = layout(4, 1.0, 1.0, &[0.0, 0.0, 1.0, 0.0]);
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
        let newest = layout(4, 1.0, 1.0, &[1.0]);
        assert!(close(newest.cards[0].scale, GROW));
        assert!(close(
            newest.strip.unwrap().frame.height,
            STRIP_HEIGHT * GROW
        ));
    }

    #[test]
    fn the_window_bounds_every_visible_part() {
        for (grow, open) in [(0.0, 0.0), (0.5, 0.0), (1.0, 0.0), (1.0, 0.5), (0.0, 1.0)] {
            let pile = layout(5, grow, open, &[0.0, 1.0]);
            for card in pile.cards.iter().filter(|card| card.opacity > 0.0) {
                let frame = card.frame;
                assert!(frame.left >= -0.01 && frame.top >= -0.01, "{grow} {open}");
                assert!(frame.left + frame.width <= pile.width + 0.01);
                assert!(frame.top + frame.height <= pile.height + 0.01);
            }
        }
    }

    #[test]
    fn ages_read_now_then_minutes_then_hours() {
        let cases = [
            (0, "now"),
            (59, "now"),
            (60, "1 min"),
            (3599, "59 min"),
            (7200, "2 h"),
        ];
        for (seconds, expected) in cases {
            assert_eq!(age_label(Duration::from_secs(seconds)), expected);
        }
    }

    #[test]
    fn a_tween_settles_on_its_target_and_restarts_from_where_it_is() {
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
