use std::path::{Path, PathBuf};
use std::time::Duration;

use qol_gpui::format_bytes;

use super::ops::{Held, MoveState};
use crate::ops::history::{
    Action, Change, Direction, Outcome as Verdict, Reason, Source, Step, Top, Undone,
};
use crate::ops::{free_name, Clash, NameProblem, Room, Survey, Verb};
use crate::relay::Message;

pub(super) fn held_note(held: Held, count: usize, into: Option<&str>) -> String {
    let word = match held {
        Held::Cut => "cut",
        Held::Copied => "copied",
    };
    match into {
        Some(folder) => format!("{count} {word}. Press ctrl+v to paste into {folder}"),
        None => format!("{count} {word}. Open a folder and press ctrl+v"),
    }
}

pub(super) fn already_in(into: &Path, here: &Path) -> Message {
    if into == here {
        Message::warn("Already in this folder", "")
    } else {
        Message::warn(format!("Already in {}", name_of(into)), "")
    }
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
) -> Message {
    let place = if here {
        "here".to_string()
    } else {
        format!("to {into}")
    };
    let undo = if undoable { "ctrl+z undoes it." } else { "" };
    Message::info(format!("{count} {} {place} from {from}", past(verb)), undo)
}

pub(super) fn trashed_note(count: usize, undoable: bool) -> Message {
    let title = format!("{count} moved to the trash");
    match (undoable, count) {
        (false, _) => Message::info(title, ""),
        (true, 1) => Message::info(title, "ctrl+z puts it back."),
        (true, _) => Message::info(title, "ctrl+z puts them back."),
    }
}

pub(super) fn nothing_changed(into: &Path, here: &Path) -> Message {
    if into == here {
        Message::info("Nothing changed here", "")
    } else {
        Message::info(format!("Nothing changed in {}", name_of(into)), "")
    }
}

pub(super) fn stopped_before_the_end(label: &str) -> Message {
    Message::error(format!("{label} stopped before it finished"), "")
}

pub(super) fn stopped_detail(arrived: usize) -> String {
    match arrived {
        0 => "Stopped before it finished. Nothing was placed.".to_string(),
        1 => "Stopped before it finished. 1 item arrived, and ctrl+z undoes it.".to_string(),
        many => {
            format!("Stopped before it finished. {many} items arrived, and ctrl+z undoes them.")
        }
    }
}

pub(super) fn arrived_items(step: &Step) -> usize {
    step.items
        .iter()
        .filter(|item| {
            matches!(
                item.change,
                Change::Moved { .. } | Change::Copied { .. } | Change::Crossed { .. }
            )
        })
        .count()
}

pub(super) fn verb_noun(verb: Verb) -> &'static str {
    match verb {
        Verb::Copy => "copy",
        Verb::Move => "move",
        Verb::Duplicate => "duplicate",
    }
}

pub(super) fn word(direction: Direction) -> &'static str {
    match direction {
        Direction::Undo => "undo",
        Direction::Redo => "redo",
    }
}

fn done_word(direction: Direction) -> &'static str {
    match direction {
        Direction::Undo => "undone",
        Direction::Redo => "redone",
    }
}

fn lead_verb(direction: Direction) -> &'static str {
    match direction {
        Direction::Undo => "Undid",
        Direction::Redo => "Redid",
    }
}

fn acts(direction: Direction) -> &'static str {
    match direction {
        Direction::Undo => "undoes",
        Direction::Redo => "redoes",
    }
}

fn key_of(direction: Direction) -> &'static str {
    match direction {
        Direction::Undo => "ctrl+z",
        Direction::Redo => "ctrl+shift+z",
    }
}

fn opposite(direction: Direction) -> Direction {
    match direction {
        Direction::Undo => Direction::Redo,
        Direction::Redo => Direction::Undo,
    }
}

