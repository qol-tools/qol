use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::walk::{drain, files, listing, outcomes_of, snapshot, texts, write_at, Rig, Undid};
use super::{start, Action, Change, Direction, Outcome, Reason, Settled, Top, Undone};
use crate::ops::print::{Print, TreePrint};
use crate::ops::{Bin, BinError, Failure, FolderBin, Held, Resolve, Verb};

const WAIT: Duration = Duration::from_secs(30);

type Hook = Box<dyn FnMut(&Path) -> Result<(), BinError> + Send>;

struct HookBin {
    inner: FolderBin,
    before_trash: Mutex<Option<Hook>>,
    before_restore: Mutex<Option<Hook>>,
}

impl HookBin {
    fn new(root: PathBuf) -> Self {
        Self {
            inner: FolderBin::new(root),
            before_trash: Mutex::new(None),
            before_restore: Mutex::new(None),
        }
    }

    fn on_trash(&self, hook: impl FnMut(&Path) -> Result<(), BinError> + Send + 'static) {
        *locked(&self.before_trash) = Some(Box::new(hook));
    }

    fn on_restore(&self, hook: impl FnMut(&Path) -> Result<(), BinError> + Send + 'static) {
        *locked(&self.before_restore) = Some(Box::new(hook));
    }

    fn disarm(&self) {
        *locked(&self.before_trash) = None;
        *locked(&self.before_restore) = None;
    }
}

impl Bin for HookBin {
    fn trash(&self, path: &Path) -> Result<Option<Held>, BinError> {
        fire(&self.before_trash, path)?;
        self.inner.trash(path)
    }

    fn restore(&self, held: &Held, to: &Path) -> Result<(), BinError> {
        fire(&self.before_restore, to)?;
        self.inner.restore(held, to)
    }
}

fn locked(slot: &Mutex<Option<Hook>>) -> MutexGuard<'_, Option<Hook>> {
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn fire(slot: &Mutex<Option<Hook>>, path: &Path) -> Result<(), BinError> {
    match locked(slot).as_mut() {
        Some(hook) => hook(path),
        None => Ok(()),
    }
}

fn hooked() -> (Rig, Arc<HookBin>) {
    let mut rig = Rig::new();
    let hooks = Arc::new(HookBin::new(rig.bin_root.clone()));
    let bin: Arc<dyn Bin> = hooks.clone();
    rig.tools.bin = bin;
    (rig, hooks)
}

fn ran(undid: Undid) -> Vec<(usize, Outcome)> {
    match undid {
        Undid::Ran { outcomes, .. } => outcomes,
        Undid::Nothing | Undid::Dropped(_) => panic!("the undo did not run"),
    }
}

fn is_binned(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::Undone(Undone::Binned(Some(_))))
}

fn refusing() -> impl FnMut(&Path) -> Result<(), BinError> + Send + 'static {
    |_| Err(BinError::Failed("the trash is full".into()))
}

#[test]
fn a_refused_trash_leaves_the_file_a_transfer_would_replace() {
    for verb in [Verb::Copy, Verb::Move] {
        let (mut rig, hooks) = hooked();
        let from = rig.work.join("from");
        let into = rig.work.join("into");
        write_at(&from.join("a.txt"), "incoming", 20);
        write_at(&into.join("a.txt"), "existing", 10);
        hooks.on_trash(refusing());
        let before = rig.held();

        let transferred = rig.transfer(verb, &[from.join("a.txt")], &into, |_| Resolve::Replace);

        let done = transferred.done().expect("the job finished");
        assert_eq!(
            done.failures,
            vec![Failure {
                path: into.join("a.txt"),
                reason: "the trash is full".into(),
            }],
            "{verb:?}"
        );
        assert!(done.placed.is_empty(), "{verb:?}: {:?}", done.placed);
        assert_eq!(
            files(&rig.work),
            listing(&[("from/a.txt", "incoming"), ("into/a.txt", "existing")]),
            "{verb:?}"
        );
        assert!(texts(&rig.bin_root).is_empty(), "{verb:?}");
        let step = rig.history.step(transferred.step).unwrap();
        assert!(step.items.is_empty(), "{verb:?}: {:?}", step.items);
        rig.keeps(&before, "a refused trash");
        assert!(
            matches!(rig.undo(|_, _| Resolve::Keep), Undid::Dropped(_)),
            "{verb:?}"
        );
        assert_eq!(
            files(&rig.work),
            listing(&[("from/a.txt", "incoming"), ("into/a.txt", "existing")]),
            "{verb:?}"
        );
    }
}

