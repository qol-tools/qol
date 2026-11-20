use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

use super::copy::{copy_to, remove_copied, Meter};
use super::print::{Print, TreePrint};
use super::{
    partial_name, partial_of, start, survey, Change, Done, Progress, Resolve, Room, Source, Tools,
    Transfer, Verb,
};

fn write(path: &Path, text: &str, age_secs: u64) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
    let when = SystemTime::now() - Duration::from_secs(age_secs);
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

fn clone_file(from: &Path, to: &Path) {
    fs::copy(from, to).unwrap();
    let modified = fs::metadata(from).unwrap().modified().unwrap();
    File::options()
        .write(true)
        .open(to)
        .unwrap()
        .set_modified(modified)
        .unwrap();
}

fn transfer(verb: Verb, sources: &[&Path], into: &Path) -> Transfer {
    Transfer {
        verb,
        sources: sources.iter().map(|path| path.to_path_buf()).collect(),
        into: into.to_path_buf(),
    }
}

#[derive(Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
enum Event {
    Placed(Change),
    Amended { to: PathBuf, source: Source },
    Retracted(PathBuf),
}

fn events(seen: &[Progress]) -> Vec<Event> {
    seen.iter()
        .filter_map(|progress| match progress {
            Progress::Placed(change) => Some(Event::Placed(Change::clone(change))),
            Progress::Amended { to, source } => Some(Event::Amended {
                to: to.clone(),
                source: *source,
            }),
            Progress::Retracted(path) => Some(Event::Retracted(path.clone())),
            _ => None,
        })
        .collect()
}

fn tools_folder() -> (TempDir, Tools) {
    let dir = tempfile::tempdir().unwrap();
    let tools = Tools::at(dir.path());
    (dir, tools)
}

fn run(survey: super::Survey) -> (Done, Vec<Progress>) {
    let (_dir, tools) = tools_folder();
    run_with(survey, &tools)
}