fn closing(direction: Direction) -> String {
    let other = opposite(direction);
    format!("{} {} it.", key_of(other), acts(other))
}

pub(super) fn nothing_to(direction: Direction) -> Message {
    Message::info(format!("Nothing to {}", word(direction)), "")
}

pub(super) fn refused(direction: Direction, when: &str) -> Message {
    Message::warn(
        format!("Nothing was {}", done_word(direction)),
        format!("Press {} again {when}.", key_of(direction)),
    )
}

pub(super) fn asked_off(direction: Direction) -> Message {
    Message::info(
        format!("Nothing was {}", done_word(direction)),
        format!("{} asks again.", key_of(direction)),
    )
}

pub(super) fn undoing_note(direction: Direction, label: &str) -> String {
    let doing = match direction {
        Direction::Undo => "Undoing",
        Direction::Redo => "Redoing",
    };
    format!("{doing} the {label}. Esc stops it.")
}

pub(super) fn menu_word(action: &Action) -> &'static str {
    match action {
        Action::Transfer { verb, .. } => verb_noun(*verb),
        Action::Rename { .. } => "rename",
        Action::Trash(_) => "move to trash",
        Action::NewFolder(_) => "new folder",
        Action::NewFile(_) => "new file",
        Action::Erase(_) => "permanent delete",
    }
}

pub(super) fn step_title(direction: Direction, action: &Action, did: bool) -> String {
    let label = step_label(action, Path::new(""));
    if did {
        format!("{} the {label}", lead_verb(direction))
    } else {
        format!("Could not {} the {label}", word(direction))
    }
}

pub(super) struct Spent {
    pub text: String,
    pub title: String,
    pub state: MoveState,
}

impl Spent {
    pub(super) fn message(&self, next: &str) -> Message {
        match self.state {
            MoveState::Failed => Message::warn(self.title.clone(), format!("{} {next}", self.text)),
            MoveState::Running | MoveState::Done | MoveState::NotStarted => {
                Message::info(self.title.clone(), next)
            }
        }
    }
}

pub(super) fn step_message(title: &str, body: &str, did: bool) -> Message {
    if did {
        Message::info(title, body)
    } else {
        Message::warn(title, body)
    }
}

pub(super) fn spent(action: &Action, here: &Path, cannot: bool) -> Spent {
    let whole = step_label(action, Path::new(""));
    if cannot {
        return Spent {
            text: "Delete permanently cannot be undone.".to_string(),
            title: format!("Cannot undo the {whole}"),
            state: MoveState::Failed,
        };
    }
    Spent {
        text: format!("The {} changed nothing.", step_label(action, here)),
        title: format!("The {whole} changed nothing"),
        state: MoveState::Done,
    }
}

pub(super) fn step_label(action: &Action, here: &Path) -> String {
    match action {
        Action::Transfer {
            verb,
            sources,
            into,
        } => {
            let mut text = format!("{} of {}", verb_noun(*verb), label_of(sources));
            if into != here {
                text.push_str(&format!(" into {}", name_of(into)));
            }
            text
        }
        Action::Rename { to, .. } => format!("rename to {}", name_of(to)),
        Action::Trash(paths) => format!("move of {} to the trash", label_of(paths)),
        Action::NewFolder(path) => format!("new folder {}", name_of(path)),
        Action::NewFile(path) => format!("new file {}", name_of(path)),
        Action::Erase(paths) => format!("permanent delete of {}", label_of(paths)),
    }
}

pub(super) fn next_text(top: Top<'_>, here: &Path) -> String {
    match top {
        Top::Nothing => "Nothing is left to undo.".to_string(),
        Top::Undo(step) => format!(
            "Next, ctrl+z undoes the {}.",
            step_label(&step.action, here)
        ),
        Top::Empty(step) => format!(
            "Next is the {}, which changed nothing.",
            step_label(&step.action, here)
        ),
        Top::Erased(step) => format!(
            "Next is the {}, which cannot be undone.",
            step_label(&step.action, here)
        ),
    }
}

