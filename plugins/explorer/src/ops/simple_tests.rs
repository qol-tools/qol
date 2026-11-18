use std::ffi::OsStr;
use std::fs;
use std::io;

use super::{
    copy_name, delete, free_name, move_noreplace, move_through_scratch, new_file, new_folder,
    rename, trash, NameProblem,
};
use crate::ops::{Bin, FolderBin};

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
fn rename_never_replaces_a_folder_with_a_folder() {
    let dir = tempfile::tempdir().unwrap();
    let moved = dir.path().join("a");
    let taken = dir.path().join("b");
    fs::create_dir(&moved).unwrap();
    fs::create_dir(&taken).unwrap();
    fs::write(taken.join("keep.md"), "kept").unwrap();
    assert_eq!(rename(&moved, "b"), Err(NameProblem::Taken));
    assert_eq!(fs::read_to_string(taken.join("keep.md")).unwrap(), "kept");
    assert!(moved.exists());
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
fn trash_hands_back_what_the_bin_holds_and_reports_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let bin_dir = tempfile::tempdir().unwrap();
    let bin = FolderBin::new(bin_dir.path().join("bin"));
    let file = dir.path().join("a.md");
    fs::write(&file, "x").unwrap();
    let gone = dir.path().join("gone.md");
    let result = trash(&bin, &[file.clone(), gone.clone()]);
    assert_eq!(result.failures.len(), 1);
    assert_eq!(result.failures[0].path, gone);
    assert_eq!(result.failures[0].reason, "it is no longer there");
    assert_eq!(result.trashed.len(), 1);
    let (path, held) = &result.trashed[0];
    assert_eq!(path, &file);
    let held = held.clone().unwrap();
    assert!(!file.exists());
    assert!(held.item.starts_with(bin.root()));
    assert_eq!(fs::read_to_string(&held.item).unwrap(), "x");
    bin.restore(&held, &file).unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "x");
}

#[test]
fn trash_of_a_missing_path_fails_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let bin_dir = tempfile::tempdir().unwrap();
    let bin = FolderBin::new(bin_dir.path().join("bin"));
    let gone = dir.path().join("gone.md");
    let result = trash(&bin, std::slice::from_ref(&gone));
    assert!(result.trashed.is_empty());
    assert_eq!(result.failures.len(), 1);
    assert_eq!(result.failures[0].path, gone);
    assert!(!result.failures[0].reason.is_empty());
}

#[test]
fn move_noreplace_moves_onto_a_free_name() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("a.md");
    let to = dir.path().join("b.md");
    fs::write(&from, "a").unwrap();
    move_noreplace(&from, &to).unwrap();
    assert!(!from.exists());
    assert_eq!(fs::read_to_string(&to).unwrap(), "a");
}

#[test]
fn move_noreplace_refuses_to_replace_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("a.md");
    let to = dir.path().join("b.md");
    fs::write(&from, "a").unwrap();
    fs::write(&to, "b").unwrap();
    let error = move_noreplace(&from, &to).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&from).unwrap(), "a");
    assert_eq!(fs::read_to_string(&to).unwrap(), "b");
}

#[test]
fn move_noreplace_refuses_to_replace_a_folder() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("a");
    let to = dir.path().join("b");
    fs::create_dir(&from).unwrap();
    fs::create_dir(&to).unwrap();
    fs::write(to.join("keep.md"), "kept").unwrap();
    let error = move_noreplace(&from, &to).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert!(from.is_dir());
    assert_eq!(fs::read_to_string(to.join("keep.md")).unwrap(), "kept");
}

#[test]
fn move_noreplace_with_a_case_only_change_keeps_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("notes.md");
    let to = dir.path().join("NOTES.md");
    fs::write(&from, "x").unwrap();
    move_noreplace(&from, &to).unwrap();
    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(names, vec![OsStr::new("NOTES.md")]);
    assert_eq!(fs::read_to_string(&to).unwrap(), "x");
}

