use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

use super::{start_undo, Action, Change, Choices, History, Outcome, Plan, Reply, Undoing};
use crate::ops::bin::Held;
use crate::ops::print::{Print, TreePrint};
use crate::ops::{Clash, Resolve, Tools};

const WAIT: Duration = Duration::from_secs(10);

fn stamp(path: &Path, age_secs: u64) {
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(age_secs))
        .unwrap();
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
    stamp(path, 1000);
}

fn edit(path: &Path, text: &str) {
    let mut file = File::options()
        .write(true)
        .truncate(true)
        .open(path)
        .unwrap();
    file.write_all(text.as_bytes()).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(5))
        .unwrap();
}

fn replace(path: &Path, text: &str) {
    let swap = path.with_file_name(".swap");
    write(&swap, text);
    fs::rename(&swap, path).unwrap();
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}

fn names(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

fn print(path: &Path) -> Print {
    Print::read(path).unwrap()
}

fn plan_of(changes: Vec<Change>) -> Plan {
    let mut history = History::default();
    let step = history.open(Action::Trash(Vec::new()));
    for change in changes {
        history.record(step, change);
    }
    history.plan(step).unwrap()
}

fn collect(undoing: Undoing) -> Vec<Reply> {
    let mut replies = Vec::new();
    while let Ok(reply) = undoing.result.recv_timeout(WAIT) {
        let last = matches!(reply, Reply::Done | Reply::Asks(_));
        replies.push(reply);
        if last {
            break;
        }
    }
    replies
}

fn settled(replies: Vec<Reply>) -> Vec<(usize, Outcome)> {
    let mut outcomes = Vec::new();
    let mut done = false;
    for reply in replies {
        match reply {
            Reply::Item(index, outcome) => outcomes.push((index, outcome)),
            Reply::Done => done = true,
            Reply::Asks(clashes) => panic!("it asked instead of undoing: {clashes:?}"),
        }
    }
    assert!(done, "the undo never reported that it was done");
    outcomes
}

fn only(replies: Vec<Reply>) -> Outcome {
    let mut outcomes = settled(replies);
    assert_eq!(outcomes.len(), 1, "{outcomes:?}");
    let (index, outcome) = outcomes.remove(0);
    assert_eq!(index, 0);
    outcome
}

fn asked(replies: &[Reply]) -> &[(usize, Clash)] {
    match replies {
        [Reply::Asks(clashes)] => clashes,
        other => panic!("expected one ask, got {other:?}"),
    }
}

struct Lab {
    work: TempDir,
    tools_dir: TempDir,
    tools: Tools,
}

impl Lab {
    fn new() -> Self {
        let work = tempfile::tempdir().unwrap();
        let tools_dir = tempfile::tempdir().unwrap();
        let tools = Tools::at(tools_dir.path());
        Self {
            work,
            tools_dir,
            tools,
        }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.work.path().join(relative)
    }

    fn tree(&self, path: &Path) -> TreePrint {
        TreePrint::read(path, &self.tools.key).unwrap()
    }

    fn held(&self, path: &Path) -> Held {
        self.tools.bin.trash(path).unwrap().unwrap()
    }

    fn binned(&self) -> Vec<String> {
        fs::read_dir(self.tools_dir.path().join("bin"))
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn run(&self, changes: Vec<Change>, choices: Option<Choices>) -> Vec<Reply> {
        self.run_plan(plan_of(changes), choices)
    }

    fn run_plan(&self, plan: Plan, choices: Option<Choices>) -> Vec<Reply> {
        collect(start_undo(plan, choices, self.tools.clone(), None, || {}))
    }

    fn undo(&self, change: Change) -> Outcome {
        only(self.run(vec![change], Some(Choices::new())))
    }

    fn undo_choosing(&self, change: Change, choice: Resolve) -> Outcome {
        only(self.run(vec![change], Some(Choices::from([(0, choice)]))))
    }
}

mod steps {
    use super::*;
    use crate::ops::history::{Reason, Settled, Source, State, Step, StepId, Top, Undone};
    use crate::ops::print::Kind;

    fn trash_action() -> Action {
        Action::Trash(Vec::new())
    }

    fn erased(name: &str) -> Change {
        Change::Erased {
            path: PathBuf::from(name),
        }
    }

    fn replaced(at: &str) -> Change {
        Change::Replaced {
            at: PathBuf::from(at),
            held: None,
        }
    }

    fn matched(from: &str) -> Change {
        Change::Matched {
            from: PathBuf::from(from),
            held: None,
        }
    }

    fn crossed(to: &str, source: Source) -> Change {
        let blank = Print {
            id: None,
            kind: Kind::File,
            len: 0,
            modified: None,
            born: None,
        };
        Change::Crossed {
            from: PathBuf::from("src"),
            to: PathBuf::from(to),
            tree: TreePrint {
                root: blank,
                entries: 0,
                bytes: 0,
                shape: 0,
                objects: 0,
            },
            source,
            from_device: None,
        }
    }

    fn closed(history: &mut History, changes: Vec<Change>) -> StepId {
        let step = history.open(trash_action());
        for change in changes {
            history.record(step, change);
        }
        history.close(step);
        step
    }

    fn changes(history: &History, step: StepId) -> Vec<Change> {
        history
            .step(step)
            .unwrap()
            .items
            .iter()
            .map(|item| item.change.clone())
            .collect()
    }

    fn settle(history: &mut History, step: StepId, outcomes: Vec<(usize, Outcome)>) {
        history.settle(&Settled { step, outcomes });
    }

    fn undone() -> Outcome {
        Outcome::Undone(Undone::Removed)
    }

    fn found(history: &History, step: StepId) -> &Step {
        history.step(step).unwrap()
    }

    #[test]
    fn an_empty_history_has_nothing_to_undo() {
        assert!(matches!(History::default().top(), Top::Nothing));
    }

    #[test]
    fn a_closed_step_without_items_is_empty() {
        let mut history = History::default();
        let step = closed(&mut history, Vec::new());
        assert!(matches!(history.top(), Top::Empty(top) if top.id == step));
    }

    #[test]
    fn top_skips_open_steps() {
        let mut history = History::default();
        let older = closed(&mut history, vec![erased("a")]);
        let newer = history.open(trash_action());
        history.record(newer, erased("b"));
        assert!(matches!(history.top(), Top::Undo(top) if top.id == older));
    }

    #[test]
    fn top_with_only_an_open_step_is_nothing() {
        let mut history = History::default();
        let step = history.open(trash_action());
        history.record(step, erased("a"));
        assert!(matches!(history.top(), Top::Nothing));
    }

    #[test]
    fn top_skips_fully_undone_steps() {
        let mut history = History::default();
        let older = closed(&mut history, vec![erased("a")]);
        let newer = closed(&mut history, vec![erased("b")]);
        settle(&mut history, newer, vec![(0, undone())]);
        assert!(matches!(history.top(), Top::Undo(top) if top.id == older));
    }

    #[test]
    fn an_erase_step_with_items_is_erased() {
        let mut history = History::default();
        let step = history.open(Action::Erase(vec![PathBuf::from("a")]));
        history.record(step, erased("a"));
        history.close(step);
        assert!(matches!(history.top(), Top::Erased(top) if top.id == step));
    }

    #[test]
    fn an_erase_step_without_items_is_empty() {
        let mut history = History::default();
        let step = history.open(Action::Erase(vec![PathBuf::from("a")]));
        history.close(step);
        assert!(matches!(history.top(), Top::Empty(top) if top.id == step));
    }

    #[test]
    fn plan_lists_done_items_newest_first_and_marks_the_tried_ones() {
        let mut history = History::default();
        let step = closed(&mut history, vec![erased("a"), erased("b"), erased("c")]);
        let failed = Outcome::Stays(Reason::Failed(PathBuf::from("b"), "busy".into()));
        settle(&mut history, step, vec![(1, failed)]);
        let plan = history.plan(step).unwrap();
        assert_eq!(plan.step, step);
        let listed: Vec<(usize, bool)> = plan
            .items
            .iter()
            .map(|planned| (planned.index, planned.retry))
            .collect();
        assert_eq!(listed, vec![(2, false), (1, true), (0, false)]);
        assert_eq!(plan.items[0].change, erased("c"));
    }

    #[test]
    fn plan_leaves_out_undone_items_and_keeps_the_original_indexes() {
        let mut history = History::default();
        let step = closed(&mut history, vec![erased("a"), erased("b"), erased("c")]);
        settle(&mut history, step, vec![(2, undone())]);
        let listed: Vec<usize> = history
            .plan(step)
            .unwrap()
            .items
            .iter()
            .map(|planned| planned.index)
            .collect();
        assert_eq!(listed, vec![1, 0]);
    }

    #[test]
    fn plan_of_a_dropped_step_is_none() {
        let mut history = History::default();
        let step = closed(&mut history, vec![erased("a")]);
        history.drop_step(step);
        assert_eq!(history.plan(step), None);
        assert!(history.step(step).is_none());
    }

    #[test]
    fn settle_undone_marks_the_item_and_keeps_it() {
        let mut history = History::default();
        let step = closed(&mut history, vec![erased("a"), erased("b")]);
        settle(&mut history, step, vec![(1, undone())]);
        let items = &found(&history, step).items;
        assert_eq!(items[0].state, State::Done);
        assert_eq!(items[1].state, State::Undone(Undone::Removed));
        assert_eq!(found(&history, step).waiting(), 1);
    }

    #[test]
    fn settle_stopped_changes_nothing() {
        let mut history = History::default();
        let step = closed(&mut history, vec![erased("a"), erased("b")]);
        let before = found(&history, step).clone();
        settle(
            &mut history,
            step,
            vec![
                (0, Outcome::Stays(Reason::Stopped)),
                (1, Outcome::Stays(Reason::Stopped)),
            ],
        );
        assert_eq!(found(&history, step), &before);
    }

    #[test]
    fn settle_any_other_stay_marks_the_item_tried() {
        let place = PathBuf::from("a");
        let reasons = [
            Reason::Gone(place.clone()),
            Reason::Changed(place.clone()),
            Reason::Taken(place.clone()),
            Reason::Unplugged(place.clone()),
            Reason::Failed(place.clone(), "busy".into()),
            Reason::NoTrash(place, "no trash".into()),
        ];
        for reason in reasons {
            let mut history = History::default();
            let step = closed(&mut history, vec![erased("a")]);
            settle(
                &mut history,
                step,
                vec![(0, Outcome::Stays(reason.clone()))],
            );
            let item = &found(&history, step).items[0];
            assert!(item.tried, "{reason:?}");
            assert_eq!(item.state, State::Done, "{reason:?}");
        }
    }

    #[test]
    fn settle_leaves_removes_the_item_and_nothing_else() {
        let mut history = History::default();
        let step = closed(&mut history, vec![erased("a"), erased("b"), erased("c")]);
        let leaves = Outcome::Leaves(Reason::CannotUndo(PathBuf::from("b")));
        settle(&mut history, step, vec![(1, leaves)]);
        assert_eq!(changes(&history, step), vec![erased("a"), erased("c")]);
    }

    #[test]
    fn settle_removes_several_leaving_items_whatever_their_order() {
        let mut history = History::default();
        let step = closed(
            &mut history,
            vec![erased("a"), erased("b"), erased("c"), erased("d")],
        );
        let leaves = |name: &str| Outcome::Leaves(Reason::CannotUndo(PathBuf::from(name)));
        settle(
            &mut history,
            step,
            vec![(2, leaves("c")), (0, leaves("a")), (2, leaves("c"))],
        );
        assert_eq!(changes(&history, step), vec![erased("b"), erased("d")]);
    }

    #[test]
    fn settle_ignores_an_index_the_step_does_not_have() {
        let mut history = History::default();
        let step = closed(&mut history, vec![erased("a")]);
        let before = found(&history, step).clone();
        let leaves = Outcome::Leaves(Reason::CannotUndo(PathBuf::from("z")));
        settle(&mut history, step, vec![(7, leaves), (9, undone())]);
        assert_eq!(found(&history, step), &before);
    }

    #[test]
    fn a_step_left_with_no_items_is_dropped() {
        let mut history = History::default();
        let kept = closed(&mut history, vec![erased("a")]);
        let step = closed(&mut history, vec![erased("b"), erased("c")]);
        let leaves = |name: &str| Outcome::Leaves(Reason::CannotUndo(PathBuf::from(name)));
        settle(&mut history, step, vec![(0, leaves("b")), (1, leaves("c"))]);
        assert!(history.step(step).is_none());
        assert!(history.step(kept).is_some());
        assert!(matches!(history.top(), Top::Undo(top) if top.id == kept));
    }

    #[test]
    fn a_tried_item_that_fails_again_leaves_the_step() {
        let mut history = History::default();
        let step = closed(&mut history, vec![erased("a")]);
        let failed = || Outcome::Stays(Reason::Failed(PathBuf::from("a"), "busy".into()));
        settle(&mut history, step, vec![(0, failed())]);
        assert!(history.plan(step).unwrap().items[0].retry);
        let leaves = Outcome::Leaves(Reason::Failed(PathBuf::from("a"), "busy".into()));
        settle(&mut history, step, vec![(0, leaves)]);
        assert!(history.step(step).is_none());
    }

    #[test]
    fn amend_updates_only_the_crossed_source_that_landed_there() {
        let mut history = History::default();
        let step = history.open(trash_action());
        for change in [
            crossed("b", Source::Removing),
            crossed("c", Source::Removing),
            erased("b"),
        ] {
            history.record(step, change);
        }
        history.amend(step, Path::new("b"), Source::Removed);
        assert_eq!(
            changes(&history, step),
            vec![
                crossed("b", Source::Removed),
                crossed("c", Source::Removing),
                erased("b"),
            ]
        );
    }

    #[test]
    fn close_turns_a_removing_source_into_partly_and_nothing_else() {
        let mut history = History::default();
        let step = history.open(trash_action());
        for change in [
            crossed("a", Source::Removing),
            crossed("b", Source::Removed),
            crossed("c", Source::Partly),
            erased("d"),
        ] {
            history.record(step, change);
        }
        assert!(found(&history, step).open);
        history.close(step);
        assert!(!found(&history, step).open);
        assert_eq!(
            changes(&history, step),
            vec![
                crossed("a", Source::Partly),
                crossed("b", Source::Removed),
                crossed("c", Source::Partly),
                erased("d"),
            ]
        );
    }

    #[test]
    fn retract_drops_the_replaced_item_at_that_place_only() {
        let mut history = History::default();
        let step = history.open(trash_action());
        for change in [
            replaced("a"),
            replaced("b"),
            matched("a"),
            erased("a"),
            replaced("a"),
        ] {
            history.record(step, change);
        }
        history.retract(step, Path::new("a"));
        assert_eq!(
            changes(&history, step),
            vec![replaced("b"), matched("a"), erased("a")]
        );
    }

    #[test]
    fn at_most_a_hundred_steps_are_kept() {
        let mut history = History::default();
        let ids: Vec<StepId> = (0..105)
            .map(|_| closed(&mut history, vec![erased("a")]))
            .collect();
        let kept: Vec<StepId> = history.steps().iter().map(|step| step.id).collect();
        assert_eq!(kept, ids[5..].to_vec());
    }

    #[test]
    fn an_open_step_is_never_trimmed() {
        let mut history = History::default();
        let first = history.open(trash_action());
        for _ in 0..120 {
            closed(&mut history, vec![erased("a")]);
        }
        assert_eq!(history.steps().len(), 100);
        assert!(history.step(first).is_some());
    }

    #[test]
    fn open_steps_alone_may_pass_the_limit() {
        let mut history = History::default();
        for _ in 0..101 {
            history.open(trash_action());
        }
        assert_eq!(history.steps().len(), 101);
    }

    #[test]
    fn open_drops_fully_undone_steps_and_keeps_partly_undone_ones() {
        let mut history = History::default();
        let whole = closed(&mut history, vec![erased("a")]);
        let partly = closed(&mut history, vec![erased("b"), erased("c")]);
        settle(&mut history, whole, vec![(0, undone())]);
        settle(&mut history, partly, vec![(0, undone())]);
        assert_eq!(history.steps().len(), 2);
        history.open(trash_action());
        assert!(history.step(whole).is_none());
        assert!(history.step(partly).is_some());
    }

    #[test]
    fn step_ids_are_never_reused() {
        let mut history = History::default();
        let first = closed(&mut history, Vec::new());
        history.drop_step(first);
        let second = history.open(trash_action());
        assert_ne!(first, second);
    }
}

mod disk {
    use super::*;
    use crate::ops::history::{Reason, Source, Undone};

    fn moved(from: &Path, to: &Path) -> Change {
        Change::Moved {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            at: print(to),
        }
    }

    fn copied(lab: &Lab, from: &Path, to: &Path) -> Change {
        Change::Copied {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            tree: lab.tree(to),
            original: lab.tree(from),
        }
    }

    fn crossed(lab: &Lab, from: &Path, to: &Path, source: Source) -> Change {
        Change::Crossed {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            tree: lab.tree(to),
            source,
            from_device: print(from.parent().unwrap()).device(),
        }
    }

    fn emptied(folder: &Path) -> Change {
        Change::Emptied {
            folder: folder.to_path_buf(),
            at: print(folder),
        }
    }

    fn made(path: &Path) -> Change {
        Change::Made {
            path: path.to_path_buf(),
            at: print(path),
        }
    }

    fn taken_home(lab: &Lab) -> (PathBuf, PathBuf) {
        let from = lab.path("old/a.txt");
        let to = lab.path("new/a.txt");
        write(&from, "occupant");
        write(&to, "moved");
        (from, to)
    }

    fn copy_pair(lab: &Lab) -> (PathBuf, PathBuf) {
        let from = lab.path("src/a.txt");
        let to = lab.path("dst/a.txt");
        write(&from, "contents");
        write(&to, "contents");
        (from, to)
    }

    fn copy_trees(lab: &Lab) -> (PathBuf, PathBuf) {
        let from = lab.path("src/tree");
        let to = lab.path("dst/tree");
        for root in [&from, &to] {
            write(&root.join("a.txt"), "top");
            write(&root.join("sub/b.txt"), "inner");
        }
        (from, to)
    }

    type Mutate = fn(&Path, &Path);
    type Expect = fn(&Path, &Path) -> Outcome;
    type Restore = fn(PathBuf, Option<Held>) -> Change;

    fn restores() -> [(&'static str, Restore); 3] {
        [
            ("replaced", |at: PathBuf, held: Option<Held>| {
                Change::Replaced { at, held }
            }),
            ("matched", |from: PathBuf, held: Option<Held>| {
                Change::Matched { from, held }
            }),
            ("trashed", |from: PathBuf, held: Option<Held>| {
                Change::Trashed { from, held }
            }),
        ]
    }

    #[test]
    fn moved_item_goes_back_to_its_old_place() {
        let lab = Lab::new();
        let from = lab.path("old/a.txt");
        let to = lab.path("new/a.txt");
        fs::create_dir_all(from.parent().unwrap()).unwrap();
        write(&to, "hello");
        let outcome = lab.undo(moved(&from, &to));
        assert_eq!(
            outcome,
            Outcome::Undone(Undone::MovedBack { to: from.clone() })
        );
        assert_eq!(read(&from), "hello");
        assert!(!to.exists());
    }

    #[test]
    fn moved_item_makes_its_old_folder_when_that_is_gone() {
        let lab = Lab::new();
        let from = lab.path("gone/deep/a.txt");
        let to = lab.path("new/a.txt");
        write(&to, "hello");
        let outcome = lab.undo(moved(&from, &to));
        assert_eq!(
            outcome,
            Outcome::Undone(Undone::MovedBack { to: from.clone() })
        );
        assert_eq!(read(&from), "hello");
    }

    #[test]
    fn moved_item_replaced_by_another_file_leaves_changed_and_nothing_moves() {
        let lab = Lab::new();
        let from = lab.path("old/a.txt");
        let to = lab.path("new/a.txt");
        fs::create_dir_all(from.parent().unwrap()).unwrap();
        write(&to, "mine");
        let change = moved(&from, &to);
        replace(&to, "other");
        let outcome = lab.undo(change);
        assert_eq!(outcome, Outcome::Leaves(Reason::Changed(to.clone())));
        assert_eq!(read(&to), "other");
        assert!(!from.exists());
    }

    #[test]
    fn moved_item_that_is_gone_leaves_gone() {
        let lab = Lab::new();
        let from = lab.path("old/a.txt");
        let to = lab.path("new/a.txt");
        fs::create_dir_all(from.parent().unwrap()).unwrap();
        write(&to, "mine");
        let change = moved(&from, &to);
        fs::remove_file(&to).unwrap();
        let outcome = lab.undo(change);
        assert_eq!(outcome, Outcome::Leaves(Reason::Gone(to)));
        assert!(!from.exists());
    }

    #[test]
    fn moved_item_that_was_edited_still_goes_back() {
        let lab = Lab::new();
        let from = lab.path("old/a.txt");
        let to = lab.path("new/a.txt");
        fs::create_dir_all(from.parent().unwrap()).unwrap();
        write(&to, "before");
        let change = moved(&from, &to);
        edit(&to, "after the edit");
        let outcome = lab.undo(change);
        assert_eq!(
            outcome,
            Outcome::Undone(Undone::MovedBack { to: from.clone() })
        );
        assert_eq!(read(&from), "after the edit");
        assert!(!to.exists());
    }

    #[test]
    fn moved_home_taken_with_no_choice_asks_before_touching_anything() {
        let lab = Lab::new();
        let (from, to) = taken_home(&lab);
        let replies = lab.run(vec![moved(&from, &to)], None);
        let clashes = asked(&replies);
        assert_eq!(clashes.len(), 1);
        let (index, clash) = &clashes[0];
        assert_eq!(*index, 0);
        assert_eq!(clash.existing, from);
        assert_eq!(clash.source, to);
        assert_eq!(clash.choice, Resolve::KeepBoth);
        assert_eq!(read(&from), "occupant");
        assert_eq!(read(&to), "moved");
    }

    #[test]
    fn moved_home_taken_with_keep_both_lands_under_a_free_name() {
        let lab = Lab::new();
        let (from, to) = taken_home(&lab);
        let outcome = lab.undo_choosing(moved(&from, &to), Resolve::KeepBoth);
        let landed = lab.path("old/a (2).txt");
        assert_eq!(
            outcome,
            Outcome::Undone(Undone::MovedBack { to: landed.clone() })
        );
        assert_eq!(read(&landed), "moved");
        assert_eq!(read(&from), "occupant");
        assert!(!to.exists());
    }

    #[test]
    fn moved_home_taken_with_a_choice_that_keeps_the_occupant_leaves_kept() {
        for choice in [Resolve::Keep, Resolve::Replace, Resolve::Merge] {
            let lab = Lab::new();
            let (from, to) = taken_home(&lab);
            let outcome = lab.undo_choosing(moved(&from, &to), choice);
            assert_eq!(
                outcome,
                Outcome::Leaves(Reason::Kept(from.clone())),
                "{choice:?}"
            );
            assert_eq!(read(&from), "occupant", "{choice:?}");
            assert_eq!(read(&to), "moved", "{choice:?}");
        }
    }

    #[test]
    fn moved_home_taken_after_the_ask_with_no_choice_leaves_taken() {
        let lab = Lab::new();
        let (from, to) = taken_home(&lab);
        let outcome = lab.undo(moved(&from, &to));
        assert_eq!(outcome, Outcome::Leaves(Reason::Taken(from.clone())));
        assert_eq!(read(&from), "occupant");
        assert_eq!(read(&to), "moved");
    }

    #[test]
    fn a_case_only_rename_goes_back() {
        let lab = Lab::new();
        let from = lab.path("dir/name.txt");
        let to = lab.path("dir/Name.txt");
        write(&to, "text");
        let replies = lab.run(vec![moved(&from, &to)], None);
        let outcome = only(replies);
        assert_eq!(
            outcome,
            Outcome::Undone(Undone::MovedBack { to: from.clone() })
        );
        assert_eq!(names(&lab.path("dir")), vec!["name.txt".to_string()]);
        assert_eq!(read(&from), "text");
    }

    #[test]
    fn an_unchanged_copy_of_an_unchanged_original_goes_to_the_bin() {
        let lab = Lab::new();
        let (from, to) = copy_pair(&lab);
        let outcome = lab.undo(copied(&lab, &from, &to));
        let Outcome::Undone(Undone::Binned(Some(held))) = &outcome else {
            panic!("{outcome:?}");
        };
        assert!(!to.exists());
        assert_eq!(read(&from), "contents");
        assert_eq!(read(&held.item), "contents");
        assert_eq!(lab.binned().len(), 1);
    }

    #[test]
    fn a_copied_folder_that_is_unchanged_goes_to_the_bin_whole() {
        let lab = Lab::new();
        let (from, to) = copy_trees(&lab);
        let outcome = lab.undo(copied(&lab, &from, &to));
        let Outcome::Undone(Undone::Binned(Some(held))) = &outcome else {
            panic!("{outcome:?}");
        };
        assert!(!to.exists());
        assert_eq!(read(&held.item.join("sub/b.txt")), "inner");
        assert_eq!(read(&from.join("sub/b.txt")), "inner");
    }

    #[test]
    fn a_copy_is_left_alone_when_it_or_its_original_changed() {
        let cases: [(&str, Mutate, Expect); 4] = [
            (
                "copy edited",
                |_: &Path, to: &Path| edit(to, "edited copy"),
                |_: &Path, to: &Path| Outcome::Leaves(Reason::Changed(to.to_path_buf())),
            ),
            (
                "copy replaced",
                |_: &Path, to: &Path| replace(to, "another file"),
                |_: &Path, to: &Path| Outcome::Leaves(Reason::Changed(to.to_path_buf())),
            ),
            (
                "original deleted",
                |from: &Path, _: &Path| fs::remove_file(from).unwrap(),
                |from: &Path, _: &Path| Outcome::Leaves(Reason::OriginalGone(from.to_path_buf())),
            ),
            (
                "original edited",
                |from: &Path, _: &Path| edit(from, "edited original"),
                |from: &Path, _: &Path| {
                    Outcome::Leaves(Reason::OriginalChanged(from.to_path_buf()))
                },
            ),
        ];
        for (name, mutate, expect) in cases {
            let lab = Lab::new();
            let (from, to) = copy_pair(&lab);
            let change = copied(&lab, &from, &to);
            mutate(&from, &to);
            let copy_now = read(&to);
            let outcome = lab.undo(change);
            assert_eq!(outcome, expect(&from, &to), "{name}");
            assert_eq!(read(&to), copy_now, "{name}");
            assert!(lab.binned().is_empty(), "{name}");
        }
    }

    #[test]
    fn a_copied_folder_with_one_inner_file_edited_leaves_changed() {
        let lab = Lab::new();
        let (from, to) = copy_trees(&lab);
        let change = copied(&lab, &from, &to);
        edit(&to.join("sub/b.txt"), "inner but longer");
        let outcome = lab.undo(change);
        assert_eq!(outcome, Outcome::Leaves(Reason::Changed(to.clone())));
        assert_eq!(read(&to.join("sub/b.txt")), "inner but longer");
        assert!(lab.binned().is_empty());
    }

    #[test]
    fn a_copied_folder_with_an_inner_file_added_leaves_changed() {
        let lab = Lab::new();
        let (from, to) = copy_trees(&lab);
        let change = copied(&lab, &from, &to);
        write(&to.join("sub/extra.txt"), "added");
        let outcome = lab.undo(change);
        assert_eq!(outcome, Outcome::Leaves(Reason::Changed(to.clone())));
        assert!(to.join("sub/extra.txt").exists());
    }

    #[test]
    fn a_crossed_move_is_copied_back_and_the_far_side_trashed() {
        let lab = Lab::new();
        let from = lab.path("home/a.txt");
        let to = lab.path("far/a.txt");
        fs::create_dir_all(from.parent().unwrap()).unwrap();
        write(&to, "far copy");
        let outcome = lab.undo(crossed(&lab, &from, &to, Source::Removed));
        let Outcome::Undone(Undone::CopiedBack {
            to: back,
            far: Some(held),
            kept: None,
        }) = &outcome
        else {
            panic!("{outcome:?}");
        };
        assert_eq!(back, &from);
        assert_eq!(read(&from), "far copy");
        assert!(!to.exists());
        assert_eq!(read(&held.item), "far copy");
    }

    #[test]
    fn a_crossed_move_with_a_taken_home_keeps_both_when_told_to() {
        let lab = Lab::new();
        let from = lab.path("home/a.txt");
        let to = lab.path("far/a.txt");
        write(&from, "occupant");
        write(&to, "far copy");
        let change = crossed(&lab, &from, &to, Source::Removed);
        let outcome = lab.undo_choosing(change, Resolve::KeepBoth);
        let landed = lab.path("home/a (2).txt");
        let Outcome::Undone(Undone::CopiedBack {
            to: back,
            far: Some(_),
            kept: None,
        }) = &outcome
        else {
            panic!("{outcome:?}");
        };
        assert_eq!(back, &landed);
        assert_eq!(read(&landed), "far copy");
        assert_eq!(read(&from), "occupant");
        assert!(!to.exists());
    }

    #[test]
    fn a_crossed_move_whose_source_was_not_fully_removed_leaves_partly() {
        for source in [Source::Partly, Source::Removing] {
            let lab = Lab::new();
            let from = lab.path("home/a.txt");
            let to = lab.path("far/a.txt");
            fs::create_dir_all(from.parent().unwrap()).unwrap();
            write(&to, "far copy");
            let outcome = lab.undo(crossed(&lab, &from, &to, source));
            assert_eq!(
                outcome,
                Outcome::Leaves(Reason::Partly(to.clone())),
                "{source:?}"
            );
            assert_eq!(read(&to), "far copy", "{source:?}");
            assert!(!from.exists(), "{source:?}");
        }
    }

    #[test]
    fn a_crossed_move_whose_far_copy_was_edited_leaves_changed() {
        let lab = Lab::new();
        let from = lab.path("home/a.txt");
        let to = lab.path("far/a.txt");
        fs::create_dir_all(from.parent().unwrap()).unwrap();
        write(&to, "far copy");
        let change = crossed(&lab, &from, &to, Source::Removed);
        edit(&to, "edited far copy");
        let outcome = lab.undo(change);
        assert_eq!(outcome, Outcome::Leaves(Reason::Changed(to.clone())));
        assert_eq!(read(&to), "edited far copy");
        assert!(!from.exists());
    }

    #[test]
    fn replaced_matched_and_trashed_items_come_back_exactly() {
        for (name, restore) in restores() {
            let lab = Lab::new();
            let home = lab.path("docs/a.txt");
            write(&home, "old");
            let held = lab.held(&home);
            assert!(!home.exists(), "{name}");
            let outcome = lab.undo(restore(home.clone(), Some(held.clone())));
            assert_eq!(
                outcome,
                Outcome::Undone(Undone::Restored { to: home.clone() }),
                "{name}"
            );
            assert_eq!(read(&home), "old", "{name}");
            assert!(!held.item.exists(), "{name}");
        }
    }

    #[test]
    fn replaced_matched_and_trashed_items_without_a_hold_cannot_be_undone() {
        for (name, restore) in restores() {
            let lab = Lab::new();
            let home = lab.path("docs/a.txt");
            let outcome = lab.undo(restore(home.clone(), None));
            assert_eq!(
                outcome,
                Outcome::Leaves(Reason::CannotUndo(home.clone())),
                "{name}"
            );
            assert!(!home.exists(), "{name}");
        }
    }

    #[test]
    fn an_item_gone_from_the_bin_leaves_not_in_trash() {
        for (name, restore) in restores() {
            let lab = Lab::new();
            let home = lab.path("docs/a.txt");
            write(&home, "old");
            let held = lab.held(&home);
            fs::remove_file(&held.item).unwrap();
            let outcome = lab.undo(restore(home.clone(), Some(held)));
            assert_eq!(
                outcome,
                Outcome::Leaves(Reason::NotInTrash(home.clone())),
                "{name}"
            );
            assert!(!home.exists(), "{name}");
        }
    }

    #[test]
    fn a_swapped_bin_item_is_never_restored() {
        for (name, restore) in restores() {
            let lab = Lab::new();
            let home = lab.path("docs/a.txt");
            write(&home, "old");
            let held = lab.held(&home);
            replace(&held.item, "swapped");
            let outcome = lab.undo(restore(home.clone(), Some(held.clone())));
            assert_eq!(
                outcome,
                Outcome::Leaves(Reason::NotInTrash(home.clone())),
                "{name}"
            );
            assert!(!home.exists(), "{name}");
            assert_eq!(read(&held.item), "swapped", "{name}");
        }
    }

    #[test]
    fn the_exact_item_is_restored_among_same_named_ones_in_the_bin() {
        let lab = Lab::new();
        let home = lab.path("docs/a.txt");
        write(&home, "first");
        let first = lab.held(&home);
        write(&home, "second");
        let second = lab.held(&home);
        assert_ne!(first.item, second.item);
        let outcome = lab.undo(Change::Trashed {
            from: home.clone(),
            held: Some(first),
        });
        assert_eq!(
            outcome,
            Outcome::Undone(Undone::Restored { to: home.clone() })
        );
        assert_eq!(read(&home), "first");
        assert_eq!(read(&second.item), "second");
    }

    #[test]
    fn another_item_with_that_name_in_the_bin_is_never_restored_in_its_place() {
        let lab = Lab::new();
        let home = lab.path("docs/a.txt");
        write(&home, "first");
        let first = lab.held(&home);
        write(&home, "second");
        let second = lab.held(&home);
        fs::remove_file(&first.item).unwrap();
        let outcome = lab.undo(Change::Trashed {
            from: home.clone(),
            held: Some(first),
        });
        assert_eq!(outcome, Outcome::Leaves(Reason::NotInTrash(home.clone())));
        assert!(!home.exists());
        assert_eq!(read(&second.item), "second");
    }

    #[test]
    fn a_trashed_item_whose_place_is_taken_asks_first() {
        let lab = Lab::new();
        let home = lab.path("docs/a.txt");
        write(&home, "old");
        let held = lab.held(&home);
        write(&home, "new");
        let change = Change::Trashed {
            from: home.clone(),
            held: Some(held.clone()),
        };
        let replies = lab.run(vec![change], None);
        let clashes = asked(&replies);
        assert_eq!(clashes[0].0, 0);
        assert_eq!(clashes[0].1.existing, home);
        assert_eq!(clashes[0].1.source, held.item);
        assert_eq!(read(&home), "new");
        assert_eq!(read(&held.item), "old");
    }

    #[test]
    fn a_trashed_item_whose_place_is_taken_after_the_ask_leaves_taken() {
        let lab = Lab::new();
        let home = lab.path("docs/a.txt");
        write(&home, "old");
        let held = lab.held(&home);
        write(&home, "new");
        let outcome = lab.undo(Change::Trashed {
            from: home.clone(),
            held: Some(held.clone()),
        });
        assert_eq!(outcome, Outcome::Leaves(Reason::Taken(home.clone())));
        assert_eq!(read(&home), "new");
        assert_eq!(read(&held.item), "old");
    }

    #[test]
    fn a_trashed_item_whose_place_is_taken_lands_under_a_free_name_when_told_to() {
        let lab = Lab::new();
        let home = lab.path("docs/a.txt");
        write(&home, "old");
        let held = lab.held(&home);
        write(&home, "new");
        let outcome = lab.undo_choosing(
            Change::Trashed {
                from: home.clone(),
                held: Some(held),
            },
            Resolve::KeepBoth,
        );
        let landed = lab.path("docs/a (2).txt");
        assert_eq!(
            outcome,
            Outcome::Undone(Undone::Restored { to: landed.clone() })
        );
        assert_eq!(read(&landed), "old");
        assert_eq!(read(&home), "new");
    }

    #[test]
    fn a_trashed_item_makes_its_folder_again_when_that_is_gone() {
        let lab = Lab::new();
        let home = lab.path("docs/deep/a.txt");
        write(&home, "old");
        let held = lab.held(&home);
        fs::remove_dir_all(lab.path("docs")).unwrap();
        let outcome = lab.undo(Change::Trashed {
            from: home.clone(),
            held: Some(held),
        });
        assert_eq!(
            outcome,
            Outcome::Undone(Undone::Restored { to: home.clone() })
        );
        assert_eq!(read(&home), "old");
    }

    #[test]
    fn an_emptied_folder_is_made_again() {
        let lab = Lab::new();
        let folder = lab.path("docs/empty");
        fs::create_dir_all(&folder).unwrap();
        let change = emptied(&folder);
        fs::remove_dir(&folder).unwrap();
        let outcome = lab.undo(change);
        assert_eq!(outcome, Outcome::Undone(Undone::Remade { made: true }));
        assert!(folder.is_dir());
    }

    #[test]
    fn an_emptied_folder_that_exists_again_is_not_made_twice() {
        let lab = Lab::new();
        let folder = lab.path("docs/empty");
        fs::create_dir_all(&folder).unwrap();
        write(&folder.join("kept.txt"), "kept");
        let outcome = lab.undo(emptied(&folder));
        assert_eq!(outcome, Outcome::Undone(Undone::Remade { made: false }));
        assert_eq!(read(&folder.join("kept.txt")), "kept");
    }

    #[test]
    fn a_file_in_the_place_of_an_emptied_folder_leaves_taken() {
        let lab = Lab::new();
        let folder = lab.path("docs/empty");
        fs::create_dir_all(&folder).unwrap();
        let change = emptied(&folder);
        fs::remove_dir(&folder).unwrap();
        write(&folder, "a file now");
        let outcome = lab.undo(change);
        assert_eq!(outcome, Outcome::Leaves(Reason::Taken(folder.clone())));
        assert_eq!(read(&folder), "a file now");
    }

    #[test]
    fn a_new_file_nobody_touched_goes_to_the_bin() {
        let lab = Lab::new();
        let path = lab.path("docs/new.txt");
        write(&path, "");
        let outcome = lab.undo(made(&path));
        assert!(matches!(outcome, Outcome::Undone(Undone::Binned(Some(_)))));
        assert!(!path.exists());
        assert_eq!(lab.binned().len(), 1);
    }

    #[test]
    fn a_new_file_that_was_edited_stays() {
        let lab = Lab::new();
        let path = lab.path("docs/new.txt");
        write(&path, "");
        let change = made(&path);
        edit(&path, "typed in");
        let outcome = lab.undo(change);
        assert_eq!(outcome, Outcome::Leaves(Reason::Changed(path.clone())));
        assert_eq!(read(&path), "typed in");
        assert!(lab.binned().is_empty());
    }

    #[test]
    fn a_new_file_replaced_by_another_stays() {
        let lab = Lab::new();
        let path = lab.path("docs/new.txt");
        write(&path, "");
        let change = made(&path);
        replace(&path, "");
        let outcome = lab.undo(change);
        assert_eq!(outcome, Outcome::Leaves(Reason::Changed(path.clone())));
        assert!(path.exists());
    }

    #[test]
    fn a_new_empty_folder_is_removed() {
        let lab = Lab::new();
        let path = lab.path("docs/new folder");
        fs::create_dir_all(&path).unwrap();
        let outcome = lab.undo(made(&path));
        assert_eq!(outcome, Outcome::Undone(Undone::Removed));
        assert!(!path.exists());
    }

    #[test]
    fn a_new_folder_with_something_inside_stays() {
        let lab = Lab::new();
        let path = lab.path("docs/new folder");
        fs::create_dir_all(&path).unwrap();
        let change = made(&path);
        write(&path.join("inside.txt"), "inside");
        let outcome = lab.undo(change);
        assert_eq!(outcome, Outcome::Leaves(Reason::Changed(path.clone())));
        assert_eq!(read(&path.join("inside.txt")), "inside");
    }

    #[test]
    fn a_new_item_that_is_gone_leaves_gone() {
        let lab = Lab::new();
        let path = lab.path("docs/new.txt");
        write(&path, "");
        let change = made(&path);
        fs::remove_file(&path).unwrap();
        let outcome = lab.undo(change);
        assert_eq!(outcome, Outcome::Leaves(Reason::Gone(path.clone())));
    }

    #[test]
    fn an_erased_item_cannot_be_undone() {
        let lab = Lab::new();
        let path = lab.path("docs/erased.txt");
        let outcome = lab.undo(Change::Erased { path: path.clone() });
        assert_eq!(outcome, Outcome::Leaves(Reason::CannotUndo(path)));
        assert!(lab.binned().is_empty());
    }

    #[test]
    fn an_earlier_item_that_frees_the_place_spares_the_ask() {
        let lab = Lab::new();
        let place = lab.path("dst/doc.txt");
        let source = lab.path("src/doc.txt");
        write(&place, "old");
        let held = lab.held(&place);
        write(&source, "new");
        write(&place, "new");
        let changes = vec![
            Change::Replaced {
                at: place.clone(),
                held: Some(held),
            },
            copied(&lab, &source, &place),
        ];
        let outcomes = settled(lab.run(changes, None));
        assert_eq!(outcomes.len(), 2, "{outcomes:?}");
        assert!(matches!(
            outcomes[0],
            (1, Outcome::Undone(Undone::Binned(Some(_))))
        ));
        assert_eq!(
            outcomes[1],
            (0, Outcome::Undone(Undone::Restored { to: place.clone() }))
        );
        assert_eq!(read(&place), "old");
        assert_eq!(read(&source), "new");
    }

    #[test]
    fn a_new_item_of_another_kind_than_its_print_stays() {
        let lab = Lab::new();
        let path = lab.path("docs/new");
        write(&path, "");
        let change = made(&path);
        fs::remove_file(&path).unwrap();
        fs::create_dir_all(&path).unwrap();
        let outcome = lab.undo(change);
        assert_eq!(outcome, Outcome::Leaves(Reason::Changed(path.clone())));
        assert!(path.is_dir());
    }

    #[cfg(unix)]
    mod unix {
        use std::os::unix::fs::PermissionsExt;

        use super::*;
        use crate::ops::history::Settled;

        struct Writable(PathBuf);

        impl Drop for Writable {
            fn drop(&mut self) {
                let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
            }
        }

        #[test]
        fn a_first_failure_stays_and_a_second_one_leaves() {
            let lab = Lab::new();
            let to = lab.path("new/a.txt");
            let shelf = lab.path("shelf");
            let home = shelf.join("a.txt");
            write(&to, "moved");
            fs::create_dir_all(&shelf).unwrap();
            let change = moved(&home, &to);
            fs::set_permissions(&shelf, fs::Permissions::from_mode(0o555)).unwrap();
            let _writable = Writable(shelf.clone());
            if File::create(shelf.join("probe")).is_ok() {
                return;
            }

            let mut history = History::default();
            let step = history.open(Action::Trash(Vec::new()));
            history.record(step, change);

            let first = settled(lab.run_plan(history.plan(step).unwrap(), Some(Choices::new())));
            assert!(
                matches!(
                    first.as_slice(),
                    [(0, Outcome::Stays(Reason::Failed(path, _)))] if path == &to
                ),
                "{first:?}"
            );
            history.settle(&Settled {
                step,
                outcomes: first,
            });
            let retry = history.plan(step).unwrap();
            assert!(retry.items[0].retry);

            let second = settled(lab.run_plan(retry, Some(Choices::new())));
            assert!(
                matches!(
                    second.as_slice(),
                    [(0, Outcome::Leaves(Reason::Failed(path, _)))] if path == &to
                ),
                "{second:?}"
            );
            history.settle(&Settled {
                step,
                outcomes: second,
            });
            assert!(history.step(step).is_none());
            assert_eq!(read(&to), "moved");
            assert!(!home.exists());
        }

        #[test]
        fn a_home_on_another_drive_than_its_print_stays_unplugged() {
            let Ok(far) = tempfile::tempdir_in("/dev/shm") else {
                return;
            };
            let lab = Lab::new();
            if print(far.path()).device() == print(lab.work.path()).device() {
                return;
            }
            let to = lab.path("new/a.txt");
            let home = far.path().join("a.txt");
            write(&to, "moved");
            let outcome = lab.undo(moved(&home, &to));
            assert_eq!(outcome, Outcome::Stays(Reason::Unplugged(home.clone())));
            assert_eq!(read(&to), "moved");
            assert!(!home.exists());
        }
    }
}
