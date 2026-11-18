use std::time::SystemTime;

use gpui::{Context, Keystroke, ScrollStrategy};

use super::ExplorerView;
use crate::ops::{Clash, Resolve, Room, Stamp, Survey};
use crate::ui::browse::{copy_only_note, label_of, short_note};

const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

pub(super) fn choices(clash: &Clash, undo: bool) -> Vec<(Resolve, &'static str)> {
    if undo {
        return vec![
            (Resolve::Keep, "Keep this"),
            (Resolve::KeepBoth, "Keep both"),
        ];
    }
    let middle = if clash.merges() {
        (Resolve::Merge, "Merge")
    } else {
        (Resolve::Replace, "Replace")
    };
    vec![
        (Resolve::Keep, "Keep this"),
        middle,
        (Resolve::KeepBoth, "Keep both"),
    ]
}

fn stepped(clash: &Clash, forward: bool, undo: bool) -> Resolve {
    let choices = choices(clash, undo);
    let index = choices
        .iter()
        .position(|(choice, _)| *choice == clash.choice)
        .unwrap_or(0);
    let next = if forward {
        (index + 1).min(choices.len() - 1)
    } else {
        index.saturating_sub(1)
    };
    choices[next].0
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
    lines.extend(clash_sentence(survey));
    lines
}

fn clash_sentence(survey: &Survey) -> Option<String> {
    if survey.clashes.is_empty() {
        return None;
    }
    let differ = survey
        .clashes
        .iter()
        .filter(|clash| !clash.merges())
        .count();
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
    let tail = match differ {
        0 => String::new(),
        1 => " This one differs:".to_string(),
        _ => format!(" These {differ} differ:"),
    };
    Some(format!("{head}{same}{tail}"))
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
                let undo = self.browser.undo_asking();
                let next = self.browser.surveyed().and_then(|survey| {
                    let clash = survey.clashes.get(self.clash_row)?;
                    clash
                        .applies(&survey.clashes)
                        .then(|| stepped(clash, forward, undo))
                });
                if let Some(choice) = next {
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
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    use super::{choices, clash_sentence, stepped, when};
    use crate::ops::Resolve::{Keep, KeepBoth, Merge, Replace};
    use crate::ops::{Clash, Resolve, Room, Stamp, Survey, Transfer, Verb};

    fn clash(name: &str, folder: bool, choice: Resolve) -> Clash {
        let stamp = Stamp {
            bytes: 1,
            modified: None,
            folder,
        };
        Clash {
            source: PathBuf::from("/from").join(name),
            existing: PathBuf::from("/into").join(name),
            here: stamp,
            incoming: stamp,
            same: false,
            choice,
        }
    }

    fn sheet(sources: &[&str], clashes: Vec<Clash>, same_inside: usize) -> Survey {
        Survey {
            transfer: Transfer {
                verb: Verb::Move,
                sources: sources
                    .iter()
                    .map(|name| PathBuf::from("/from").join(name))
                    .collect(),
                into: PathBuf::from("/into"),
            },
            clashes,
            same_inside,
            bytes: 0,
            room: Room::Enough,
            copy_only: Vec::new(),
            crosses_volume: false,
        }
    }

    #[test]
    fn choices_step_left_and_right_and_stop_at_the_ends() {
        let cases = [
            (false, false, Keep, false, Keep),
            (false, false, Keep, true, Replace),
            (false, false, Replace, true, KeepBoth),
            (false, false, KeepBoth, true, KeepBoth),
            (false, false, KeepBoth, false, Replace),
            (true, false, Keep, false, Keep),
            (true, false, Keep, true, Merge),
            (true, false, Merge, true, KeepBoth),
            (true, false, KeepBoth, true, KeepBoth),
            (true, false, KeepBoth, false, Merge),
            (false, true, Keep, false, Keep),
            (false, true, Keep, true, KeepBoth),
            (false, true, KeepBoth, true, KeepBoth),
            (false, true, KeepBoth, false, Keep),
            (true, true, Keep, true, KeepBoth),
            (true, true, KeepBoth, false, Keep),
        ];
        for (folder, undo, from, forward, expected) in cases {
            let row = clash("docs", folder, from);
            assert_eq!(
                stepped(&row, forward, undo),
                expected,
                "{folder} {undo} {from:?} {forward}"
            );
        }
    }

    #[test]
    fn a_folder_onto_a_folder_offers_merge_where_a_file_offers_replace() {
        let labels = |folder, undo| {
            choices(&clash("docs", folder, Keep), undo)
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>()
        };
        assert_eq!(labels(false, false), ["Keep this", "Replace", "Keep both"]);
        assert_eq!(labels(true, false), ["Keep this", "Merge", "Keep both"]);
    }

    #[test]
    fn the_undo_sheet_offers_only_keep_this_and_keep_both() {
        for folder in [false, true] {
            assert_eq!(
                choices(&clash("docs", folder, KeepBoth), true),
                [(Keep, "Keep this"), (KeepBoth, "Keep both")]
            );
        }
    }

    #[test]
    fn the_sheet_says_what_is_already_here_and_what_differs() {
        let cases = [
            (
                sheet(&["a.txt"], vec![clash("a.txt", false, Replace)], 0),
                "a.txt is already here. This one differs:",
            ),
            (
                sheet(&["hello"], vec![clash("hello", true, KeepBoth)], 0),
                "hello is already here.",
            ),
            (
                sheet(
                    &["docs"],
                    vec![
                        clash("docs", true, KeepBoth),
                        clash("docs/a", false, Replace),
                        clash("docs/b", false, Keep),
                    ],
                    3,
                ),
                "docs is already here. 3 of its 5 files are the same. These 2 differ:",
            ),
            (
                sheet(
                    &["docs", "a.txt"],
                    vec![
                        clash("docs", true, KeepBoth),
                        clash("a.txt", false, Replace),
                    ],
                    0,
                ),
                "Some of these are already here. This one differs:",
            ),
        ];
        for (survey, expected) in cases {
            assert_eq!(clash_sentence(&survey).as_deref(), Some(expected));
        }
        assert_eq!(clash_sentence(&sheet(&["a.txt"], Vec::new(), 0)), None);
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