#[test]
fn a_refused_trash_leaves_the_copy_an_undo_would_bin() {
    let (mut rig, hooks) = hooked();
    let from = rig.work.join("from");
    let into = rig.work.join("into");
    write_at(&from.join("a.txt"), "original", 10);
    fs::create_dir_all(&into).unwrap();
    let transferred = rig.transfer(Verb::Copy, &[from.join("a.txt")], &into, |_| Resolve::Keep);
    let copy = into.join("a.txt");
    let step = transferred.step;
    let before = rig.held();
    let expected = listing(&[("from/a.txt", "original"), ("into/a.txt", "original")]);
    hooks.on_trash(refusing());

    let first = ran(rig.undo(|_, _| Resolve::Keep));

    assert_eq!(
        first,
        vec![(
            0,
            Outcome::Stays(Reason::Failed(copy.clone(), "the trash is full".into()))
        )]
    );
    assert_eq!(files(&rig.work), expected);
    assert!(texts(&rig.bin_root).is_empty());
    let kept = rig.history.step(step).expect("the step stays for a retry");
    assert!(kept.items[0].tried);
    assert!(matches!(rig.history.top(), Top::Undo(_)));

    let second = ran(rig.undo(|_, _| Resolve::Keep));

    assert_eq!(
        second,
        vec![(
            0,
            Outcome::Leaves(Reason::Failed(copy, "the trash is full".into()))
        )]
    );
    assert_eq!(files(&rig.work), expected);
    assert!(texts(&rig.bin_root).is_empty());
    assert!(rig.history.step(step).is_none());
    assert!(matches!(rig.history.top(), Top::Nothing));
    rig.keeps(&before, "a refused trash during an undo");
}

#[test]
fn a_move_that_dies_halfway_keeps_its_first_item_undoable() {
    let (mut rig, hooks) = hooked();
    let from = rig.work.join("from");
    let into = rig.work.join("into");
    write_at(&from.join("a.txt"), "incoming a", 30);
    write_at(&from.join("b.txt"), "incoming b", 40);
    write_at(&into.join("b.txt"), "existing b", 10);
    hooks.on_trash(|path| panic!("the trash died on {}", path.display()));
    let before = rig.held();

    let transferred = rig.transfer(
        Verb::Move,
        &[from.join("a.txt"), from.join("b.txt")],
        &into,
        |_| Resolve::Replace,
    );
    hooks.disarm();

    assert!(
        transferred.done().is_none(),
        "the job thread should have died"
    );
    assert_eq!(
        files(&rig.work),
        listing(&[
            ("from/b.txt", "incoming b"),
            ("into/a.txt", "incoming a"),
            ("into/b.txt", "existing b"),
        ])
    );
    assert!(texts(&rig.bin_root).is_empty());
    let step = rig.history.step(transferred.step).unwrap();
    assert_eq!(step.items.len(), 1, "{:?}", step.items);
    assert!(
        matches!(
            &step.items[0].change,
            Change::Moved { from: moved_from, to, .. }
                if *moved_from == from.join("a.txt") && *to == into.join("a.txt")
        ),
        "{:?}",
        step.items
    );

    let outcomes = ran(rig.undo(|_, _| Resolve::Keep));

    let home = from.join("a.txt");
    assert_eq!(
        outcomes,
        vec![(
            0,
            Outcome::Undone(Undone::MovedBack {
                at: Print::read(&home).unwrap(),
                to: home,
            })
        )]
    );
    assert_eq!(
        files(&rig.work),
        listing(&[
            ("from/a.txt", "incoming a"),
            ("from/b.txt", "incoming b"),
            ("into/b.txt", "existing b"),
        ])
    );
    assert!(matches!(rig.history.top(), Top::Nothing));
    rig.keeps(&before, "a move that died halfway");
}

