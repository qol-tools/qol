use std::path::Path;
use std::time::Duration;

use qol_gpui::format_bytes;

use super::ops::Held;
use crate::ops::{Clash, NameProblem, Room, Survey, Verb};

pub(super) fn held_note(held: Held, count: usize) -> String {
    let word = match held {
        Held::Cut => "cut",
        Held::Copied => "copied",
    };
    format!("{count} {word}. Open a folder and press ctrl+v")
}

pub(super) fn drop_hint(copy: bool) -> &'static str {
    if copy {
        "Drop to copy. Release ctrl to move."
    } else {
        "Drop to move. Hold ctrl to copy."
    }
}

pub(super) fn name_of(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), crate::listing::display_name)
}

pub(crate) fn label_of(sources: &[std::path::PathBuf]) -> String {
    match sources {
        [one] => name_of(one),
        many => qol_gpui::plural(many.len(), "item", "items"),
    }
}

pub(super) fn title_of(label: &str, into: &Path) -> String {
    format!("{label} to {}", name_of(into))
}

fn present(verb: Verb) -> &'static str {
    match verb {
        Verb::Copy => "Copying",
        Verb::Move => "Moving",
        Verb::Duplicate => "Duplicating",
    }
}

fn past(verb: Verb) -> &'static str {
    match verb {
        Verb::Copy => "copied",
        Verb::Move => "moved",
        Verb::Duplicate => "duplicated",
    }
}

pub(super) fn of_total(done: u64, total: u64) -> String {
    let total_text = format_bytes(total);
    let done_text = format_bytes(done);
    match (done_text.rsplit_once(' '), total_text.rsplit_once(' ')) {
        (Some((number, unit)), Some((_, total_unit))) if unit == total_unit => {
            format!("{number} of {total_text}")
        }
        _ => format!("{done_text} of {total_text}"),
    }
}

pub(super) fn time_left(done: u64, total: u64, elapsed: Duration) -> Option<String> {
    if done == 0 || done >= total || elapsed < Duration::from_secs(1) {
        return None;
    }
    let rate = done as f64 / elapsed.as_secs_f64();
    let seconds = ((total - done) as f64 / rate).ceil() as u64;
    Some(if seconds < 60 {
        format!("{seconds} s left")
    } else {
        format!("{} min left", seconds.div_ceil(60))
    })
}

pub(super) fn running_progress(
    verb: Verb,
    label: &str,
    from: &str,
    done: u64,
    total: u64,
    elapsed: Duration,
) -> String {
    let mut parts = vec![format!("{} {label} from {from}", present(verb))];
    if total > 0 {
        parts.push(of_total(done, total));
    }
    parts.extend(time_left(done, total, elapsed));
    parts.join(" · ")
}

pub(super) fn running_detail(
    verb: Verb,
    from: &str,
    done: u64,
    total: u64,
    elapsed: Duration,
) -> String {
    let mut parts = vec![format!("{} from {from}", present(verb))];
    if total > 0 {
        parts.push(of_total(done, total));
    }
    parts.extend(time_left(done, total, elapsed));
    parts.join(" · ")
}

pub(super) fn arrived_note(
    verb: Verb,
    count: usize,
    here: bool,
    into: &str,
    from: &str,
    undoable: bool,
) -> String {
    let place = if here {
        "here".to_string()
    } else {
        format!("to {into}")
    };
    let undo = if undoable { " ctrl+z undoes it." } else { "" };
    format!("{count} {} {place} from {from}.{undo}", past(verb))
}

pub(super) fn trashed_note(count: usize) -> String {
    format!("{count} moved to the trash. ctrl+z puts them back.")
}

pub(super) fn erased_note(count: usize) -> String {
    format!(
        "{} deleted for good.",
        qol_gpui::plural(count, "item", "items")
    )
}

pub(crate) fn erase_lines(paths: &[std::path::PathBuf]) -> Vec<String> {
    let count = qol_gpui::plural(paths.len(), "item", "items");
    let head = match paths {
        [one] => format!("Delete {count} permanently? {}", name_of(one)),
        _ => format!("Delete {count} permanently?"),
    };
    vec![
        head,
        "They skip the trash and this cannot be undone.".to_string(),
    ]
}

