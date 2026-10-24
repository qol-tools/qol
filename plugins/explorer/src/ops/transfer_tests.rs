use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

use super::{partial_name, start, survey, Done, Progress, Resolve, Room, Transfer, Verb};

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

fn run(survey: super::Survey) -> (Done, Vec<Progress>) {
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = wakes.clone();
    let job = start(survey, move || {
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
fn survey_merges_folder_onto_folder_and_lists_only_differing_files() {
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
    assert_eq!(found.clashes.len(), 1);
    assert_eq!(found.clashes[0].existing, into.join("docs/three"));
    assert_eq!(found.bytes, "three".len() as u64 + "fresh".len() as u64);
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
fn move_on_one_volume_renames_and_reports_placed() {
    let (_root, from, into) = two_folders();
    write(&from.join("a.txt"), "hello", 10);
    write(&from.join("dir/b.txt"), "world", 10);
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
    let found = survey(transfer(Verb::Copy, &[&from.join("dir")], &into)).unwrap();
    let (done, seen) = run(found);
    assert!(done.failures.is_empty());
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
    let (done, _) = run(found);
    assert_eq!(
        done.placed,
        vec![(from.join("a.md"), from.join("a (2).md"))]
    );
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
    let (done, _) = run(found);
    assert!(done.placed.is_empty());
    assert_eq!(fs::read_to_string(from.join("a.txt")).unwrap(), "incoming");
    assert_eq!(fs::read_to_string(into.join("a.txt")).unwrap(), "here");
}

#[test]
fn keep_both_choice_lands_under_a_free_name() {
    let (_root, from, into) = two_folders();
    write(&from.join("a.txt"), "incoming", 100);
    write(&into.join("a.txt"), "here", 10);
    let mut found = survey(transfer(Verb::Copy, &[&from.join("a.txt")], &into)).unwrap();
    found.clashes[0].choice = Resolve::KeepBoth;
    let (done, _) = run(found);
    assert_eq!(
        done.placed,
        vec![(from.join("a.txt"), into.join("a (2).txt"))]
    );
    assert_eq!(fs::read_to_string(into.join("a.txt")).unwrap(), "here");
    assert_eq!(
        fs::read_to_string(into.join("a (2).txt")).unwrap(),
        "incoming"
    );
}

#[test]
fn replace_choice_puts_incoming_in_place_of_existing() {
    let (_root, from, into) = two_folders();
    write(&from.join("a.txt"), "incoming", 10);
    write(&into.join("a.txt"), "here", 100);
    let mut found = survey(transfer(Verb::Copy, &[&from.join("a.txt")], &into)).unwrap();
    found.clashes[0].choice = Resolve::Replace;
    let (done, _) = run(found);
    assert!(done.failures.is_empty());
    assert_eq!(fs::read_to_string(into.join("a.txt")).unwrap(), "incoming");
}

#[test]
fn move_merge_removes_identical_and_moved_files_but_keeps_kept_ones() {
    let (_root, from, into) = two_folders();
    write(&from.join("docs/same"), "same", 50);
    write(&from.join("docs/new"), "new", 50);
    write(&from.join("docs/clash"), "incoming", 100);
    fs::create_dir_all(into.join("docs")).unwrap();
    clone_file(&from.join("docs/same"), &into.join("docs/same"));
    write(&into.join("docs/clash"), "here", 10);
    let found = survey(transfer(Verb::Move, &[&from.join("docs")], &into)).unwrap();
    assert_eq!(found.clashes[0].choice, Resolve::Keep);
    let (done, _) = run(found);
    assert!(done.failures.is_empty());
    assert_eq!(names(&from.join("docs")), vec!["clash"]);
    assert_eq!(names(&into.join("docs")), vec!["clash", "new", "same"]);
    assert_eq!(fs::read_to_string(into.join("docs/clash")).unwrap(), "here");
    assert_eq!(
        done.matched,
        vec![(from.join("docs/same"), into.join("docs/same"))]
    );
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
fn partial_names_stay_within_a_file_name_limit() {
    let long = format!("{}a", "é".repeat(127));
    let partial = partial_name(&Path::new("/x").join(&long)).unwrap();
    let name = partial.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.len() <= 240, "{}", name.len());
    assert!(name.ends_with(".qol-partial"));
}

#[cfg(unix)]
#[test]
fn replace_puts_the_original_back_when_the_incoming_file_cannot_be_read() {
    use std::os::unix::fs::PermissionsExt;
    let (_root, from, into) = two_folders();
    write(&from.join("a.txt"), "incoming", 10);
    write(&into.join("a.txt"), "here", 100);
    let mut found = survey(transfer(Verb::Copy, &[&from.join("a.txt")], &into)).unwrap();
    found.clashes[0].choice = Resolve::Replace;
    fs::set_permissions(from.join("a.txt"), fs::Permissions::from_mode(0o000)).unwrap();
    let (done, _) = run(found);
    assert!(!done.failures.is_empty());
    assert!(done.replaced.is_empty());
    assert_eq!(fs::read_to_string(into.join("a.txt")).unwrap(), "here");
}

#[test]
fn copy_merge_replace_lands_the_new_file_and_undo_reverses_only_arrivals() {
    let (_root, from, into) = two_folders();
    write(&from.join("docs/clash"), "incoming", 10);
    write(&from.join("docs/new"), "new", 10);
    write(&into.join("docs/clash"), "here", 100);
    write(&into.join("docs/kept"), "kept", 100);
    let mut found = survey(transfer(Verb::Copy, &[&from.join("docs")], &into)).unwrap();
    found.clashes[0].choice = Resolve::Replace;
    let (done, _) = run(found);
    assert!(done.failures.is_empty());
    assert_eq!(
        fs::read_to_string(into.join("docs/clash")).unwrap(),
        "incoming"
    );
    assert_eq!(done.placed, vec![(from.join("docs"), into.join("docs"))]);
    let mut landed = done.landed.clone();
    landed.sort();
    assert_eq!(
        landed,
        vec![
            (from.join("docs/clash"), into.join("docs/clash")),
            (from.join("docs/new"), into.join("docs/new")),
        ]
    );
}

#[test]
fn move_merge_removes_the_source_folder_when_nothing_is_left() {
    let (_root, from, into) = two_folders();
    write(&from.join("docs/new"), "new", 50);
    fs::create_dir_all(into.join("docs")).unwrap();
    let found = survey(transfer(Verb::Move, &[&from.join("docs")], &into)).unwrap();
    let (done, _) = run(found);
    assert!(done.failures.is_empty());
    assert!(!from.join("docs").exists());
    assert_eq!(names(&into.join("docs")), vec!["new"]);
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
    let first = start(
        survey(transfer(Verb::Copy, &[&from.join("a.txt")], &into)).unwrap(),
        || {},
    );
    let second = start(
        survey(transfer(Verb::Copy, &[&from.join("a.txt")], &other)).unwrap(),
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
    assert!(found.clashes.is_empty());
    assert_eq!(found.same_inside, 1);
}