pub(super) fn next_redo_text(step: Option<&Step>, here: &Path) -> String {
    match step {
        Some(step) => format!(
            "Next, ctrl+shift+z redoes the {}.",
            step_label(&step.action, here)
        ),
        None => "Nothing is left to redo.".to_string(),
    }
}

fn place_of(path: &Path, here: &Path) -> String {
    let folder = path.parent().unwrap_or(path);
    if folder == here {
        "here".to_string()
    } else {
        format!("in {}", name_of(folder))
    }
}

pub(crate) fn undo_sheet_lines(
    direction: Direction,
    clashes: &[Clash],
    here: &Path,
) -> Vec<String> {
    let Some(first) = clashes.first() else {
        return Vec::new();
    };
    let place = place_of(&first.existing, here);
    let (head, there) = match direction {
        Direction::Undo => ("Undo", "puts it back"),
        Direction::Redo => ("Redo", "puts it there"),
    };
    let each = match direction {
        Direction::Undo => "puts each one back",
        Direction::Redo => "puts each one there",
    };
    if let [one] = clashes {
        let parent = one.existing.parent().unwrap_or(&one.existing);
        let free = one
            .existing
            .file_name()
            .map_or_else(|| one.existing.clone(), |name| free_name(parent, name));
        return vec![
            format!("{head}: {} is already {place}.", name_of(&one.existing)),
            format!(
                "{head} never replaces anything. Keep both {there} as {}.",
                name_of(&free)
            ),
        ];
    }
    vec![
        format!("{head}: some of these are already {place}."),
        format!("{head} never replaces anything. Keep both {each} with a number after its name."),
    ]
}

struct Problem {
    text: String,
    named: bool,
}

impl Problem {
    fn named(text: String) -> Self {
        Self { text, named: true }
    }

    fn worded(text: String) -> Self {
        Self { text, named: false }
    }

    fn sentence(&self) -> String {
        if self.named {
            return self.text.clone();
        }
        let mut letters = self.text.chars();
        match letters.next() {
            Some(first) => first.to_uppercase().chain(letters).collect(),
            None => String::new(),
        }
    }
}

fn problem_of(reason: &Reason, direction: Direction) -> Option<Problem> {
    let redo = direction == Direction::Redo;
    let problem = match reason {
        Reason::Gone(path) => Problem::named(format!("{} is gone.", name_of(path))),
        Reason::Changed(path) => Problem::named(format!("{} changed, so it stays.", name_of(path))),
        Reason::OriginalGone(path) if redo => Problem::worded(format!(
            "the original {} is gone, so it is not copied again.",
            name_of(path)
        )),
        Reason::OriginalGone(path) => Problem::worded(format!(
            "the original {} is gone, so the copy stays.",
            name_of(path)
        )),
        Reason::OriginalChanged(path) if redo => Problem::worded(format!(
            "the original {} changed, so it is not copied again.",
            name_of(path)
        )),
        Reason::OriginalChanged(path) => Problem::worded(format!(
            "the original {} changed, so the copy stays.",
            name_of(path)
        )),
        Reason::NotInTrash(path) => {
            Problem::named(format!("{} is no longer in the trash.", name_of(path)))
        }
        Reason::Taken(path) => Problem::worded(format!(
            "the name {} is taken, so it stays where it is.",
            name_of(path)
        )),
        Reason::Kept(path) => Problem::named(format!(
            "{} stays where it is, as you chose.",
            name_of(path)
        )),
        Reason::Partly(path) => Problem::named(format!(
            "{} was only partly moved, so its files stay in both places.",
            name_of(path)
        )),
        Reason::CannotUndo(path) if redo => {
            Problem::named(format!("{} cannot be redone from here.", name_of(path)))
        }
        Reason::CannotUndo(path) => Problem::named(format!(
            "{} cannot be put back from here, but your desktop's trash can restore it.",
            name_of(path)
        )),
        Reason::NoTrash(path, reason) | Reason::Failed(path, reason) => {
            Problem::named(format!("{} stays where it is: {reason}.", name_of(path)))
        }
        Reason::Unplugged(path) => Problem::worded(format!(
            "the drive of {} is not plugged in, so it stays for now.",
            name_of(path)
        )),
        Reason::Stopped => return None,
    };
    Some(problem)
}