pub(crate) fn erase_elsewhere_note() -> String {
    "Delete permanently only takes items in this folder, so open the folder that holds them first."
        .to_string()
}

pub(crate) fn short_note(survey: &Survey, volume: &str) -> Option<String> {
    let Room::Short { need, free } = survey.room else {
        return None;
    };
    let name = label_of(&survey.transfer.sources);
    Some(format!(
        "{name} is {}. {volume} has {} free.",
        format_bytes(need),
        format_bytes(free)
    ))
}

pub(crate) fn copy_only_note(survey: &Survey) -> Option<String> {
    match survey.copy_only.as_slice() {
        [] => None,
        [one] => Some(format!(
            "{} {}. It can be copied, not moved.",
            name_of(&one.path),
            one.reason
        )),
        many => Some(format!(
            "{} cannot be removed from where they are. They can be copied, not moved.",
            qol_gpui::plural(many.len(), "item", "items")
        )),
    }
}

pub(super) fn not_started_detail(survey: &Survey) -> String {
    match survey.room {
        Room::Short { need, free } => format!(
            "Not started. It is {} and only {} was free.",
            format_bytes(need),
            format_bytes(free)
        ),
        Room::Enough => "Not started.".to_string(),
    }
}

pub(super) fn kept_sentence(clashes: &[Clash], same_inside: usize) -> String {
    let kept: Vec<&Clash> = clashes
        .iter()
        .filter(|clash| clash.choice == crate::ops::Resolve::Keep)
        .collect();
    let mut text = String::new();
    if !kept.is_empty() {
        let newer = kept.iter().all(|clash| {
            matches!((clash.here.modified, clash.incoming.modified), (Some(here), Some(incoming)) if here >= incoming)
        });
        let count = kept.len();
        let files = if count == 1 { "file" } else { "files" };
        if newer {
            text.push_str(&format!("Kept the {count} newer {files} there."));
        } else {
            text.push_str(&format!("Kept {count} {files} there."));
        }
    }
    if same_inside > 0 {
        let others = if kept.is_empty() { "The" } else { " The other" };
        text.push_str(&format!("{others} {same_inside} were already the same."));
    }
    text
}

pub(super) struct Outcome<'a> {
    pub verb: Verb,
    pub bytes: u64,
    pub crosses_volume: bool,
    pub from_volume: &'a str,
    pub from_folder: &'a str,
    pub kept_originals: usize,
    pub failures: usize,
    pub kept: &'a str,
}

pub(super) fn done_detail(outcome: &Outcome) -> String {
    let mut text = match (outcome.verb, outcome.kept_originals) {
        (Verb::Move, 0) if outcome.crosses_volume => format!(
            "Moved from {}. The copy was checked before the originals were removed.",
            outcome.from_volume
        ),
        (Verb::Move, 0) => format!("Moved from {}.", outcome.from_folder),
        (Verb::Copy | Verb::Duplicate, 0) => format!("Copied {}.", format_bytes(outcome.bytes)),
        (_, _) => format!(
            "Copied {}, as you chose. The originals belong to another user and stay in {}.",
            format_bytes(outcome.bytes),
            outcome.from_folder
        ),
    };
    if !outcome.kept.is_empty() {
        text.push(' ');
        text.push_str(outcome.kept);
    }
    if outcome.failures > 0 {
        let word = past(outcome.verb);
        text.push_str(&format!(
            " {} could not be {word}.",
            qol_gpui::plural(outcome.failures, "item", "items")
        ));
    }
    text
}

pub(super) fn failure_text(failures: &[crate::ops::Failure]) -> Option<String> {
    let first = failures.first()?;
    let more = failures.len() - 1;
    let tail = if more > 0 {
        format!(" ({more} more)")
    } else {
        String::new()
    };
    Some(format!("{}: {}{tail}", name_of(&first.path), first.reason))
}

pub(super) fn problem_text(problem: &NameProblem, name: &str) -> String {
    match problem {
        NameProblem::Empty => "A name cannot be empty".to_string(),
        NameProblem::Taken => format!("{name} already exists here"),
        NameProblem::Forbidden(reason) => reason.clone(),
        NameProblem::TooLong => "That name is too long".to_string(),
        NameProblem::Io(reason) => reason.clone(),
    }
}