#[test]
fn a_copy_written_at_the_last_moment_is_binned_with_its_new_contents() {
    let (mut rig, hooks) = hooked();
    let from = rig.work.join("from");
    let into = rig.work.join("into");
    write_at(&from.join("a.txt"), "original", 10);
    fs::create_dir_all(&into).unwrap();
    rig.transfer(Verb::Copy, &[from.join("a.txt")], &into, |_| Resolve::Keep);
    let copy = into.join("a.txt");
    let before = rig.held();
    hooks.on_trash(move |path| {
        if path == copy.as_path() {
            fs::write(path, "edited at the last moment").unwrap();
        }
        Ok(())
    });

    let outcomes = ran(rig.undo(|_, _| Resolve::Keep));

    assert_eq!(outcomes.len(), 1, "{outcomes:?}");
    assert!(is_binned(&outcomes[0].1), "{outcomes:?}");
    assert_eq!(files(&rig.work), listing(&[("from/a.txt", "original")]));
    assert_eq!(texts(&rig.bin_root), vec!["edited at the last moment"]);
    rig.keeps(&before, "an edit at the last moment");
}

#[test]
fn a_new_file_written_at_the_last_moment_is_binned_with_its_new_contents() {
    let (mut rig, hooks) = hooked();
    let work = rig.work.clone();
    let made = rig.make_file(&work, "new.txt").unwrap();
    let before = rig.held();
    hooks.on_trash(move |path| {
        if path == made.as_path() {
            fs::write(path, "typed at the last moment").unwrap();
        }
        Ok(())
    });

    let outcomes = ran(rig.undo(|_, _| Resolve::Keep));

    assert_eq!(outcomes.len(), 1, "{outcomes:?}");
    assert!(is_binned(&outcomes[0].1), "{outcomes:?}");
    assert!(files(&rig.work).is_empty());
    assert_eq!(texts(&rig.bin_root), vec!["typed at the last moment"]);
    rig.keeps(&before, "an edit at the last moment");
}

#[test]
fn a_name_taken_in_the_middle_of_an_undo_is_not_replaced() {
    let (mut rig, hooks) = hooked();
    let from = rig.work.join("from");
    let into = rig.work.join("into");
    write_at(&from.join("m.txt"), "moved", 10);
    write_at(&from.join("o.txt"), "original", 20);
    fs::create_dir_all(&into).unwrap();
    let key = rig.tools.key.clone();
    let moved_from = from.join("m.txt");
    let moved_to = into.join("m.txt");
    let original_path = from.join("o.txt");
    let copy = into.join("o.txt");

    let step = rig.history.open(Action::Transfer {
        verb: Verb::Copy,
        sources: vec![moved_from.clone(), original_path.clone()],
        into: into.clone(),
    });
    let at = Print::read(&moved_from).unwrap();
    fs::rename(&moved_from, &moved_to).unwrap();
    rig.history.record(
        step,
        Change::Moved {
            from: moved_from.clone(),
            to: moved_to.clone(),
            at,
        },
    );
    let original = TreePrint::read(&original_path, &key).unwrap();
    fs::copy(&original_path, &copy).unwrap();
    let tree = TreePrint::read(&copy, &key).unwrap();
    rig.history.record(
        step,
        Change::Copied {
            from: original_path.clone(),
            to: copy.clone(),
            tree,
            original,
        },
    );
    rig.history.close(step);
    let before = rig.held();
    let home = moved_from.clone();
    let target = copy.clone();
    hooks.on_trash(move |path| {
        if path == target.as_path() {
            fs::write(&home, "taken in the middle").unwrap();
        }
        Ok(())
    });

    let outcomes = ran(rig.undo(|_, _| Resolve::Keep));

    assert_eq!(outcomes.len(), 2, "{outcomes:?}");
    assert_eq!(outcomes[0].0, 1);
    assert!(is_binned(&outcomes[0].1), "{outcomes:?}");
    assert_eq!(
        outcomes[1],
        (0, Outcome::Leaves(Reason::Taken(moved_from.clone())))
    );
    assert_eq!(
        files(&rig.work),
        listing(&[
            ("from/m.txt", "taken in the middle"),
            ("from/o.txt", "original"),
            ("into/m.txt", "moved"),
        ])
    );
    assert_eq!(texts(&rig.bin_root), vec!["original"]);
    assert!(matches!(rig.history.top(), Top::Nothing));
    rig.keeps(&before, "a name taken in the middle of an undo");
}

