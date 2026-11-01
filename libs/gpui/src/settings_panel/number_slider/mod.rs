pub(in crate::settings_panel) mod edit;
pub(in crate::settings_panel) mod numbers;
mod track;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::hash::Hash;
use std::time::{Duration, Instant};

use gpui::{AsyncApp, Context, Div, SharedString, WeakEntity};

use super::components::{number_field, NumberText, RowGround, SliderStyle};
use crate::kit::Kit;
use numbers::{
    coarse_drag, format_number, next_settle, number_cue, number_preview, number_release,
    pressed_slider, slider_fraction, slider_value_from_fraction, widest_number, NumberMotion,
    NumberSettle, SliderPress,
};

pub use edit::{number_key, NumberEdit, NumberKey};
pub use numbers::{NumberRelease, NumberSpec};
pub use track::{compact_track, slider_track};

pub const KEY_GLOW: Duration = Duration::from_millis(260);

pub struct NumberSlider<K> {
    hover: Option<K>,
    press: Option<SliderPress>,
    scrub: Option<f32>,
    drag_start: Option<(K, f64)>,
    key_glow: Option<(K, usize)>,
    key_glow_generation: usize,
    motion: RefCell<NumberMotion<K>>,
}

impl<K> Default for NumberSlider<K> {
    fn default() -> Self {
        Self {
            hover: None,
            press: None,
            scrub: None,
            drag_start: None,
            key_glow: None,
            key_glow_generation: 0,
            motion: RefCell::new(NumberMotion::default()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scrub {
    Still,
    Began,
    Moving,
}

pub struct SliderField<'a, K> {
    pub key: K,
    pub number: NumberSpec,
    pub edit: Option<&'a NumberEdit>,
    pub dragging: bool,
    pub unit: Option<SharedString>,
}

pub(in crate::settings_panel) struct FieldLook {
    pub(in crate::settings_panel) style: Option<SliderStyle>,
    pub(in crate::settings_panel) level: f32,
    pub(in crate::settings_panel) display: String,
}

impl<K: Clone + Eq + Hash + 'static> NumberSlider<K> {
    pub fn pointer_over(&mut self, key: K, over: bool) {
        if over {
            self.hover = Some(key);
        } else if self.hover.as_ref() == Some(&key) {
            self.hover = None;
        }
    }

    pub fn press(&mut self, fraction: f32) {
        self.press = Some(SliderPress::Held(fraction));
    }

    pub fn scrub(&mut self, fraction: f32) -> Scrub {
        let Some(press) = self.press else {
            return Scrub::Still;
        };
        let next = pressed_slider(press, fraction, SliderStyle::Compact.track_width());
        self.press = Some(next);
        match (press, next) {
            (_, SliderPress::Held(_)) => Scrub::Still,
            (SliderPress::Held(_), SliderPress::Scrubbing) => Scrub::Began,
            (SliderPress::Scrubbing, SliderPress::Scrubbing) => Scrub::Moving,
        }
    }

    pub fn begin_drag(&mut self, start: Option<(K, f64)>) {
        self.drag_start = start;
    }

    pub fn drag_start(&self) -> Option<&(K, f64)> {
        self.drag_start.as_ref()
    }

    pub fn take_drag_start(&mut self) -> Option<(K, f64)> {
        self.drag_start.take()
    }

    pub fn drag_value(&self, key: &K, fraction: f32, coarse: bool, bounds: (f64, f64, f64)) -> f64 {
        let (min, max, step) = bounds;
        let dragged = slider_value_from_fraction(min, max, step, fraction);
        match &self.drag_start {
            Some((start_key, start)) if coarse && start_key == key => {
                coarse_drag(*start, dragged, true, min, max, step)
            }
            _ => dragged,
        }
    }

    pub fn follow(&mut self, fraction: Option<f32>) {
        self.scrub = fraction;
    }

    pub fn drag(
        &mut self,
        key: K,
        fraction: f32,
        coarse: bool,
        number: &NumberSpec,
    ) -> Option<(Scrub, f64)> {
        let (Some(min), Some(max)) = (number.min, number.max) else {
            return None;
        };
        let scrub = self.scrub(fraction);
        match scrub {
            Scrub::Still => return None,
            Scrub::Began => self.begin_drag(Some((key.clone(), number.value))),
            Scrub::Moving => {}
        }
        let value = self.drag_value(
            &key,
            fraction,
            coarse,
            (min, max, number.step.unwrap_or(1.0)),
        );
        self.follow(Some(slider_fraction(value, number.min, number.max)));
        Some((scrub, value))
    }

    pub fn release(&mut self, row: usize, selected: usize, editing: bool) -> NumberRelease {
        self.scrub = None;
        number_release(self.press.take(), row, selected, editing)
    }

    pub fn glow<V: 'static>(
        &mut self,
        key: K,
        slider: fn(&mut V) -> &mut Self,
        cx: &mut Context<V>,
    ) {
        self.key_glow_generation = self.key_glow_generation.wrapping_add(1);
        let generation = self.key_glow_generation;
        self.key_glow = Some((key, generation));
        cx.spawn(move |this: WeakEntity<V>, cx: &mut AsyncApp| {
            let mut async_cx = cx.clone();
            async move {
                async_cx.background_executor().timer(KEY_GLOW).await;
                let _ = this.update(&mut async_cx, |view, cx| {
                    if slider(view).end_glow(generation) {
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    pub fn end_glow(&mut self, generation: usize) -> bool {
        let current = self
            .key_glow
            .as_ref()
            .is_some_and(|(_, glowing)| *glowing == generation);
        if current {
            self.key_glow = None;
        }
        current
    }

    pub fn glowing(&self, key: &K) -> bool {
        self.key_glow
            .as_ref()
            .is_some_and(|(glowing, _)| glowing == key)
    }

    pub fn clear(&mut self) {
        self.press = None;
        self.scrub = None;
        self.hover = None;
        let motion = self.motion.get_mut();
        motion.settles.clear();
        motion.blooms.clear();
        self.key_glow = None;
    }

    pub fn compact(
        &self,
        field: SliderField<'_, K>,
        interact: impl FnOnce(Div) -> Div,
        row: RowGround,
        kit: Kit,
    ) -> Div {
        let display = format_number(field.number.value);
        self.draw(
            field,
            FieldLook {
                style: Some(SliderStyle::Compact),
                level: 0.0,
                display,
            },
            interact,
            row,
            kit,
        )
    }

    pub(in crate::settings_panel) fn draw(
        &self,
        field: SliderField<'_, K>,
        look: FieldLook,
        interact: impl FnOnce(Div) -> Div,
        row: RowGround,
        kit: Kit,
    ) -> Div {
        let SliderField {
            key,
            number,
            edit,
            dragging,
            unit,
        } = field;
        let FieldLook {
            style,
            level,
            display,
        } = look;
        let text = edit.map(NumberEdit::text);
        let pointer = self.scrub.filter(|_| dragging);
        let preview = number_preview(text, number.value, number.min, number.max, number.step);
        let track = style.map(|style| {
            let fraction =
                pointer.unwrap_or_else(|| slider_fraction(preview, number.min, number.max));
            (fraction, style)
        });
        let mut cue = number_cue(
            style,
            edit.map(|edit| (edit.text(), edit.capped())),
            dragging,
            self.press,
            level,
            number.min,
            number.max,
        );
        if let Some((fraction, SliderStyle::Compact)) = track {
            cue.marked = self.hover.as_ref() == Some(&key) || cue.dragging || cue.editing;
            cue.glowing = cue.dragging || (cue.editing && self.glowing(&key));
            cue.bloom = self.bloom(key.clone(), cue.glowing);
            cue.settle = self.settle(key, fraction, pointer.is_some());
        }
        let shown = match (style, text) {
            (Some(_), Some(text)) => text.to_string(),
            _ => display,
        };
        number_field(
            NumberText {
                widest: widest_number(&number, &shown),
                shown,
                selected: edit.is_some_and(NumberEdit::selects_all),
            },
            unit,
            track,
            cue,
            interact,
            row,
            kit,
        )
    }

    fn settle(&self, key: K, to: f32, follow: bool) -> Option<(f32, usize)> {
        let now = Instant::now();
        let mut motion = self.motion.borrow_mut();
        let step = motion.step + 1;
        let current = motion.settles.get(&key).copied();
        if let Some(next) = next_settle(current, to, follow, step, now) {
            motion.step = step;
            motion.settles.insert(key.clone(), next);
        }
        motion
            .settles
            .get(&key)
            .and_then(|settle| settle.in_flight(now))
    }

    fn bloom(&self, key: K, active: bool) -> Option<(f32, usize)> {
        let now = Instant::now();
        let mut motion = self.motion.borrow_mut();
        let step = motion.step + 1;
        let current = motion.blooms.get(&key).copied().or(Some(NumberSettle {
            from: 0.0,
            to: 0.0,
            step: 0,
            started: now,
        }));
        if let Some(next) = next_settle(current, if active { 1.0 } else { 0.0 }, false, step, now) {
            motion.step = step;
            motion.blooms.insert(key.clone(), next);
        }
        motion
            .blooms
            .get(&key)
            .and_then(|bloom| bloom.in_flight(now))
    }
}