fn run_with(survey: super::Survey, tools: &Tools) -> (Done, Vec<Progress>) {
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = wakes.clone();
    let job = start(survey, tools.clone(), move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let mut seen = Vec::new();
    loop {
        match job.progress.recv().unwrap() {
            Progress::Finished(done) => {
                assert!(wakes.load(Ordering::SeqCst) > 0);
                return (done, seen);
            }
            other => seen.push(other),
        }
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

fn two_folders() -> (TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let from = root.path().join("from");
    let into = root.path().join("into");
    fs::create_dir_all(&from).unwrap();
    fs::create_dir_all(&into).unwrap();
    (root, from, into)
}

#[test]
fn survey_of_clean_copy_has_no_clashes_and_counts_bytes() {
    let (_root, from, into) = two_folders();
    write(&from.join("a.txt"), "12345", 10);
    write(&from.join("dir/b.txt"), "123", 10);
    let found = survey(transfer(
        Verb::Copy,
        &[&from.join("a.txt"), &from.join("dir")],
        &into,
    ))
    .unwrap();
    assert!(found.clashes.is_empty());
    assert_eq!(found.bytes, 8);
    assert_eq!(found.room, Room::Enough);
    assert!(!found.crosses_volume);
    assert!(found.copy_only.is_empty());
}

#[test]
fn survey_defaults_keep_for_newer_here_and_replace_for_older_here() {
    let (_root, from, into) = two_folders();
    write(&from.join("old.txt"), "incoming", 100);
    write(&into.join("old.txt"), "here", 10);
    write(&from.join("new.txt"), "incoming", 10);
    write(&into.join("new.txt"), "here", 100);
    let found = survey(transfer(
        Verb::Copy,
        &[&from.join("old.txt"), &from.join("new.txt")],
        &into,
    ))
    .unwrap();
    assert_eq!(found.clashes.len(), 2);
    assert_eq!(found.clashes[0].choice, Resolve::Keep);
    assert_eq!(found.clashes[1].choice, Resolve::Replace);
    assert!(!found.clashes[0].same);
}

#[test]
fn survey_marks_identical_file_same_and_keep() {
    let (_root, from, into) = two_folders();
    write(&from.join("a.txt"), "same", 50);
    clone_file(&from.join("a.txt"), &into.join("a.txt"));
    let found = survey(transfer(Verb::Copy, &[&from.join("a.txt")], &into)).unwrap();
    assert_eq!(found.clashes.len(), 1);
    assert!(found.clashes[0].same);
    assert_eq!(found.clashes[0].choice, Resolve::Keep);
    assert_eq!(found.bytes, 0);
}

#[test]
fn survey_asks_about_a_folder_onto_a_folder_and_lists_differing_files_inside() {
    let (_root, from, into) = two_folders();
    for name in ["one", "two", "three"] {
        write(&from.join("docs").join(name), name, 50);
    }
    write(&from.join("docs/fresh"), "fresh", 50);
    fs::create_dir_all(into.join("docs")).unwrap();
    clone_file(&from.join("docs/one"), &into.join("docs/one"));
    clone_file(&from.join("docs/two"), &into.join("docs/two"));
    write(&into.join("docs/three"), "different!", 5);
    let found = survey(transfer(Verb::Copy, &[&from.join("docs")], &into)).unwrap();
    assert_eq!(found.same_inside, 2);
    assert_eq!(found.clashes.len(), 2);
    assert!(found.clashes[0].merges());
    assert_eq!(found.clashes[0].existing, into.join("docs"));
    assert_eq!(found.clashes[0].choice, Resolve::KeepBoth);
    assert_eq!(found.clashes[1].existing, into.join("docs/three"));
    let whole: u64 = ["one", "two", "three", "fresh"]
        .iter()
        .map(|name| name.len() as u64)
        .sum();
    assert_eq!(found.bytes, whole);
}

#[test]
fn a_folder_onto_a_folder_keeps_both_merges_or_stays_as_chosen() {
    let cases = [
        (None, vec!["hello", "hello (2)"], vec!["old.txt"], false),
        (
            Some(Resolve::Merge),
            vec!["hello"],
            vec!["new.txt", "old.txt"],
            false,
        ),
        (Some(Resolve::Keep), vec!["hello"], vec!["old.txt"], true),
    ];
    for (choice, landed, inside, stays) in cases {
        let (_root, from, into) = two_folders();
        write(&from.join("hello/new.txt"), "new", 10);
        write(&into.join("hello/old.txt"), "old", 10);
        let mut found = survey(transfer(Verb::Move, &[&from.join("hello")], &into)).unwrap();
        assert_eq!(found.clashes.len(), 1, "{choice:?}");
        assert!(found.clashes[0].merges(), "{choice:?}");
        assert_eq!(found.clashes[0].choice, Resolve::KeepBoth, "{choice:?}");
        if let Some(choice) = choice {
            found.clashes[0].choice = choice;
        }
        let (done, _) = run(found);
        assert!(done.failures.is_empty(), "{choice:?}");
        assert_eq!(names(&into), landed, "{choice:?}");
        assert_eq!(names(&into.join("hello")), inside, "{choice:?}");
        assert_eq!(from.join("hello").exists(), stays, "{choice:?}");
        if into.join("hello (2)").exists() {
            assert_eq!(names(&into.join("hello (2)")), vec!["new.txt"]);
        }
    }
}

#[test]
fn rows_inside_a_folder_apply_only_while_it_merges_and_choices_fit_each_row() {
    let (_root, from, into) = two_folders();
    write(&from.join("docs/a.txt"), "incoming", 10);
    write(&into.join("docs/a.txt"), "here", 100);
    write(&from.join("b.txt"), "incoming", 10);
    write(&into.join("b.txt"), "here", 100);
    let mut found = survey(transfer(
        Verb::Copy,
        &[&from.join("docs"), &from.join("b.txt")],
        &into,
    ))
    .unwrap();
    let existing: Vec<PathBuf> = found
        .clashes
        .iter()
        .map(|clash| clash.existing.clone())
        .collect();
    assert_eq!(
        existing,
        vec![
            into.join("docs"),
            into.join("docs/a.txt"),
            into.join("b.txt")
        ]
    );
    let applies = |found: &super::Survey| {
        found
            .clashes
            .iter()
            .map(|clash| clash.applies(&found.clashes))
            .collect::<Vec<_>>()
    };
    assert_eq!(applies(&found), vec![true, false, true]);
    found.clashes[0].choice = Resolve::Merge;
    assert_eq!(applies(&found), vec![true, true, true]);
    let fitted = |choice| {
        found
            .clashes
            .iter()
            .map(|clash| clash.fitted(choice))
            .collect::<Vec<_>>()
    };
    let middle = vec![Resolve::Merge, Resolve::Replace, Resolve::Replace];
    assert_eq!(fitted(Resolve::Replace), middle);
    assert_eq!(fitted(Resolve::Merge), middle);
    assert_eq!(fitted(Resolve::KeepBoth), vec![Resolve::KeepBoth; 3]);
    assert_eq!(fitted(Resolve::Keep), vec![Resolve::Keep; 3]);
}

#[test]
fn survey_refuses_a_folder_inside_itself() {
    let (_root, from, _into) = two_folders();
    fs::create_dir_all(from.join("sub")).unwrap();
    let error = survey(transfer(Verb::Move, &[&from], &from.join("sub"))).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn survey_target_must_be_a_folder() {
    let (_root, from, _into) = two_folders();
    write(&from.join("a.txt"), "x", 1);
    assert!(survey(transfer(
        Verb::Copy,
        &[&from.join("a.txt")],
        &from.join("a.txt")
    ))
    .is_err());
}

#[test]
fn move_on_one_volume_renames_and_records_each_move() {
    let (_root, from, into) = two_folders();
    write(&from.join("a.txt"), "hello", 10);
    write(&from.join("dir/b.txt"), "world", 10);
    let file_at = Print::read(&from.join("a.txt")).unwrap();
    let dir_at = Print::read(&from.join("dir")).unwrap();
    let found = survey(transfer(
        Verb::Move,
        &[&from.join("a.txt"), &from.join("dir")],
        &into,
    ))
    .unwrap();
    let (done, seen) = run(found);
    assert!(done.failures.is_empty());
    assert_eq!(done.verb, Verb::Move);
    assert_eq!(
        done.placed,
        vec![
            (from.join("a.txt"), into.join("a.txt")),
            (from.join("dir"), into.join("dir")),
        ]
    );
    assert_eq!(
        events(&seen),
        vec![
            Event::Placed(Change::Moved {
                from: from.join("a.txt"),
                to: into.join("a.txt"),
                at: file_at,
            }),
            Event::Placed(Change::Moved {
                from: from.join("dir"),
                to: into.join("dir"),
                at: dir_at,
            }),
        ]
    );
    assert!(!from.join("a.txt").exists());
    assert_eq!(fs::read_to_string(into.join("dir/b.txt")).unwrap(), "world");
    assert!(seen
        .iter()
        .any(|p| matches!(p, Progress::Arriving(path) if *path == into.join("a.txt"))));
}

#[test]
fn copy_keeps_originals_and_modified_times_and_leaves_no_partial() {
    let (_root, from, into) = two_folders();
    write(&from.join("dir/b.txt"), "world", 3600);
    let before = fs::metadata(from.join("dir/b.txt"))
        .unwrap()
        .modified()
        .unwrap();
    let (_dir, tools) = tools_folder();
    let original = TreePrint::read(&from.join("dir"), &tools.key).unwrap();
    let found = survey(transfer(Verb::Copy, &[&from.join("dir")], &into)).unwrap();
    let (done, seen) = run_with(found, &tools);
    assert!(done.failures.is_empty());
    let tree = TreePrint::read(&into.join("dir"), &tools.key).unwrap();
    assert_eq!(
        events(&seen),
        vec![Event::Placed(Change::Copied {
            from: from.join("dir"),
            to: into.join("dir"),
            tree,
            original,
        })]
    );
    assert!(from.join("dir/b.txt").exists());
    let after = fs::metadata(into.join("dir/b.txt"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(names(&into), vec!["dir"]);
    assert!(matches!(
        seen.last(),
        Some(Progress::Bytes { done: 5, total: 5 })
    ));
}

#[test]
fn copy_into_its_own_folder_makes_a_numbered_copy() {
    let (_root, from, _into) = two_folders();
    write(&from.join("a.md"), "text", 10);
    let found = survey(transfer(Verb::Copy, &[&from.join("a.md")], &from)).unwrap();
    assert!(found.clashes.is_empty());
    let (done, seen) = run(found);
    assert_eq!(
        done.placed,
        vec![(from.join("a.md"), from.join("a (2).md"))]
    );
    assert!(matches!(
        events(&seen).as_slice(),
        [Event::Placed(Change::Copied { to, .. })] if *to == from.join("a (2).md")
    ));
    assert_eq!(names(&from), vec!["a (2).md", "a.md"]);
}

#[test]
fn move_into_its_own_folder_does_nothing() {
    let (_root, from, _into) = two_folders();
    write(&from.join("a.md"), "text", 10);
    let found = survey(transfer(Verb::Move, &[&from.join("a.md")], &from)).unwrap();
    let (done, _) = run(found);
    assert!(done.placed.is_empty());
    assert!(done.failures.is_empty());
    assert_eq!(names(&from), vec!["a.md"]);
}

#[test]
fn keep_choice_leaves_both_files_untouched() {
    let (_root, from, into) = two_folders();
    write(&from.join("a.txt"), "incoming", 100);
    write(&into.join("a.txt"), "here", 10);
    let mut found = survey(transfer(Verb::Move, &[&from.join("a.txt")], &into)).unwrap();
    found.clashes[0].choice = Resolve::Keep;
    let (done, seen) = run(found);
    assert!(done.placed.is_empty());
    assert_eq!(events(&seen), Vec::new());
    assert_eq!(fs::read_to_string(from.join("a.txt")).unwrap(), "incoming");
    assert_eq!(fs::read_to_string(into.join("a.txt")).unwrap(), "here");
}

#[test]
fn keep_both_choice_lands_under_a_free_name() {
    for verb in [Verb::Copy, Verb::Move] {
        let (_root, from, into) = two_folders();
        write(&from.join("a.txt"), "incoming", 100);
        write(&into.join("a.txt"), "here", 10);
        let at = Print::read(&from.join("a.txt")).unwrap();
        let mut found = survey(transfer(verb, &[&from.join("a.txt")], &into)).unwrap();
        found.clashes[0].choice = Resolve::KeepBoth;
        let (done, seen) = run(found);
        assert_eq!(
            done.placed,
            vec![(from.join("a.txt"), into.join("a (2).txt"))]
        );
        assert_eq!(fs::read_to_string(into.join("a.txt")).unwrap(), "here");
        assert_eq!(
            fs::read_to_string(into.join("a (2).txt")).unwrap(),
            "incoming"
        );
        match (verb, events(&seen).as_slice()) {
            (
                Verb::Move,
                [Event::Placed(Change::Moved {
                    from: was,
                    to,
                    at: moved,
                }), ..],
            ) => {
                assert_eq!(*was, from.join("a.txt"));
                assert_eq!(*to, into.join("a (2).txt"));
                assert_eq!(*moved, at);
            }
            (Verb::Copy, [Event::Placed(Change::Copied { from: was, to, .. })]) => {
                assert_eq!(*was, from.join("a.txt"));
                assert_eq!(*to, into.join("a (2).txt"));
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn replace_choice_trashes_the_old_file_then_places_the_incoming_one() {
    let (_root, from, into) = two_folders();
    let (dir, tools) = tools_folder();
    write(&from.join("a.txt"), "incoming", 10);
    write(&into.join("a.txt"), "here", 100);
    let original = TreePrint::read(&from.join("a.txt"), &tools.key).unwrap();
    let mut found = survey(transfer(Verb::Copy, &[&from.join("a.txt")], &into)).unwrap();
    found.clashes[0].choice = Resolve::Replace;
    let (done, seen) = run_with(found, &tools);
    assert!(done.failures.is_empty());
    assert_eq!(fs::read_to_string(into.join("a.txt")).unwrap(), "incoming");
    let seen = events(&seen);
    assert_eq!(seen.len(), 2);
    let Event::Placed(Change::Replaced {
        at,
        held: Some(held),
    }) = &seen[0]
    else {
        panic!("{seen:?}");
    };
    assert_eq!(*at, into.join("a.txt"));
    assert!(held.item.starts_with(dir.path().join("bin")));
    assert_eq!(fs::read_to_string(&held.item).unwrap(), "here");
    let tree = TreePrint::read(&into.join("a.txt"), &tools.key).unwrap();
    assert_eq!(
        seen[1],
        Event::Placed(Change::Copied {
            from: from.join("a.txt"),
            to: into.join("a.txt"),
            tree,
            original,
        })
    );
    fs::remove_file(into.join("a.txt")).unwrap();
    tools.bin.restore(held, &into.join("a.txt")).unwrap();
    assert_eq!(fs::read_to_string(into.join("a.txt")).unwrap(), "here");
}

#[test]
fn move_merge_trashes_identical_files_and_keeps_kept_ones() {
    let (_root, from, into) = two_folders();
    let (dir, tools) = tools_folder();
    write(&from.join("docs/same"), "same", 50);
    write(&from.join("docs/new"), "new", 50);
    write(&from.join("docs/clash"), "incoming", 100);
    fs::create_dir_all(into.join("docs")).unwrap();
    clone_file(&from.join("docs/same"), &into.join("docs/same"));
    write(&into.join("docs/clash"), "here", 10);
    let mut found = survey(transfer(Verb::Move, &[&from.join("docs")], &into)).unwrap();
    assert_eq!(found.clashes[1].choice, Resolve::Keep);
    found.clashes[0].choice = Resolve::Merge;
    let (done, seen) = run_with(found, &tools);
    assert!(done.failures.is_empty());
    assert_eq!(names(&from.join("docs")), vec!["clash"]);
    assert_eq!(names(&into.join("docs")), vec!["clash", "new", "same"]);
    assert_eq!(fs::read_to_string(into.join("docs/clash")).unwrap(), "here");
    let seen = events(&seen);
    assert_eq!(seen.len(), 2);
    assert!(matches!(
        &seen[0],
        Event::Placed(Change::Moved { from: moved, to, .. })
            if *moved == from.join("docs/new") && *to == into.join("docs/new")
    ));
    let Event::Placed(Change::Matched {
        from: child,
        held: Some(held),
    }) = &seen[1]
    else {
        panic!("{seen:?}");
    };
    assert_eq!(*child, from.join("docs/same"));
    assert!(held.item.starts_with(dir.path().join("bin")));
    assert_eq!(fs::read_to_string(&held.item).unwrap(), "same");
}

#[test]
fn move_merge_trashes_an_identical_child_and_records_the_emptied_folder() {
    let (_root, from, into) = two_folders();
    let (dir, tools) = tools_folder();
    write(&from.join("docs/same"), "same", 50);
    fs::create_dir_all(into.join("docs")).unwrap();
    clone_file(&from.join("docs/same"), &into.join("docs/same"));
    let folder = Print::read(&from.join("docs")).unwrap();
    let mut found = survey(transfer(Verb::Move, &[&from.join("docs")], &into)).unwrap();
    found.clashes[0].choice = Resolve::Merge;
    let (done, seen) = run_with(found, &tools);
    assert!(done.failures.is_empty());
    assert!(!from.join("docs").exists());
    assert_eq!(names(&into.join("docs")), vec!["same"]);
    let seen = events(&seen);
    assert_eq!(seen.len(), 2);
    let Event::Placed(Change::Matched {
        from: child,
        held: Some(held),
    }) = &seen[0]
    else {
        panic!("{seen:?}");
    };
    assert_eq!(*child, from.join("docs/same"));
    assert!(held.item.starts_with(dir.path().join("bin")));
    assert_eq!(fs::read_to_string(&held.item).unwrap(), "same");
    let Event::Placed(Change::Emptied {
        folder: emptied,
        at,
    }) = &seen[1]
    else {
        panic!("{seen:?}");
    };
    assert_eq!(*emptied, from.join("docs"));
    assert!(at.same_object(&folder));
}

#[test]
fn copy_leaves_an_existing_partial_named_file_alone() {
    let (_root, from, into) = two_folders();
    write(&from.join("a.txt"), "incoming", 10);
    write(&into.join(".a.txt.qol-partial"), "mine", 10);
    let found = survey(transfer(Verb::Copy, &[&from.join("a.txt")], &into)).unwrap();
    let (done, _) = run(found);
    assert!(done.failures.is_empty());
    assert_eq!(fs::read_to_string(into.join("a.txt")).unwrap(), "incoming");
    assert_eq!(
        fs::read_to_string(into.join(".a.txt.qol-partial")).unwrap(),
        "mine"
    );
}

#[test]
fn copy_to_takes_the_next_partial_name_and_leaves_the_taken_ones_alone() {
    let (_root, from, into) = two_folders();
    write(&from.join("a.txt"), "incoming", 10);
    write(&into.join(".a.txt.qol-partial"), "one", 10);
    write(&into.join(".a.txt (2).qol-partial"), "two", 10);
    let copied = copy_to(&from.join("a.txt"), &into.join("a.txt"), &mut Quiet, false).unwrap();
    assert_eq!(copied.len(), 1);
    assert_eq!(fs::read_to_string(into.join("a.txt")).unwrap(), "incoming");
    assert_eq!(
        names(&into),
        vec![".a.txt (2).qol-partial", ".a.txt.qol-partial", "a.txt"]
    );
    assert_eq!(
        fs::read_to_string(into.join(".a.txt.qol-partial")).unwrap(),
        "one"
    );
    assert_eq!(
        fs::read_to_string(into.join(".a.txt (2).qol-partial")).unwrap(),
        "two"
    );
}

#[test]
fn copy_to_never_replaces_the_target_and_removes_only_its_own_partial() {
    let (_root, from, into) = two_folders();
    write(&from.join("a.txt"), "incoming", 10);
    write(&into.join("a.txt"), "here", 10);
    write(&into.join(".a.txt.qol-partial"), "mine", 10);
    let error = copy_to(&from.join("a.txt"), &into.join("a.txt"), &mut Quiet, true)
        .err()
        .unwrap();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(into.join("a.txt")).unwrap(), "here");
    assert_eq!(names(&into), vec![".a.txt.qol-partial", "a.txt"]);
    assert_eq!(
        fs::read_to_string(into.join(".a.txt.qol-partial")).unwrap(),
        "mine"
    );
}

#[test]
fn copy_to_with_sync_copies_a_whole_folder() {
    let (_root, from, into) = two_folders();
    write(&from.join("dir/a.txt"), "a", 10);
    write(&from.join("dir/sub/b.txt"), "b", 10);
    let copied = copy_to(&from.join("dir"), &into.join("dir"), &mut Quiet, true).unwrap();
    assert_eq!(copied.len(), 4);
    assert_eq!(names(&into), vec!["dir"]);
    assert_eq!(fs::read_to_string(into.join("dir/sub/b.txt")).unwrap(), "b");
}

#[cfg(unix)]
#[test]
fn a_failed_copy_removes_only_its_own_partial() {
    use std::os::unix::fs::PermissionsExt;
    let (_root, from, into) = two_folders();
    write(&from.join("a.txt"), "incoming", 10);
    write(&into.join(".a.txt.qol-partial"), "mine", 10);
    fs::set_permissions(from.join("a.txt"), fs::Permissions::from_mode(0o000)).unwrap();
    let outcome = copy_to(&from.join("a.txt"), &into.join("a.txt"), &mut Quiet, false);
    assert!(outcome.is_err());
    assert_eq!(names(&into), vec![".a.txt.qol-partial"]);
    assert_eq!(
        fs::read_to_string(into.join(".a.txt.qol-partial")).unwrap(),
        "mine"
    );
}

#[test]
fn partial_names_stay_within_a_file_name_limit() {
    let long = format!("{}a", "é".repeat(127));
    let partial = partial_name(&Path::new("/x").join(&long)).unwrap();
    let name = partial.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.len() + " (99)".len() <= 255, "{}", name.len());
    assert!(name.ends_with(".qol-partial"));
}

#[test]
fn a_numbered_partial_still_belongs_to_its_target() {
    let target = Path::new("/x/a.txt");
    assert!(partial_of(target, Path::new("/x/.a.txt.qol-partial")));
    assert!(partial_of(target, Path::new("/x/.a.txt (2).qol-partial")));
    assert!(!partial_of(target, Path::new("/y/.a.txt.qol-partial")));
    assert!(!partial_of(target, Path::new("/x/.b.txt.qol-partial")));
}

#[cfg(unix)]
#[test]
fn replace_puts_the_original_back_when_the_incoming_file_cannot_be_read() {
    use std::os::unix::fs::PermissionsExt;
    let (_root, from, into) = two_folders();
    let (dir, tools) = tools_folder();
    write(&from.join("a.txt"), "incoming", 10);
    write(&into.join("a.txt"), "here", 100);
    let mut found = survey(transfer(Verb::Copy, &[&from.join("a.txt")], &into)).unwrap();
    found.clashes[0].choice = Resolve::Replace;
    fs::set_permissions(from.join("a.txt"), fs::Permissions::from_mode(0o000)).unwrap();
    let (done, seen) = run_with(found, &tools);
    assert_eq!(done.failures.len(), 1);
    assert_eq!(fs::read_to_string(into.join("a.txt")).unwrap(), "here");
    let seen = events(&seen);
    assert_eq!(seen.len(), 2);
    assert!(matches!(
        &seen[0],
        Event::Placed(Change::Replaced { at, held: Some(_) }) if *at == into.join("a.txt")
    ));
    assert_eq!(seen[1], Event::Retracted(into.join("a.txt")));
    assert_eq!(names(&dir.path().join("bin")), Vec::<String>::new());
}

#[test]
fn copy_merge_replace_lands_the_new_file_and_records_each_arrival() {
    let (_root, from, into) = two_folders();
    write(&from.join("docs/clash"), "incoming", 10);
    write(&from.join("docs/new"), "new", 10);
    write(&into.join("docs/clash"), "here", 100);
    write(&into.join("docs/kept"), "kept", 100);
    let mut found = survey(transfer(Verb::Copy, &[&from.join("docs")], &into)).unwrap();
    found.clashes[0].choice = Resolve::Merge;
    found.clashes[1].choice = Resolve::Replace;
    let (done, seen) = run(found);
    assert!(done.failures.is_empty());
    assert_eq!(
        fs::read_to_string(into.join("docs/clash")).unwrap(),
        "incoming"
    );
    assert_eq!(done.placed, vec![(from.join("docs"), into.join("docs"))]);
    let seen = events(&seen);
    assert_eq!(seen.len(), 3);
    assert!(matches!(
        &seen[0],
        Event::Placed(Change::Replaced { at, .. }) if *at == into.join("docs/clash")
    ));
    assert!(matches!(
        &seen[1],
        Event::Placed(Change::Copied { to, .. }) if *to == into.join("docs/clash")
    ));
    assert!(matches!(
        &seen[2],
        Event::Placed(Change::Copied { to, .. }) if *to == into.join("docs/new")
    ));
}

#[test]
fn move_merge_removes_the_source_folder_when_nothing_is_left() {
    let (_root, from, into) = two_folders();
    write(&from.join("docs/new"), "new", 50);
    fs::create_dir_all(into.join("docs")).unwrap();
    let mut found = survey(transfer(Verb::Move, &[&from.join("docs")], &into)).unwrap();
    found.clashes[0].choice = Resolve::Merge;
    let (done, seen) = run(found);
    assert!(done.failures.is_empty());
    assert!(!from.join("docs").exists());
    assert_eq!(names(&into.join("docs")), vec!["new"]);
    let seen = events(&seen);
    assert_eq!(seen.len(), 2);
    assert!(matches!(
        &seen[0],
        Event::Placed(Change::Moved { to, .. }) if *to == into.join("docs/new")
    ));
    assert!(matches!(
        &seen[1],
        Event::Placed(Change::Emptied { folder, .. }) if *folder == from.join("docs")
    ));
}

#[test]
fn move_merge_records_the_emptied_folders_it_removed() {
    let (_root, from, into) = two_folders();
    fs::create_dir_all(from.join("docs/empty")).unwrap();
    fs::create_dir_all(into.join("docs/empty")).unwrap();
    let before = Print::read(&from.join("docs/empty")).unwrap();
    let mut found = survey(transfer(Verb::Move, &[&from.join("docs")], &into)).unwrap();
    found.clashes[0].choice = Resolve::Merge;
    let (done, seen) = run(found);
    assert!(done.failures.is_empty());
    assert!(!from.join("docs").exists());
    assert!(into.join("docs/empty").is_dir());
    let seen = events(&seen);
    assert_eq!(seen.len(), 2);
    assert_eq!(
        seen[0],
        Event::Placed(Change::Emptied {
            folder: from.join("docs/empty"),
            at: before,
        })
    );
    assert!(matches!(
        &seen[1],
        Event::Placed(Change::Emptied { folder, .. }) if *folder == from.join("docs")
    ));
}

#[test]
fn missing_source_is_reported_as_a_failure_with_a_reason() {
    let (_root, from, into) = two_folders();
    write(&from.join("a.txt"), "x", 10);
    let found = survey(transfer(Verb::Copy, &[&from.join("a.txt")], &into)).unwrap();
    fs::remove_file(from.join("a.txt")).unwrap();
    let (done, _) = run(found);
    assert_eq!(done.failures.len(), 1);
    assert_eq!(done.failures[0].path, from.join("a.txt"));
    assert!(!done.failures[0].reason.is_empty());
    assert_eq!(names(&into), Vec::<String>::new());
}

#[test]
fn job_ids_are_unique() {
    let (root, from, into) = two_folders();
    let other = root.path().join("other");
    fs::create_dir_all(&other).unwrap();
    write(&from.join("a.txt"), "x", 10);
    let (_dir, tools) = tools_folder();
    let first = start(
        survey(transfer(Verb::Copy, &[&from.join("a.txt")], &into)).unwrap(),
        tools.clone(),
        || {},
    );
    let second = start(
        survey(transfer(Verb::Copy, &[&from.join("a.txt")], &other)).unwrap(),
        tools,
        || {},
    );
    assert_ne!(first.id, second.id);
}

#[cfg(unix)]
#[test]
fn links_with_the_same_target_count_as_the_same_whatever_their_age() {
    let (_root, from, into) = two_folders();
    fs::create_dir_all(from.join("docs")).unwrap();
    fs::create_dir_all(into.join("docs")).unwrap();
    std::os::unix::fs::symlink("../shared", from.join("docs/link")).unwrap();
    std::thread::sleep(Duration::from_millis(2500));
    std::os::unix::fs::symlink("../shared", into.join("docs/link")).unwrap();
    let found = survey(transfer(Verb::Copy, &[&from.join("docs")], &into)).unwrap();
    assert_eq!(found.clashes.len(), 1);
    assert!(found.clashes[0].merges());
    assert_eq!(found.same_inside, 1);
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[test]
fn a_hard_linked_pair_is_a_kept_clash_and_a_move_leaves_the_source_file() {
    let (_root, from, into) = two_folders();
    write(&from.join("a.txt"), "shared", 50);
    fs::hard_link(from.join("a.txt"), into.join("a.txt")).unwrap();
    let found = survey(transfer(Verb::Move, &[&from.join("a.txt")], &into)).unwrap();
    assert_eq!(found.clashes.len(), 1);
    assert!(!found.clashes[0].same);
    assert_eq!(found.clashes[0].choice, Resolve::Keep);
    assert_eq!(found.bytes, 0);
    let (done, seen) = run(found);
    assert!(done.failures.is_empty());
    assert!(done.placed.is_empty());
    assert_eq!(events(&seen), Vec::new());
    assert_eq!(fs::read_to_string(from.join("a.txt")).unwrap(), "shared");
    assert_eq!(fs::read_to_string(into.join("a.txt")).unwrap(), "shared");
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[test]
fn a_hard_linked_pair_inside_a_merge_is_a_kept_clash_and_stays_put() {
    let (_root, from, into) = two_folders();
    write(&from.join("docs/a.txt"), "shared", 50);
    fs::create_dir_all(into.join("docs")).unwrap();
    fs::hard_link(from.join("docs/a.txt"), into.join("docs/a.txt")).unwrap();
    let mut found = survey(transfer(Verb::Move, &[&from.join("docs")], &into)).unwrap();
    assert_eq!(found.same_inside, 0);
    assert_eq!(found.clashes.len(), 2);
    assert_eq!(found.clashes[1].choice, Resolve::Keep);
    found.clashes[0].choice = Resolve::Merge;
    let (done, seen) = run(found);
    assert!(done.failures.is_empty());
    assert_eq!(events(&seen), Vec::new());
    assert_eq!(
        fs::read_to_string(from.join("docs/a.txt")).unwrap(),
        "shared"
    );
    assert_eq!(
        fs::read_to_string(into.join("docs/a.txt")).unwrap(),
        "shared"
    );
}

#[cfg(unix)]
#[test]
fn survey_refuses_a_folder_inside_itself_through_a_symlinked_target() {
    let (root, from, _into) = two_folders();
    fs::create_dir_all(from.join("sub")).unwrap();
    let link = root.path().join("link");
    std::os::unix::fs::symlink(from.join("sub"), &link).unwrap();
    let error = survey(transfer(Verb::Move, &[&from], &link)).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

struct Quiet;

impl Meter for Quiet {
    fn advance(&mut self, _bytes: u64) {}
}

fn copied_tree() -> (TempDir, PathBuf, Vec<super::copy::Copied>) {
    let root = tempfile::tempdir().unwrap();
    let src = root.path().join("src");
    write(&src.join("a.txt"), "a", 50);
    write(&src.join("b.txt"), "b", 50);
    write(&src.join("sub/c.txt"), "c", 50);
    let copied = copy_to(&src, &root.path().join("dst"), &mut Quiet, false).unwrap();
    (root, src, copied)
}

#[test]
fn removing_a_recorded_tree_takes_every_recorded_entry() {
    let (_root, src, copied) = copied_tree();
    remove_copied(&copied).unwrap();
    assert!(!src.exists());
}

#[test]
fn removal_keeps_a_file_that_landed_in_a_copied_folder_and_stops_at_the_error() {
    let (_root, src, copied) = copied_tree();
    write(&src.join("sub/late.txt"), "late", 1);
    let error = remove_copied(&copied).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::DirectoryNotEmpty);
    assert_eq!(names(&src), vec!["a.txt", "b.txt", "sub"]);
    assert_eq!(names(&src.join("sub")), vec!["late.txt"]);
}

#[test]
fn removal_keeps_a_file_edited_after_it_was_copied_and_stops_at_the_error() {
    let (_root, src, copied) = copied_tree();
    write(&src.join("a.txt"), "edited after the copy", 1);
    let error = remove_copied(&copied).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Other);
    assert_eq!(names(&src), vec!["a.txt"]);
    assert_eq!(
        fs::read_to_string(src.join("a.txt")).unwrap(),
        "edited after the copy"
    );
}

#[cfg(unix)]
fn other_device_folder(like: &Path) -> Option<TempDir> {
    use std::os::unix::fs::MetadataExt;
    let other = tempfile::tempdir_in("/dev/shm").ok()?;
    let here = fs::metadata(like).ok()?.dev();
    let there = fs::metadata(other.path()).ok()?.dev();
    (here != there).then_some(other)
}

#[cfg(unix)]
#[test]
fn a_cross_volume_move_records_the_copy_before_removing_the_source() {
    let (_root, from, _into) = two_folders();
    let Some(far) = other_device_folder(&from) else {
        return;
    };
    let (_dir, tools) = tools_folder();
    write(&from.join("dir/a.txt"), "hello", 10);
    let device = Print::read(&from.join("dir")).unwrap().device();
    let found = survey(transfer(Verb::Move, &[&from.join("dir")], far.path())).unwrap();
    let (done, seen) = run_with(found, &tools);
    assert!(done.failures.is_empty());
    assert!(!from.join("dir").exists());
    assert_eq!(
        fs::read_to_string(far.path().join("dir/a.txt")).unwrap(),
        "hello"
    );
    let tree = TreePrint::read(&far.path().join("dir"), &tools.key).unwrap();
    assert_eq!(
        events(&seen),
        vec![
            Event::Placed(Change::Crossed {
                from: from.join("dir"),
                to: far.path().join("dir"),
                tree,
                source: Source::Removing,
                from_device: device,
            }),
            Event::Amended {
                to: far.path().join("dir"),
                source: Source::Removed,
            },
        ]
    );
}
