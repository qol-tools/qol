use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;

use super::{free_name, new_folder, rename, trash, NameProblem};
use crate::ops::{Done, JobId, Journal, Transfer, Undo, Verb};

fn done(verb: Verb, placed: Vec<(PathBuf, PathBuf)>) -> Done {
    Done {
        id: JobId(1),
        verb,
        landed: placed.clone(),
        matched: Vec::new(),
        trashed: false,
        placed,
        replaced: Vec::new(),
        kept_originals: Vec::new(),
        failures: Vec::new(),
    }
}

#[test]
fn free_name_returns_the_name_when_it_is_free() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        free_name(dir.path(), OsStr::new("a.md")),
        dir.path().join("a.md")
    );
}

#[test]
fn free_name_counts_up_before_the_extension() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.md"), "").unwrap();
    assert_eq!(
        free_name(dir.path(), OsStr::new("a.md")),
        dir.path().join("a (2).md")
    );
    fs::write(dir.path().join("a (2).md"), "").unwrap();
    assert_eq!(
        free_name(dir.path(), OsStr::new("a.md")),
        dir.path().join("a (3).md")
    );
}

#[test]
fn free_name_keeps_dots_in_folder_and_dotfile_names() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("v1.2")).unwrap();
    fs::write(dir.path().join(".bashrc"), "").unwrap();
    assert_eq!(
        free_name(dir.path(), OsStr::new("v1.2")),
        dir.path().join("v1.2 (2)")
    );
    assert_eq!(
        free_name(dir.path(), OsStr::new(".bashrc")),
        dir.path().join(".bashrc (2)")
    );
}

#[test]
fn new_folder_makes_it_and_refuses_a_taken_name() {
    let dir = tempfile::tempdir().unwrap();
    let made = new_folder(dir.path(), "plans").unwrap();
    assert!(made.is_dir());
    assert_eq!(new_folder(dir.path(), "plans"), Err(NameProblem::Taken));
}

#[test]
fn new_folder_refuses_an_empty_name() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(new_folder(dir.path(), "  "), Err(NameProblem::Empty));
}

#[test]
fn rename_moves_the_entry_to_the_new_name() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.md");
    fs::write(&file, "x").unwrap();
    let renamed = rename(&file, "b.md").unwrap();
    assert_eq!(renamed, dir.path().join("b.md"));
    assert!(!file.exists());
    assert_eq!(fs::read_to_string(renamed).unwrap(), "x");
}

#[test]
fn rename_to_the_same_name_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.md");
    fs::write(&file, "x").unwrap();
    assert_eq!(rename(&file, "a.md").unwrap(), file);
    assert!(file.exists());
}

#[test]
fn rename_never_overwrites_a_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.md");
    fs::write(&file, "a").unwrap();
    fs::write(dir.path().join("b.md"), "b").unwrap();
    assert_eq!(rename(&file, "b.md"), Err(NameProblem::Taken));
    assert_eq!(fs::read_to_string(dir.path().join("b.md")).unwrap(), "b");
    assert!(file.exists());
}

#[test]
fn rename_changes_only_the_case() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.md");
    fs::write(&file, "x").unwrap();
    let renamed = rename(&file, "NOTES.md").unwrap();
    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(names, vec![OsStr::new("NOTES.md")]);
    assert_eq!(fs::read_to_string(renamed).unwrap(), "x");
}

#[test]
fn rename_case_only_refuses_when_a_different_sibling_has_that_name() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.md");
    fs::write(&file, "a").unwrap();
    fs::write(dir.path().join("A.md"), "upper").unwrap();
    let both = fs::read_dir(dir.path()).unwrap().count() == 2;
    if both {
        assert_eq!(rename(&file, "A.md"), Err(NameProblem::Taken));
    }
}

