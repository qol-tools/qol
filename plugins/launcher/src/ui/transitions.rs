use std::collections::HashMap;
use std::time::Instant;

use qol_gpui::theme::Motion;

const STILL: f32 = 0.01;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Prop {
    Top,
    Strength,
    Lit,
    Shown,
    Height,
}

#[derive(Clone, Copy, Debug)]
struct Tween {
    from: f32,
    to: f32,
    started: Instant,
    motion: Motion,
}

impl Tween {
    fn at(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.started);
        if elapsed >= self.motion.duration {
            return self.to;
        }
        self.from + (self.to - self.from) * self.motion.progress(elapsed)
    }

    fn moving(&self, now: Instant) -> bool {
        (self.to - self.from).abs() > STILL
            && now.saturating_duration_since(self.started) < self.motion.duration
    }
}

#[derive(Default)]
pub struct Transitions {
    tweens: HashMap<(String, Prop), Tween>,
}

impl Transitions {
    pub fn knows(&self, key: &str, prop: Prop) -> bool {
        self.tweens.contains_key(&(key.to_owned(), prop))
    }

    pub fn current(&self, key: &str, prop: Prop, now: Instant) -> Option<f32> {
        self.tweens
            .get(&(key.to_owned(), prop))
            .map(|tween| tween.at(now))
    }

    pub fn value(
        &mut self,
        key: &str,
        prop: Prop,
        target: f32,
        motion: Motion,
        now: Instant,
    ) -> f32 {
        let tween = self.tweens.entry((key.to_owned(), prop)).or_insert(Tween {
            from: target,
            to: target,
            started: now,
            motion,
        });
        if (tween.to - target).abs() > STILL {
            *tween = Tween {
                from: tween.at(now),
                to: target,
                started: now,
                motion,
            };
        }
        tween.at(now)
    }

    pub fn enter(
        &mut self,
        key: &str,
        prop: Prop,
        (from, target): (f32, f32),
        motion: Motion,
        now: Instant,
    ) -> f32 {
        let tween = Tween {
            from,
            to: target,
            started: now,
            motion,
        };
        self.tweens.insert((key.to_owned(), prop), tween);
        tween.at(now)
    }

    pub fn moving(&self, now: Instant) -> bool {
        self.tweens.values().any(|tween| tween.moving(now))
    }

    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.tweens.retain(|(key, _), _| keep(key));
    }

    pub fn clear(&mut self) {
        self.tweens.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_new_value_settles_at_once_and_a_change_eases_from_where_it_is() {
        let start = Instant::now();
        let mut transitions = Transitions::default();
        assert_eq!(
            transitions.value("row", Prop::Top, 10.0, Motion::SETTLE, start),
            10.0
        );
        assert!(!transitions.moving(start));
        assert_eq!(
            transitions.value("row", Prop::Top, 110.0, Motion::SETTLE, start),
            10.0
        );
        assert!(transitions.moving(start));
        let half = start + Motion::SETTLE.duration / 2;
        let midway = transitions.value("row", Prop::Top, 110.0, Motion::SETTLE, half);
        assert!(midway > 60.0 && midway < 110.0, "{midway}");
        let turned = transitions.value("row", Prop::Top, 0.0, Motion::SETTLE, half);
        assert!((turned - midway).abs() < 1e-3);
        let end = half + Motion::SETTLE.duration + Duration::from_millis(1);
        assert_eq!(
            transitions.value("row", Prop::Top, 0.0, Motion::SETTLE, end),
            0.0
        );
        assert!(!transitions.moving(end));
    }

    #[test]
    fn entering_rows_start_where_they_are_told() {
        let start = Instant::now();
        let mut transitions = Transitions::default();
        assert_eq!(
            transitions.enter("row", Prop::Shown, (0.0, 1.0), Motion::QUICK, start),
            0.0
        );
        assert!(transitions.knows("row", Prop::Shown));
        transitions.retain(|key| key != "row");
        assert!(!transitions.knows("row", Prop::Shown));
    }
}