fn kept_copy(change: &Change, undone: &Undone) -> Option<Problem> {
    let (
        Change::Crossed { to: copy, .. },
        Undone::CopiedBack {
            kept: Some(reason), ..
        },
    ) = (change, undone)
    else {
        return None;
    };
    let folder = copy.parent().map(name_of).unwrap_or_default();
    Some(Problem::worded(format!(
        "the copy of {} in {folder} stays: {reason}.",
        name_of(copy)
    )))
}

fn kept_source(change: &Change) -> Option<Problem> {
    let Change::Crossed {
        from,
        source: Source::Partly,
        ..
    } = change
    else {
        return None;
    };
    let folder = from.parent().map(name_of).unwrap_or_default();
    Some(Problem::worded(format!(
        "the original {} in {folder} stays too, so it is in both places.",
        name_of(from)
    )))
}

fn back_to(undone: &Undone) -> Option<&Path> {
    match undone {
        Undone::MovedBack { to, .. } | Undone::CopiedBack { to, .. } => Some(to),
        Undone::Binned(_) | Undone::Restored { .. } | Undone::Removed | Undone::Remade { .. } => {
            None
        }
    }
}

fn binned(undone: &Undone) -> bool {
    matches!(undone, Undone::Binned(_))
}

fn undo_success_text(action: &Action, undone: &[(&Change, &Undone)], here: &Path) -> String {
    let lead = format!("Undid the {}.", step_label(action, here));
    let main = match action {
        Action::Transfer { verb, .. } => transfer_text(*verb, undone, here),
        Action::Rename { .. } => undone
            .iter()
            .find_map(|(_, done)| back_to(done))
            .map(|to| format!("Renamed back to {}.", name_of(to))),
        Action::Trash(_) => {
            let restored: Vec<PathBuf> = undone
                .iter()
                .filter_map(|(_, done)| match done {
                    Undone::Restored { to, .. } => Some(to.clone()),
                    Undone::MovedBack { .. }
                    | Undone::CopiedBack { .. }
                    | Undone::Binned(_)
                    | Undone::Removed
                    | Undone::Remade { .. } => None,
                })
                .collect();
            restored.first().map(|first| {
                format!(
                    "Put {} back {}.",
                    label_of(&restored),
                    place_of(first, here)
                )
            })
        }
        Action::NewFolder(path) => undone
            .iter()
            .any(|(_, done)| matches!(done, Undone::Removed))
            .then(|| format!("Removed the new folder {}, which was empty.", name_of(path))),
        Action::NewFile(path) => undone
            .iter()
            .any(|(_, done)| binned(done))
            .then(|| format!("The new file {} went to the trash.", name_of(path))),
        Action::Erase(_) => None,
    };
    let mut text = main.unwrap_or(lead);
    for (change, done) in undone {
        if let (Change::Replaced { at, .. }, Undone::Restored { .. }) = (change, done) {
            text.push_str(&format!(" The old {} is back.", name_of(at)));
        }
    }
    text
}