#[test]
fn an_emptied_bin_leaves_a_trash_step_with_nothing_to_restore() {
    let mut rig = Rig::new();
    let kept = rig.work.join("kept.txt");
    let gone = rig.work.join("gone.txt");
    write_at(&kept, "kept", 10);
    write_at(&gone, "binned", 20);
    let step = rig.trash(std::slice::from_ref(&gone));
    assert!(rig.bin_root.exists());
    fs::remove_dir_all(&rig.bin_root).unwrap();
    let work_before = snapshot(&rig.work);

    let outcomes = ran(rig.undo(|_, _| Resolve::Keep));

    assert_eq!(
        outcomes,
        vec![(0, Outcome::Leaves(Reason::NotInTrash(gone)))]
    );
    assert_eq!(snapshot(&rig.work), work_before);
    assert_eq!(files(&rig.work), listing(&[("kept.txt", "kept")]));
    assert!(texts(&rig.bin_root).is_empty());
    assert!(rig.history.step(step).is_none());
    assert!(matches!(rig.history.top(), Top::Nothing));
}

#[test]
fn a_refused_restore_keeps_the_item_in_the_bin() {
    let (mut rig, hooks) = hooked();
    let gone = rig.work.join("gone.txt");
    write_at(&gone, "binned", 20);
    let step = rig.trash(std::slice::from_ref(&gone));
    let before = rig.held();
    hooks.on_restore(|_| Err(BinError::Failed("the trash is jammed".into())));

    let first = ran(rig.undo(|_, _| Resolve::Keep));

    assert_eq!(
        first,
        vec![(
            0,
            Outcome::Stays(Reason::Failed(gone.clone(), "the trash is jammed".into()))
        )]
    );
    assert!(files(&rig.work).is_empty());
    assert_eq!(texts(&rig.bin_root), vec!["binned"]);
    assert!(rig.history.step(step).is_some());

    let second = ran(rig.undo(|_, _| Resolve::Keep));

    assert_eq!(
        second,
        vec![(
            0,
            Outcome::Leaves(Reason::Failed(gone, "the trash is jammed".into()))
        )]
    );
    assert!(files(&rig.work).is_empty());
    assert_eq!(texts(&rig.bin_root), vec!["binned"]);
    assert!(rig.history.step(step).is_none());
    rig.keeps(&before, "a refused restore");
}

