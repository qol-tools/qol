use std::time::Instant;

use gpui::*;
use qol_theme::Motion;

use super::super::components::{settings_row_shade, SettingsRow};
use super::structured_list_editor::{level_entries_len, sync_entries_level};
use super::{Level, SettingsPanelView};

pub(super) const ENTRY_PITCH: f32 = qol_theme::HEIGHT_RULE_ROW + qol_theme::SPACE_TIGHT;
const LIFT_RISE: f32 = qol_theme::SPACE_INSET;
const LIFT_LEFT: f32 = qol_theme::SPACE_CELL;
const LIFT_RIGHT: f32 = qol_theme::SPACE_INSET;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Held {
    Enter,
    Ctrl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Carry {
    pub(super) home: usize,
    pub(super) held: Held,
}

#[derive(Clone, Copy, Debug)]
struct Ease {
    from: f32,
    to: f32,
    at: Instant,
    motion: Motion,
}

impl Ease {
    fn value(&self, now: Instant) -> f32 {
        self.from + (self.to - self.from) * self.progress(now)
    }

    fn done(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }

    fn progress(&self, now: Instant) -> f32 {
        crate::motion::progress(self.motion, now.saturating_duration_since(self.at))
    }
}

#[derive(Clone, Copy, Debug)]
struct Slide {
    row: usize,
    ease: Ease,
}

#[derive(Debug, Default)]
pub(super) struct CarryMotion {
    row: usize,
    lift: Option<Ease>,
    slides: Vec<Slide>,
}

impl CarryMotion {
    pub(super) fn lifted(&self, now: Instant) -> Option<(usize, f32)> {
        let lift = self.lift.map_or(0.0, |ease| ease.value(now));
        (lift > 0.0).then_some((self.row, lift))
    }

    pub(super) fn offset(&self, row: usize, now: Instant) -> f32 {
        self.slides
            .iter()
            .filter(|slide| slide.row == row)
            .map(|slide| slide.ease.value(now))
            .sum()
    }

    fn moving(&self, now: Instant) -> bool {
        self.lift.is_some_and(|ease| !ease.done(now))
            || self.slides.iter().any(|slide| !slide.ease.done(now))
    }

    fn settle(&mut self, now: Instant) {
        self.slides.retain(|slide| !slide.ease.done(now));
        if self
            .lift
            .is_some_and(|ease| ease.to == 0.0 && ease.done(now))
        {
            self.lift = None;
        }
    }

    fn lift_row(&mut self, row: usize, now: Instant) {
        let from = match self.row == row {
            true => self.lifted(now).map_or(0.0, |(_, lift)| lift),
            false => 0.0,
        };
        self.row = row;
        self.lift = Some(Ease {
            from,
            to: 1.0,
            at: now,
            motion: Motion::QUICK,
        });
    }

    fn drop_lift(&mut self, motion: Motion, now: Instant) {
        let from = self.lifted(now).map_or(0.0, |(_, lift)| lift);
        self.lift = Some(Ease {
            from,
            to: 0.0,
            at: now,
            motion,
        });
    }

    fn reorder(&mut self, from: usize, to: usize, motion: Motion, now: Instant) {
        let moved = moved_rows(from, to);
        let shown: Vec<(usize, f32)> = moved
            .iter()
            .map(|&(old, new)| (new, old as f32 + self.offset(old, now) - new as f32))
            .collect();
        self.slides
            .retain(|slide| !moved.iter().any(|&(old, _)| old == slide.row));
        self.slides
            .extend(
                shown
                    .into_iter()
                    .filter(|(_, offset)| *offset != 0.0)
                    .map(|(row, offset)| Slide {
                        row,
                        ease: Ease {
                            from: offset,
                            to: 0.0,
                            at: now,
                            motion,
                        },
                    }),
            );
        if self.row == from {
            self.row = to;
        }
    }
}

fn moved_rows(from: usize, to: usize) -> Vec<(usize, usize)> {
    let mut moved = vec![(from, to)];
    if from < to {
        moved.extend((from + 1..=to).map(|row| (row, row - 1)));
    } else {
        moved.extend((to..from).map(|row| (row, row + 1)));
    }
    moved
}

fn move_entry(level: &mut Level, from: usize, to: usize) -> bool {
    let (Some(from), Some(to)) = (from.checked_sub(1), to.checked_sub(1)) else {
        return false;
    };
    if let Some(values) = level.entries.as_mut().and_then(|card| card.values.as_mut()) {
        if from >= values.len() || to >= values.len() {
            return false;
        }
        let value = values.remove(from);
        values.insert(to, value);
        return true;
    }
    let Some(state) = level.object_array.as_mut() else {
        return false;
    };
    if from >= state.entries.len() || to >= state.entries.len() {
        return false;
    }
    let entry = state.entries.remove(from);
    state.entries.insert(to, entry);
    true
}

impl SettingsPanelView {
    pub(super) fn carrying(&self) -> Option<Carry> {
        self.level().entries.as_ref().and_then(|card| card.carry)
    }

    pub(super) fn on_carry_key(&mut self, event: &KeyDownEvent) -> bool {
        let key = event.keystroke.key.as_str();
        if self.carrying().is_some() {
            match key {
                "up" => self.carry_step(-1),
                "down" => self.carry_step(1),
                "enter" | "return" | "space" => self.put_down(),
                "escape" => self.put_back(),
                _ => {}
            }
            return true;
        }
        let reorders = self
            .level()
            .entries
            .as_ref()
            .is_some_and(|card| card.reorder);
        if !reorders || self.level().selected == 0 {
            return false;
        }
        let modifiers = &event.keystroke.modifiers;
        let ctrl_only =
            modifiers.control && !modifiers.alt && !modifiers.shift && !modifiers.platform;
        match key {
            "up" | "down" if ctrl_only => {
                self.pick_up(Held::Ctrl);
                self.carry_step(if key == "up" { -1 } else { 1 });
                true
            }
            "enter" | "return" | "space" => {
                self.pick_up(Held::Enter);
                true
            }
            _ => false,
        }
    }

    pub(super) fn on_carry_modifiers(
        &mut self,
        event: &ModifiersChangedEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers.control {
            return;
        }
        if self
            .carrying()
            .is_some_and(|carry| carry.held == Held::Ctrl)
        {
            self.put_down();
            cx.notify();
        }
    }

    pub(super) fn pick_up(&mut self, held: Held) {
        let now = Instant::now();
        let level = self.level_mut();
        let row = level.selected;
        let Some(card) = level.entries.as_mut() else {
            return;
        };
        card.carry = Some(Carry { home: row, held });
        card.motion.lift_row(row, now);
    }

    pub(super) fn put_down(&mut self) {
        let index = self.stack.len() - 1;
        let level = &mut self.stack[index];
        let moved = level.selected;
        let Some(card) = level.entries.as_mut() else {
            return;
        };
        let Some(carry) = card.carry.take() else {
            return;
        };
        card.motion.drop_lift(Motion::QUICK, Instant::now());
        if moved != carry.home {
            self.write_entries_back(index);
        }
    }

    fn put_back(&mut self) {
        let index = self.stack.len() - 1;
        let from = self.stack[index].selected;
        let Some(carry) = self.stack[index]
            .entries
            .as_mut()
            .and_then(|card| card.carry.take())
        else {
            return;
        };
        if from != carry.home {
            self.move_carried(from, carry.home, Motion::TRAVEL);
        }
        if let Some(card) = self.stack[index].entries.as_mut() {
            card.motion.drop_lift(Motion::TRAVEL, Instant::now());
        }
    }

    fn carry_step(&mut self, step: isize) {
        let from = self.level().selected;
        let count = level_entries_len(self.level());
        let Some(to) = from
            .checked_add_signed(step)
            .filter(|row| (1..=count).contains(row))
        else {
            return;
        };
        self.move_carried(from, to, Motion::SETTLE);
    }

    fn move_carried(&mut self, from: usize, to: usize, motion: Motion) {
        let index = self.stack.len() - 1;
        let level = &mut self.stack[index];
        if !move_entry(level, from, to) {
            return;
        }
        sync_entries_level(level);
        level.selected = to;
        if let Some(card) = level.entries.as_mut() {
            card.motion.reorder(from, to, motion, Instant::now());
        }
        self.sync_scroll();
    }

    pub(super) fn entries_lift(&self, now: Instant) -> Option<(usize, f32)> {
        self.level()
            .entries
            .as_ref()
            .and_then(|card| card.motion.lifted(now))
    }

    pub(super) fn entries_offset(&self, row: usize, now: Instant) -> f32 {
        self.level()
            .entries
            .as_ref()
            .map_or(0.0, |card| card.motion.offset(row, now) * ENTRY_PITCH)
    }

    pub(super) fn entries_moving(&mut self) -> bool {
        let now = Instant::now();
        let Some(card) = self
            .stack
            .last_mut()
            .and_then(|level| level.entries.as_mut())
        else {
            return false;
        };
        card.motion.settle(now);
        card.motion.moving(now)
    }

    pub(super) fn carried_slot(&self, index: usize, lift: f32) -> SettingsRow {
        SettingsRow::rule(("settings-entry", index), self.kit)
            .child(self.row_bounds_canvas(index))
            .child(settings_row_shade(lift, self.kit))
    }

    pub(super) fn carried_row(
        &self,
        carried: usize,
        lift: f32,
        host: usize,
        children: Vec<AnyElement>,
        now: Instant,
    ) -> Div {
        let selected = carried == self.level().selected;
        let top = (carried as f32 - host as f32) * ENTRY_PITCH + self.entries_offset(carried, now)
            - self.entries_offset(host, now)
            - LIFT_RISE * lift;
        div()
            .absolute()
            .top(px(top))
            .left(px(LIFT_LEFT * lift))
            .right(px(-LIFT_RIGHT * lift))
            .flex()
            .flex_col()
            .child(
                SettingsRow::rule(("settings-entry-carried", carried), self.kit)
                    .selected(selected, self.body_has_focus())
                    .lifted(lift)
                    .children(children),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use qol_theme::Motion;

    use super::{moved_rows, CarryMotion};

    #[test]
    fn a_move_names_every_row_that_changes_place() {
        assert_eq!(moved_rows(3, 4), [(3, 4), (4, 3)]);
        assert_eq!(moved_rows(4, 3), [(4, 3), (3, 4)]);
        assert_eq!(moved_rows(6, 2), [(6, 2), (2, 3), (3, 4), (4, 5), (5, 6)]);
        assert_eq!(moved_rows(2, 5), [(2, 5), (3, 2), (4, 3), (5, 4)]);
    }

    #[test]
    fn rows_slide_from_where_they_were_drawn_into_their_new_place() {
        crate::motion::pin(true);
        let start = Instant::now();
        let mut motion = CarryMotion::default();
        motion.lift_row(3, start);
        motion.reorder(3, 4, Motion::SETTLE, start);
        assert_eq!(motion.lifted(start), None);
        assert_eq!(motion.offset(4, start), -1.0);
        assert_eq!(motion.offset(3, start), 1.0);
        let later = start + Motion::SETTLE.duration;
        assert_eq!(motion.offset(4, later), 0.0);
        assert_eq!(motion.lifted(later), Some((4, 1.0)));

        let mid = start + Motion::SETTLE.duration / 2;
        let halfway = motion.offset(4, mid);
        motion.reorder(4, 5, Motion::SETTLE, mid);
        assert!((motion.offset(5, mid) - (halfway - 1.0)).abs() < 1e-6);
        assert_eq!(motion.offset(4, mid), 1.0);
        assert!(motion.moving(mid));
        let after = mid + Duration::from_secs(1);
        motion.settle(after);
        assert!(!motion.moving(after));
    }

    #[test]
    fn a_dropped_row_lands_and_stops_drawing_lifted() {
        crate::motion::pin(true);
        let start = Instant::now();
        let mut motion = CarryMotion::default();
        motion.lift_row(2, start);
        let lifted = start + Motion::QUICK.duration;
        assert_eq!(motion.lifted(lifted), Some((2, 1.0)));
        motion.drop_lift(Motion::QUICK, lifted);
        let landed = lifted + Motion::QUICK.duration;
        assert_eq!(motion.lifted(landed), None);
        motion.settle(landed);
        assert!(!motion.moving(landed));
    }
}