#[cfg(unix)]
#[test]
fn move_noreplace_with_a_case_only_change_is_still_the_same_file() {
    use std::os::unix::fs::MetadataExt;
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("notes.md");
    let to = dir.path().join("NOTES.md");
    fs::write(&from, "x").unwrap();
    let index = fs::metadata(&from).unwrap().ino();
    move_noreplace(&from, &to).unwrap();
    assert_eq!(fs::metadata(&to).unwrap().ino(), index);
}

#[test]
fn move_noreplace_with_a_case_only_change_never_replaces_a_different_entry() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("a.md");
    let to = dir.path().join("A.md");
    fs::write(&from, "a").unwrap();
    fs::write(&to, "upper").unwrap();
    if fs::read_dir(dir.path()).unwrap().count() == 2 {
        let error = move_noreplace(&from, &to).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&from).unwrap(), "a");
        assert_eq!(fs::read_to_string(&to).unwrap(), "upper");
    }
}

#[test]
fn the_two_leg_move_puts_the_entry_back_when_the_second_leg_fails() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("a.md");
    let to = dir.path().join("b.md");
    fs::write(&from, "a").unwrap();
    fs::write(&to, "b").unwrap();
    let error = move_through_scratch(&from, &to).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&from).unwrap(), "a");
    assert_eq!(fs::read_to_string(&to).unwrap(), "b");
    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(names.len(), 2);
}

#[test]
fn the_two_leg_move_leaves_an_occupied_scratch_name_alone() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("a.md");
    let to = dir.path().join("A.md");
    let scratch = dir.path().join(".a.md.qol-rename");
    fs::write(&from, "a").unwrap();
    fs::write(&scratch, "mine").unwrap();
    move_through_scratch(&from, &to).unwrap();
    assert_eq!(fs::read_to_string(&to).unwrap(), "a");
    assert_eq!(fs::read_to_string(&scratch).unwrap(), "mine");
    assert!(!dir.path().join(".a.md.qol-rename (2)").exists());
}

#[test]
fn copy_name_adds_copy_before_the_extension_then_counts_up() {
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        ("a.md", false, "a copy.md"),
        ("a.tar.gz", false, "a.tar copy.gz"),
        (".bashrc", false, ".bashrc copy"),
        ("v1.2", true, "v1.2 copy"),
        ("notes", false, "notes copy"),
    ];
    for (name, folder, expected) in cases {
        assert_eq!(
            copy_name(dir.path(), OsStr::new(name), folder),
            dir.path().join(expected),
            "{name}"
        );
    }
    fs::write(dir.path().join("a copy.md"), "").unwrap();
    assert_eq!(
        copy_name(dir.path(), OsStr::new("a.md"), false),
        dir.path().join("a copy 2.md")
    );
    fs::write(dir.path().join("a copy 2.md"), "").unwrap();
    assert_eq!(
        copy_name(dir.path(), OsStr::new("a.md"), false),
        dir.path().join("a copy 3.md")
    );
}

#[test]
fn new_file_creates_an_empty_file_and_refuses_a_taken_or_bad_name() {
    let dir = tempfile::tempdir().unwrap();
    let made = new_file(dir.path(), "New file").unwrap();
    assert!(made.is_file());
    assert_eq!(fs::metadata(&made).unwrap().len(), 0);
    assert_eq!(new_file(dir.path(), "New file"), Err(NameProblem::Taken));
    assert_eq!(new_file(dir.path(), ""), Err(NameProblem::Empty));
}

#[test]
fn delete_removes_files_and_folders_and_reports_what_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    fs::write(&file, "a").unwrap();
    let folder = dir.path().join("d");
    fs::create_dir_all(folder.join("inner")).unwrap();
    fs::write(folder.join("inner").join("b"), "b").unwrap();
    let missing = dir.path().join("missing");

    let failures = delete(&[file.clone(), folder.clone(), missing.clone()]);
    assert!(!file.exists());
    assert!(!folder.exists());
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].path, missing);
}