#[test]
fn a_stopped_undo_keeps_its_unfinished_items_for_the_next_undo() {
    let (mut rig, hooks) = hooked();
    let from = rig.work.join("from");
    let into = rig.work.join("into");
    let sources: Vec<PathBuf> = ["a", "b", "c"]
        .iter()
        .enumerate()
        .map(|(number, name)| {
            let path = from.join(format!("{name}.txt"));
            write_at(&path, &format!("text {name}"), 10 * (number as u64 + 1));
            path
        })
        .collect();
    fs::create_dir_all(&into).unwrap();
    rig.transfer(Verb::Copy, &sources, &into, |_| Resolve::Keep);
    let before = rig.held();
    let (entered_tx, entered_rx) = channel::<()>();
    let (release_tx, release_rx) = channel::<()>();
    hooks.on_trash(move |_| {
        let _ = entered_tx.send(());
        let _ = release_rx.recv_timeout(WAIT);
        Ok(())
    });
    let id = rig.top_undo();
    let plan = rig.history.plan(id, Direction::Undo).unwrap();
    let undoing = start(plan, Direction::Undo, None, rig.tools.clone(), None, || {});
    entered_rx.recv_timeout(WAIT).unwrap();
    undoing.stop.store(true, Ordering::SeqCst);
    release_tx.send(()).unwrap();

    let outcomes = outcomes_of(drain(&undoing.result));
    rig.history.settle(&Settled {
        step: id,
        outcomes: outcomes.clone(),
    });

    assert_eq!(outcomes.len(), 3, "{outcomes:?}");
    assert_eq!(outcomes[0].0, 2);
    assert!(is_binned(&outcomes[0].1), "{outcomes:?}");
    assert_eq!(
        &outcomes[1..],
        &[
            (1, Outcome::Stays(Reason::Stopped)),
            (0, Outcome::Stays(Reason::Stopped)),
        ]
    );
    let step = rig.history.step(id).expect("the step is kept");
    assert_eq!(step.waiting(), 2);
    assert!(step.items.iter().all(|item| !item.tried));
    assert_eq!(rig.top_undo(), id);
    assert_eq!(texts(&rig.bin_root), vec!["text c"]);

    hooks.disarm();
    let rest = ran(rig.undo(|_, _| Resolve::Keep));

    assert_eq!(rest.len(), 2, "{rest:?}");
    assert_eq!(rest[0].0, 1);
    assert_eq!(rest[1].0, 0);
    assert!(
        rest.iter().all(|(_, outcome)| is_binned(outcome)),
        "{rest:?}"
    );
    assert_eq!(
        files(&rig.work),
        listing(&[
            ("from/a.txt", "text a"),
            ("from/b.txt", "text b"),
            ("from/c.txt", "text c"),
        ])
    );
    assert_eq!(texts(&rig.bin_root), vec!["text a", "text b", "text c"]);
    assert!(matches!(rig.history.top(), Top::Nothing));
    rig.keeps(&before, "a stopped undo");
}

fn is_redone(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::Redone(_))
}

#[test]
fn a_refused_trash_leaves_the_file_a_redo_would_bin() {
    let (mut rig, hooks) = hooked();
    let gone = rig.work.join("gone.txt");
    write_at(&gone, "binned", 20);
    let step = rig.trash(std::slice::from_ref(&gone));
    ran(rig.undo(|_, _| Resolve::Keep));
    let before = rig.held();
    hooks.on_trash(refusing());

    let first = ran(rig.redo(|_, _| Resolve::Keep));

    assert_eq!(
        first,
        vec![(
            0,
            Outcome::Stays(Reason::Failed(gone.clone(), "the trash is full".into()))
        )]
    );
    assert_eq!(files(&rig.work), listing(&[("gone.txt", "binned")]));
    assert!(texts(&rig.bin_root).is_empty());
    assert_eq!(rig.history.redo_top().map(|found| found.id), Some(step));

    let second = ran(rig.redo(|_, _| Resolve::Keep));

    assert_eq!(
        second,
        vec![(
            0,
            Outcome::Leaves(Reason::Failed(gone, "the trash is full".into()))
        )]
    );
    assert_eq!(files(&rig.work), listing(&[("gone.txt", "binned")]));
    assert!(texts(&rig.bin_root).is_empty());
    assert!(rig.history.step(step).is_none());
    assert!(rig.history.redo_top().is_none());
    rig.keeps(&before, "a refused trash during a redo");
}

