use std::fs;
use std::path::Path;

use super::*;
use crate::ops::platform::{copy_link, free_bytes, name_problem, removable, volume_of};
use crate::ops::NameProblem;
use std::os::unix::fs::symlink;

#[test]
fn name_rules_reject_empty_slash_null_dots_and_long_names() {
    assert_eq!(name_problem(""), Some(NameProblem::Empty));
    assert_eq!(name_problem("   "), Some(NameProblem::Empty));
    assert!(matches!(
        name_problem("a/b"),
        Some(NameProblem::Forbidden(_))
    ));
    assert!(matches!(
        name_problem("a\0b"),
        Some(NameProblem::Forbidden(_))
    ));
    assert!(matches!(
        name_problem(".."),
        Some(NameProblem::Forbidden(_))
    ));
    assert_eq!(name_problem(&"x".repeat(256)), Some(NameProblem::TooLong));
}

#[test]
fn name_rules_allow_what_linux_allows() {
    assert_eq!(name_problem("a:b*c?.txt"), None);
    assert_eq!(name_problem("CON"), None);
    assert_eq!(name_problem("trailing. "), None);
    assert_eq!(name_problem(&"x".repeat(255)), None);
}

#[test]
fn free_bytes_reports_room_on_a_real_folder() {
    let dir = tempfile::tempdir().unwrap();
    assert!(free_bytes(dir.path()).unwrap() > 0);
    assert!(free_bytes(&dir.path().join("missing")).is_err());
}

#[test]
fn volume_of_matches_inside_one_folder() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a");
    fs::write(&file, "").unwrap();
    assert_eq!(volume_of(dir.path()).unwrap(), volume_of(&file).unwrap());
    assert!(volume_of(&dir.path().join("missing")).is_err());
}

#[test]
fn removable_accepts_a_file_in_a_writable_folder() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a");
    fs::write(&file, "").unwrap();
    assert_eq!(removable(&file), Ok(()));
}

#[test]
fn removable_names_the_owner_when_the_folder_is_not_ours() {
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let reason = removable(Path::new("/usr/bin/env")).unwrap_err();
    assert!(reason.starts_with("belongs to the user root"), "{reason}");
}

#[test]
fn copy_link_points_where_the_original_points() {
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("link");
    let copy = dir.path().join("copy");
    symlink("target", &link).unwrap();
    copy_link(&link, &copy).unwrap();
    assert_eq!(fs::read_link(copy).unwrap(), Path::new("target"));
}

#[test]
fn in_trash_recognises_freedesktop_trash_files() {
    assert!(in_trash(Path::new(
        "/home/me/.local/share/Trash/files/a.md"
    )));
    assert!(in_trash(Path::new("/mnt/data/.Trash-1000/files/a.md")));
    assert!(!in_trash(Path::new("/home/me/files/a.md")));
}

#[test]
fn locate_maps_a_trashinfo_to_its_file() {
    let item = TrashItem {
        id: "/home/me/.local/share/Trash/info/a.tar.gz.trashinfo".into(),
        name: "a.tar.gz".into(),
        original_parent: "/home/me".into(),
        time_deleted: 0,
    };
    assert_eq!(
        locate(&item),
        Path::new("/home/me/.local/share/Trash/files/a.tar.gz")
    );
}
