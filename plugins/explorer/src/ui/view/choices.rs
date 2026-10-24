use std::time::SystemTime;

use gpui::{Context, Keystroke, ScrollStrategy};

use super::ExplorerView;
use crate::ops::{Clash, Resolve, Room, Stamp, Survey};
use crate::ui::browse::{copy_only_note, label_of, short_note};

pub(super) const CHOICES: [(Resolve, &str); 3] = [
    (Resolve::Keep, "Keep this"),
    (Resolve::Replace, "Replace"),
    (Resolve::KeepBoth, "Keep both"),
];
const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

pub(super) fn choice_index(choice: Resolve) -> usize {
    CHOICES
        .iter()
        .position(|(candidate, _)| *candidate == choice)
        .unwrap_or(0)
}

fn stepped(choice: Resolve, forward: bool) -> Resolve {
    let index = choice_index(choice);
    let next = if forward {
        (index + 1).min(CHOICES.len() - 1)
    } else {
        index.saturating_sub(1)
    };
    CHOICES[next].0
}

pub(super) fn newer_here(clash: &Clash) -> bool {
    matches!((clash.here.modified, clash.incoming.modified), (Some(here), Some(incoming)) if here > incoming)
}

pub(super) fn newer_incoming(clash: &Clash) -> bool {
    matches!((clash.here.modified, clash.incoming.modified), (Some(here), Some(incoming)) if incoming > here)
}

pub(super) fn when(modified: SystemTime, now: SystemTime) -> String {
    let age = now
        .duration_since(modified)
        .map_or(0, |elapsed| elapsed.as_secs());
    if age < MINUTE {
        return "just now".to_string();
    }
    if age < HOUR {
        return format!("{} min ago", age / MINUTE);
    }
    if age < DAY {
        return format!("{} h ago", age / HOUR);
    }
    if age < 2 * DAY {
        return "yesterday".to_string();
    }
    if age < 30 * DAY {
        return format!("{} days ago", age / DAY);
    }
    if age < 365 * DAY {
        let months = (age / (30 * DAY)) as usize;
        return qol_gpui::plural(months, "month ago", "months ago");
    }
    qol_gpui::plural((age / (365 * DAY)) as usize, "year ago", "years ago")
}

pub(super) fn stamp_label(stamp: &Stamp, newer: bool, now: SystemTime) -> String {
    let size = qol_gpui::format_bytes(stamp.bytes);
    let mut parts = vec![size];
    if let Some(modified) = stamp.modified {
        parts.push(when(modified, now));
    }
    if newer {
        parts.push("newer".to_string());
    }
    parts.join("  \u{b7}  ")
}

pub(super) fn say(view: &ExplorerView, survey: &Survey) -> Vec<String> {
    let mut lines: Vec<String> = [
        short_note(survey, &view.volume_label(&survey.transfer.into)),
        copy_only_note(survey),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !survey.clashes.is_empty() {
        let differ = survey.clashes.len();
        let single = survey.transfer.sources.len() == 1;
        let head = if single {
            format!("{} is already here.", label_of(&survey.transfer.sources))
        } else {
            "Some of these are already here.".to_string()
        };
        let same = if survey.same_inside > 0 {
            format!(
                " {} of {} {} files {} the same.",
                survey.same_inside,
                if single { "its" } else { "the" },
                survey.same_inside + differ,
                if survey.same_inside == 1 { "is" } else { "are" }
            )
        } else {
            String::new()
        };
        let tail = if differ == 1 {
            "This one differs:".to_string()
        } else {
            format!("These {differ} differ:")
        };
        lines.push(format!("{head}{same} {tail}"));
    }
    lines
}

impl ExplorerView {
    pub(super) fn select_clash(&mut self, row: usize, cx: &mut Context<Self>) {
        let count = self
            .browser
            .surveyed()
            .map_or(0, |survey| survey.clashes.len());
        if count == 0 {
            return;
        }
        self.clash_row = row.min(count - 1);
        self.clash_scroll
            .scroll_to_item(self.clash_row, ScrollStrategy::Top);
        cx.notify();
    }

    pub(super) fn choose_clash(&mut self, row: usize, choice: Resolve, cx: &mut Context<Self>) {
        self.clash_row = row;
        self.browser.choose(row, choice);
        cx.notify();
    }

    pub(super) fn apply_clash(&mut self, cx: &mut Context<Self>) {
        let blocked = self
            .browser
            .surveyed()
            .is_some_and(|survey| matches!(survey.room, Room::Short { .. }));
        if !blocked && self.browser.confirm() {
            self.clash_row = 0;
        }
        cx.notify();
    }

    pub(super) fn cancel_clash(&mut self, cx: &mut Context<Self>) {
        self.browser.cancel_survey();
        self.clash_row = 0;
        cx.notify();
    }

    pub(super) fn clash_key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let shift = keystroke.modifiers.shift;
        match keystroke.key.as_str() {
            "escape" => self.cancel_clash(cx),
            "enter" => self.apply_clash(cx),
            "up" => self.select_clash(self.clash_row.saturating_sub(1), cx),
            "down" => self.select_clash(self.clash_row + 1, cx),
            "left" | "right" => {
                let forward = keystroke.key == "right";
                let current = self
                    .browser
                    .surveyed()
                    .and_then(|survey| survey.clashes.get(self.clash_row))
                    .map(|clash| clash.choice);
                if let Some(current) = current {
                    let choice = stepped(current, forward);
                    if shift {
                        self.browser.choose_all(choice);
                    } else {
                        self.browser.choose(self.clash_row, choice);
                    }
                }
            }
            _ => {}
        }
        cx.stop_propagation();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::{stepped, when, CHOICES};
    use crate::ops::Resolve;

    #[test]
    fn choices_step_left_and_right_and_stop_at_the_ends() {
        assert_eq!(stepped(Resolve::Keep, false), Resolve::Keep);
        assert_eq!(stepped(Resolve::Keep, true), Resolve::Replace);
        assert_eq!(stepped(Resolve::Replace, true), Resolve::KeepBoth);
        assert_eq!(stepped(Resolve::KeepBoth, true), Resolve::KeepBoth);
        assert_eq!(stepped(Resolve::KeepBoth, false), Resolve::Replace);
        assert_eq!(CHOICES.len(), 3);
    }

    #[test]
    fn ages_read_as_plain_relative_words() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000_000);
        let cases = [
            (5, "just now"),
            (300, "5 min ago"),
            (7_200, "2 h ago"),
            (100_000, "yesterday"),
            (5 * 86_400, "5 days ago"),
        ];
        for (seconds, expected) in cases {
            let then = now - Duration::from_secs(seconds);
            assert_eq!(when(then, now), expected, "{seconds}");
        }
    }
}