#[test]
fn a_redo_leaves_a_trashed_file_that_was_edited_since_the_undo() {
    let mut rig = Rig::new();
    let gone = rig.work.join("gone.txt");
    write_at(&gone, "binned", 20);
    let step = rig.trash(std::slice::from_ref(&gone));
    ran(rig.undo(|_, _| Resolve::Keep));
    write_at(&gone, "edited since the undo", 77);
    let before = rig.held();

    let outcomes = ran(rig.redo(|_, _| Resolve::Keep));

    assert_eq!(
        outcomes,
        vec![(0, Outcome::Leaves(Reason::Changed(gone.clone())))]
    );
    assert_eq!(
        files(&rig.work),
        listing(&[("gone.txt", "edited since the undo")])
    );
    assert!(texts(&rig.bin_root).is_empty());
    assert!(rig.history.step(step).is_none());
    rig.keeps(&before, "a redo of an edited file");
}

#[test]
fn a_redo_leaves_a_moved_file_that_was_swapped_since_the_undo() {
    let mut rig = Rig::new();
    let path = rig.work.join("a.txt");
    write_at(&path, "text", 20);
    let renamed = rig.rename(&path, "b.txt").unwrap();
    let step = rig.top_undo();
    ran(rig.undo(|_, _| Resolve::Keep));
    let swap = rig.work.join(".swap");
    write_at(&swap, "another file", 99);
    fs::rename(&swap, &path).unwrap();
    let before = rig.held();

    let outcomes = ran(rig.redo(|_, _| Resolve::Keep));

    assert_eq!(
        outcomes,
        vec![(0, Outcome::Leaves(Reason::Changed(path.clone())))]
    );
    assert_eq!(files(&rig.work), listing(&[("a.txt", "another file")]));
    assert!(!renamed.exists());
    assert!(rig.history.step(step).is_none());
    rig.keeps(&before, "a redo of a swapped file");
}

#[test]
fn a_name_taken_in_the_middle_of_a_redo_is_not_replaced() {
    let (mut rig, hooks) = hooked();
    let from = rig.work.join("from");
    let into = rig.work.join("into");
    write_at(&from.join("a.txt"), "text a", 10);
    write_at(&from.join("b.txt"), "text b", 20);
    fs::create_dir_all(&into).unwrap();
    let step = rig
        .transfer(
            Verb::Copy,
            &[from.join("a.txt"), from.join("b.txt")],
            &into,
            |_| Resolve::Keep,
        )
        .step;
    ran(rig.undo(|_, _| Resolve::Keep));
    let before = rig.held();
    let target = into.join("b.txt");
    hooks.on_restore(move |path| {
        if path == target.as_path() {
            fs::write(path, "taken in the middle").unwrap();
        }
        Ok(())
    });

    let outcomes = ran(rig.redo(|_, _| Resolve::Keep));

    assert_eq!(outcomes.len(), 2, "{outcomes:?}");
    assert_eq!(outcomes[0].0, 0);
    assert!(is_redone(&outcomes[0].1), "{outcomes:?}");
    assert_eq!(
        outcomes[1],
        (1, Outcome::Leaves(Reason::Taken(into.join("b.txt"))))
    );
    assert_eq!(
        files(&rig.work),
        listing(&[
            ("from/a.txt", "text a"),
            ("from/b.txt", "text b"),
            ("into/a.txt", "text a"),
            ("into/b.txt", "taken in the middle"),
        ])
    );
    assert_eq!(texts(&rig.bin_root), vec!["text b"]);
    let kept = rig
        .history
        .step(step)
        .expect("the step keeps its first item");
    assert_eq!(kept.items.len(), 1);
    assert!(rig.history.redo_top().is_none());
    assert!(matches!(rig.history.top(), Top::Undo(top) if top.id == step));
    rig.keeps(&before, "a name taken in the middle of a redo");
}