#[test]
fn rename_reports_a_missing_entry_plainly() {
    let dir = tempfile::tempdir().unwrap();
    match rename(&dir.path().join("gone.md"), "b.md") {
        Err(NameProblem::Io(reason)) => assert_eq!(reason, "it is no longer there"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn trash_of_a_missing_path_fails_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("gone.md");
    let result = trash(std::slice::from_ref(&gone));
    assert!(result.placed.is_empty());
    assert_eq!(result.failures.len(), 1);
    assert_eq!(result.failures[0].path, gone);
    assert!(!result.failures[0].reason.is_empty());
}

#[test]
fn journal_undoes_a_copy_as_trash_of_the_copies() {
    let mut journal = Journal::default();
    journal.record(&done(
        Verb::Copy,
        vec![
            ("/a/x".into(), "/b/x".into()),
            ("/a/y".into(), "/b/y".into()),
        ],
    ));
    assert_eq!(
        journal.undo(),
        Some(Undo::Trash {
            copies: vec!["/b/x".into(), "/b/y".into()],
            restore: Vec::new(),
        })
    );
    assert_eq!(journal.undo(), None);
}

#[test]
fn journal_undoes_a_move_in_one_step_per_original_folder() {
    let mut journal = Journal::default();
    journal.record(&done(
        Verb::Move,
        vec![
            ("/a/x".into(), "/c/x".into()),
            ("/b/y".into(), "/c/y".into()),
        ],
    ));
    assert_eq!(
        journal.undo(),
        Some(Undo::Transfer {
            back: vec![
                Transfer {
                    verb: Verb::Move,
                    sources: vec!["/c/x".into()],
                    into: "/a".into(),
                },
                Transfer {
                    verb: Verb::Move,
                    sources: vec!["/c/y".into()],
                    into: "/b".into(),
                },
            ],
            renames: Vec::new(),
            restore: Vec::new(),
        })
    );
    assert_eq!(journal.undo(), None);
}

#[test]
fn journal_undo_of_a_keep_both_move_returns_the_original_name() {
    let mut journal = Journal::default();
    journal.record(&done(Verb::Move, vec![("/a/x".into(), "/c/x (2)".into())]));
    assert_eq!(
        journal.undo(),
        Some(Undo::Transfer {
            back: vec![Transfer {
                verb: Verb::Move,
                sources: vec!["/c/x (2)".into()],
                into: "/a".into(),
            }],
            renames: vec![("/a/x (2)".into(), "/a/x".into())],
            restore: Vec::new(),
        })
    );
}

#[test]
fn journal_undo_of_a_merging_move_copies_back_the_files_that_matched() {
    let mut journal = Journal::default();
    let mut moved = done(Verb::Move, Vec::new());
    moved.matched = vec![("/a/docs/same".into(), "/c/docs/same".into())];
    journal.record(&moved);
    assert_eq!(
        journal.undo(),
        Some(Undo::Transfer {
            back: vec![Transfer {
                verb: Verb::Copy,
                sources: vec!["/c/docs/same".into()],
                into: "/a/docs".into(),
            }],
            renames: Vec::new(),
            restore: Vec::new(),
        })
    );
}

#[test]
fn journal_undoes_a_move_into_a_folder_named_like_the_trash_as_a_move() {
    let mut journal = Journal::default();
    let into = "/home/me/Trash/files/x";
    journal.record(&done(Verb::Move, vec![("/a/x".into(), into.into())]));
    assert!(matches!(journal.undo(), Some(Undo::Transfer { .. })));
}

#[test]
fn journal_undo_of_a_copy_puts_back_what_it_replaced() {
    let mut journal = Journal::default();
    let mut copied = done(Verb::Copy, vec![("/a/x".into(), "/b/x".into())]);
    copied.replaced = vec!["/b/x".into()];
    journal.record(&copied);
    assert_eq!(
        journal.undo(),
        Some(Undo::Trash {
            copies: vec!["/b/x".into()],
            restore: vec!["/b/x".into()],
        })
    );
}

#[test]
fn journal_undoes_a_trash_whose_location_is_unknown_as_restore() {
    let mut journal = Journal::default();
    let mut trashed = done(Verb::Move, vec![("/a/x".into(), "/a/x".into())]);
    trashed.trashed = true;
    journal.record(&trashed);
    assert_eq!(journal.undo(), Some(Undo::Restore(vec!["/a/x".into()])));
}

#[test]
fn journal_undoes_a_rename_by_swapping_the_names() {
    let mut journal = Journal::default();
    journal.record_rename("/a/old".into(), "/a/new".into());
    assert_eq!(
        journal.undo(),
        Some(Undo::Rename {
            from: "/a/new".into(),
            to: "/a/old".into(),
        })
    );
}

#[test]
fn journal_removes_a_new_folder_only_while_it_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty");
    let filled = dir.path().join("filled");
    fs::create_dir(&empty).unwrap();
    fs::create_dir(&filled).unwrap();
    fs::write(filled.join("x"), "").unwrap();
    let mut journal = Journal::default();
    journal.record_new_folder(empty.clone());
    journal.record_new_folder(filled);
    assert_eq!(journal.undo(), None);
    assert_eq!(journal.undo(), Some(Undo::Remove(empty)));
}

#[test]
fn journal_ignores_a_job_that_placed_nothing() {
    let mut journal = Journal::default();
    journal.record(&done(Verb::Copy, Vec::new()));
    assert_eq!(journal.undo(), None);
}