fn transfer_text(verb: Verb, undone: &[(&Change, &Undone)], here: &Path) -> Option<String> {
    match verb {
        Verb::Copy | Verb::Duplicate => {
            let count = undone.iter().filter(|(_, done)| binned(done)).count();
            let noun = if verb == Verb::Copy {
                ("copy", "copies")
            } else {
                ("duplicate", "duplicates")
            };
            let word = if count == 1 { noun.0 } else { noun.1 };
            (count > 0).then(|| format!("The {word} went to the trash."))
        }
        Verb::Move => {
            let backs: Vec<&Path> = undone
                .iter()
                .filter_map(|(_, done)| back_to(done))
                .collect();
            let place = place_of(backs.first()?, here);
            if backs.len() == 1 {
                Some(format!("It is back {place}."))
            } else {
                Some(format!("They are back {place}."))
            }
        }
    }
}

fn landing_of(change: &Change) -> Option<&Path> {
    match change {
        Change::Moved { to, .. } | Change::Copied { to, .. } | Change::Crossed { to, .. } => {
            Some(to)
        }
        Change::Replaced { .. }
        | Change::Matched { .. }
        | Change::Emptied { .. }
        | Change::Made { .. }
        | Change::Trashed { .. }
        | Change::Erased { .. } => None,
    }
}

fn again(count: usize, one: &str, many: &str, place: &str) -> String {
    let (subject, be) = if count == 1 {
        (one, "is")
    } else {
        (many, "are")
    };
    format!("{subject} {be} {place} again.")
}

fn redo_transfer_text(verb: Verb, landed: &[&Path], here: &Path) -> Option<String> {
    let place = place_of(landed.first()?, here);
    let text = match verb {
        Verb::Move => again(landed.len(), "It", "They", &place),
        Verb::Copy => again(landed.len(), "The copy", "The copies", &place),
        Verb::Duplicate => again(landed.len(), "The duplicate", "The duplicates", &place),
    };
    Some(text)
}

fn redo_success_text(action: &Action, redone: &[&Change], here: &Path) -> String {
    let lead = format!("Redid the {}.", step_label(action, here));
    let landed: Vec<&Path> = redone.iter().copied().filter_map(landing_of).collect();
    let trashed = redone
        .iter()
        .filter(|change| matches!(change, Change::Trashed { .. }))
        .count();
    let made = redone
        .iter()
        .any(|change| matches!(change, Change::Made { .. }));
    let main = match action {
        Action::Transfer { verb, .. } => redo_transfer_text(*verb, &landed, here),
        Action::Rename { .. } => landed
            .first()
            .map(|to| format!("It is named {} again.", name_of(to))),
        Action::Trash(_) => (trashed > 0).then(|| again(trashed, "It", "They", "in the trash")),
        Action::NewFolder(path) => {
            made.then(|| format!("The new folder {} is back.", name_of(path)))
        }
        Action::NewFile(path) => made.then(|| format!("The new file {} is back.", name_of(path))),
        Action::Erase(_) => None,
    };
    let mut text = main.unwrap_or(lead);
    for change in redone {
        if let Change::Replaced { at, .. } = change {
            text.push_str(&format!(" The old {} went to the trash.", name_of(at)));
        }
    }
    text
}

pub(super) struct Report<'a> {
    pub direction: Direction,
    pub stopped: bool,
    pub here: &'a Path,
    pub next: &'a str,
    pub reversible: bool,
}