#[test]
fn an_emptied_bin_leaves_a_redo_with_nothing_to_restore() {
    let mut rig = Rig::new();
    let from = rig.work.join("from");
    let into = rig.work.join("into");
    write_at(&from.join("a.txt"), "original", 10);
    fs::create_dir_all(&into).unwrap();
    let step = rig
        .transfer(Verb::Copy, &[from.join("a.txt")], &into, |_| Resolve::Keep)
        .step;
    ran(rig.undo(|_, _| Resolve::Keep));
    assert_eq!(texts(&rig.bin_root), vec!["original"]);
    fs::remove_dir_all(&rig.bin_root).unwrap();
    let work_before = snapshot(&rig.work);

    let outcomes = ran(rig.redo(|_, _| Resolve::Keep));

    assert_eq!(
        outcomes,
        vec![(0, Outcome::Leaves(Reason::NotInTrash(into.join("a.txt"))))]
    );
    assert_eq!(snapshot(&rig.work), work_before);
    assert_eq!(files(&rig.work), listing(&[("from/a.txt", "original")]));
    assert!(rig.history.step(step).is_none());
    assert!(rig.history.redo_top().is_none());
}

#[test]
fn a_replaced_file_goes_back_and_forth_with_both_files_where_the_step_had_them() {
    for verb in [Verb::Copy, Verb::Move] {
        let mut rig = Rig::new();
        let from = rig.work.join("from");
        let into = rig.work.join("into");
        write_at(&from.join("a.txt"), "incoming", 20);
        write_at(&into.join("a.txt"), "existing", 10);
        rig.transfer(verb, &[from.join("a.txt")], &into, |_| Resolve::Replace);
        let before = rig.held();
        let (done, bin_when_undone) = match verb {
            Verb::Move => (listing(&[("into/a.txt", "incoming")]), Vec::new()),
            Verb::Copy | Verb::Duplicate => (
                listing(&[("from/a.txt", "incoming"), ("into/a.txt", "incoming")]),
                vec!["incoming"],
            ),
        };
        let undone = listing(&[("from/a.txt", "incoming"), ("into/a.txt", "existing")]);
        assert_eq!(files(&rig.work), done, "{verb:?}");

        for round in 0..2 {
            let back = ran(rig.undo(|_, _| Resolve::Keep));
            assert_eq!(back.len(), 2, "{verb:?} round {round}: {back:?}");
            assert!(
                back.iter()
                    .all(|(_, outcome)| matches!(outcome, Outcome::Undone(_))),
                "{verb:?} round {round}: {back:?}"
            );
            assert_eq!(files(&rig.work), undone, "{verb:?} round {round}");
            assert_eq!(
                texts(&rig.bin_root),
                bin_when_undone,
                "{verb:?} round {round}"
            );

            let forth = ran(rig.redo(|_, _| Resolve::Keep));

            assert_eq!(forth.len(), 2, "{verb:?} round {round}: {forth:?}");
            assert_eq!(forth[0].0, 0, "{verb:?} round {round}");
            assert_eq!(forth[1].0, 1, "{verb:?} round {round}");
            assert!(
                matches!(
                    &forth[0].1,
                    Outcome::Redone(Change::Replaced { at, held: Some(_) })
                        if *at == into.join("a.txt")
                ),
                "{verb:?} round {round}: {forth:?}"
            );
            assert!(is_redone(&forth[1].1), "{verb:?} round {round}: {forth:?}");
            assert_eq!(files(&rig.work), done, "{verb:?} round {round}");
            assert_eq!(
                texts(&rig.bin_root),
                vec!["existing"],
                "{verb:?} round {round}"
            );
            assert!(rig.history.redo_top().is_none(), "{verb:?} round {round}");
        }
        rig.keeps(&before, "a replaced file round trip");
    }
}