pub(super) fn report_note(
    step: &Step,
    outcomes: &[(usize, Verdict)],
    report: &Report<'_>,
) -> String {
    let direction = report.direction;
    let mut undone: Vec<(&Change, &Undone)> = Vec::new();
    let mut redone: Vec<&Change> = Vec::new();
    let mut problems: Vec<Problem> = Vec::new();
    let mut stopped = report.stopped;
    let mut retry = false;
    for (index, outcome) in outcomes {
        let Some(item) = step.items.get(*index) else {
            continue;
        };
        match outcome {
            Verdict::Undone(done) => {
                undone.push((&item.change, done));
                problems.extend(kept_copy(&item.change, done));
            }
            Verdict::Redone(change) => {
                redone.push(change);
                problems.extend(kept_source(change));
            }
            Verdict::Stays(Reason::Stopped) => stopped = true,
            Verdict::Stays(reason) => {
                retry = true;
                problems.extend(problem_of(reason, direction));
            }
            Verdict::Leaves(reason) => problems.extend(problem_of(reason, direction)),
        }
    }
    let key = key_of(direction);
    let tail = if stopped {
        format!(
            " The {} stopped before it finished. {key} {} the rest.",
            word(direction),
            acts(direction)
        )
    } else if retry {
        format!(" {key} tries again.")
    } else {
        String::new()
    };
    let more = match problems.len().saturating_sub(1) {
        0 => String::new(),
        1 => " 1 more stays as it is.".to_string(),
        many => format!(" {many} more stay as they are."),
    };
    if undone.is_empty() && redone.is_empty() {
        let mut text = match problems.first() {
            Some(first) => format!("Nothing was {}: {}{more}", done_word(direction), first.text),
            None => format!("Nothing was {}.", done_word(direction)),
        };
        text.push_str(&tail);
        if tail.is_empty() {
            text.push(' ');
            text.push_str(report.next);
        }
        return text;
    }
    let mut text = match direction {
        Direction::Undo => undo_success_text(&step.action, &undone, report.here),
        Direction::Redo => redo_success_text(&step.action, &redone, report.here),
    };
    if let Some(first) = problems.first() {
        text.push(' ');
        text.push_str(&first.sentence());
        text.push_str(&more);
    }
    text.push_str(&tail);
    if report.reversible {
        text.push(' ');
        text.push_str(&closing(direction));
    }
    text
}

pub(super) fn erased_note(count: usize) -> Message {
    Message::info(
        format!(
            "{} deleted for good",
            qol_gpui::plural(count, "item", "items")
        ),
        "",
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

pub(crate) fn erase_elsewhere_note() -> Message {
    Message::warn(
        "Delete permanently only takes items in this folder",
        "Open the folder that holds them first.",
    )
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
        .filter(|clash| clash.choice == crate::ops::Resolve::Keep && clash.applies(clashes))
        .collect();
    let mut text = String::new();
    if !kept.is_empty() {
        let folders = kept.iter().filter(|clash| clash.merges()).count();
        let newer = folders == 0
            && kept.iter().all(|clash| {
                matches!((clash.here.modified, clash.incoming.modified), (Some(here), Some(incoming)) if here >= incoming)
            });
        let count = kept.len();
        let (one, many) = if folders == 0 {
            ("file", "files")
        } else if folders == count {
            ("folder", "folders")
        } else {
            ("item", "items")
        };
        let noun = if count == 1 { one } else { many };
        if newer {
            text.push_str(&format!("Kept the {count} newer {noun} there."));
        } else {
            text.push_str(&format!("Kept {count} {noun} there."));
        }
    }
    let declined = clashes
        .iter()
        .any(|clash| clash.merges() && clash.choice != crate::ops::Resolve::Merge);
    if same_inside > 0 && !declined {
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

pub(super) fn failure_text(
    verb: &str,
    after: &str,
    failures: &[crate::ops::Failure],
) -> Option<Message> {
    let first = failures.first()?;
    let more = failures.len() - 1;
    let tail = if more > 0 {
        format!(" ({more} more)")
    } else {
        String::new()
    };
    let items = qol_gpui::plural(failures.len(), "item", "items");
    Some(Message::error(
        format!("Could not {verb} {items}{after}"),
        format!("{}: {}{tail}", name_of(&first.path), first.reason),
    ))
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

pub(super) fn name_problem(problem: &NameProblem, name: &str, what: &str) -> Message {
    let title = format!("Could not {what} {name}");
    let body = problem_text(problem, name);
    match problem {
        NameProblem::Io(_) => Message::error(title, body),
        NameProblem::Empty
        | NameProblem::Taken
        | NameProblem::Forbidden(_)
        | NameProblem::TooLong => Message::warn(title, body),
    }
}