#[test]
fn a_new_action_after_an_undo_leaves_nothing_to_redo() {
    let mut rig = Rig::new();
    let gone = rig.work.join("gone.txt");
    write_at(&gone, "binned", 20);
    rig.trash(std::slice::from_ref(&gone));
    ran(rig.undo(|_, _| Resolve::Keep));
    assert!(rig.history.redo_top().is_some());
    let work = rig.work.clone();
    rig.make_file(&work, "new.txt").unwrap();

    let redo = rig.redo(|_, _| Resolve::Keep);

    assert!(matches!(redo, Undid::Nothing));
    assert_eq!(
        files(&rig.work),
        listing(&[("gone.txt", "binned"), ("new.txt", "")])
    );
}

#[test]
fn a_redo_that_cannot_bin_the_replaced_file_holds_back_the_move_in() {
    let (mut rig, hooks) = hooked();
    let from = rig.work.join("from");
    let into = rig.work.join("into");
    write_at(&from.join("a.txt"), "incoming", 20);
    write_at(&into.join("a.txt"), "existing", 10);
    let step = rig
        .transfer(Verb::Move, &[from.join("a.txt")], &into, |_| {
            Resolve::Replace
        })
        .step;
    ran(rig.undo(|_, _| Resolve::Keep));
    let undone = listing(&[("from/a.txt", "incoming"), ("into/a.txt", "existing")]);
    assert_eq!(files(&rig.work), undone);
    let before = rig.held();
    hooks.on_trash(refusing());

    let first = ran(rig.redo(|_, _| Resolve::Keep));

    let refused = Reason::Failed(into.join("a.txt"), "the trash is full".into());
    assert_eq!(
        first,
        vec![
            (0, Outcome::Stays(refused)),
            (1, Outcome::Stays(Reason::Stopped)),
        ]
    );
    assert_eq!(files(&rig.work), undone);
    assert!(texts(&rig.bin_root).is_empty());
    assert_eq!(rig.history.redo_top().map(|found| found.id), Some(step));
    hooks.disarm();

    let second = ran(rig.redo(|_, _| Resolve::Keep));

    assert_eq!(second.len(), 2, "{second:?}");
    assert!(
        second.iter().all(|(_, outcome)| is_redone(outcome)),
        "{second:?}"
    );
    assert_eq!(files(&rig.work), listing(&[("into/a.txt", "incoming")]));
    assert_eq!(texts(&rig.bin_root), vec!["existing"]);
    rig.keeps(&before, "a redo held back by a refused trash");
}

#[test]
fn a_copy_whose_name_is_taken_before_its_redo_asks_and_keeps_both() {
    let mut rig = Rig::new();
    let from = rig.work.join("from");
    let into = rig.work.join("into");
    write_at(&from.join("a.txt"), "original", 10);
    fs::create_dir_all(&into).unwrap();
    rig.transfer(Verb::Copy, &[from.join("a.txt")], &into, |_| Resolve::Keep);
    ran(rig.undo(|_, _| Resolve::Keep));
    write_at(&into.join("a.txt"), "taken", 20);
    let before = rig.held();

    let Undid::Ran {
        outcomes, asked, ..
    } = rig.redo(|_, _| Resolve::KeepBoth)
    else {
        panic!("the redo did not run");
    };

    assert_eq!(asked, vec![0]);
    assert_eq!(outcomes.len(), 1, "{outcomes:?}");
    assert!(is_redone(&outcomes[0].1), "{outcomes:?}");
    assert_eq!(
        files(&rig.work),
        listing(&[
            ("from/a.txt", "original"),
            ("into/a (2).txt", "original"),
            ("into/a.txt", "taken"),
        ])
    );
    assert!(texts(&rig.bin_root).is_empty());
    rig.keeps(&before, "a copy name taken before its redo");
}
