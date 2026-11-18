use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;
use tempfile::TempDir;

use super::trash_dirs::{shared_root, Root};
use super::trash_info::encoded;
use super::trasher::{candidate, Trasher};
use crate::ops::bin::{Bin, BinError};
use crate::ops::platform::{copy_link, free_bytes, name_problem, removable, volume_of};
use crate::ops::print::{Kind, Print};
use crate::ops::NameProblem;

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

fn yard() -> (TempDir, Trasher) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("work")).unwrap();
    let trasher = Trasher::new(Some(dir.path().join("home/Trash")));
    (dir, trasher)
}

fn work(dir: &TempDir, name: &str) -> PathBuf {
    dir.path().join("work").join(name)
}

#[test]
fn trashing_a_file_writes_the_item_and_its_info() {
    let (dir, trasher) = yard();
    let file = work(&dir, "a b.txt");
    fs::write(&file, "hello").unwrap();
    let before = Print::read(&file).unwrap();
    let held = trasher.trash(&file).unwrap().unwrap();
    let root = dir.path().join("home/Trash");
    let info = root.join("info/a b.txt.trashinfo");
    assert_eq!(held.item, root.join("files/a b.txt"));
    assert_eq!(held.sidecar.as_deref(), Some(info.as_path()));
    assert_eq!(held.before, before);
    assert!(!file.exists());
    assert_eq!(fs::read_to_string(&held.item).unwrap(), "hello");
    let text = fs::read_to_string(&info).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "[Trash Info]");
    let folder = encoded(&fs::canonicalize(dir.path().join("work")).unwrap());
    assert_eq!(lines[1], format!("Path={folder}/a%20b.txt"));
    let date = lines[2].strip_prefix("DeletionDate=").unwrap();
    assert!(NaiveDateTime::parse_from_str(date, "%Y-%m-%dT%H:%M:%S").is_ok());
}

#[test]
fn the_trash_folders_are_private() {
    let (dir, trasher) = yard();
    let file = work(&dir, "a");
    fs::write(&file, "").unwrap();
    trasher.trash(&file).unwrap();
    for folder in ["home/Trash", "home/Trash/files", "home/Trash/info"] {
        let mode = fs::metadata(dir.path().join(folder))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700, "{folder}");
    }
}

#[test]
fn restore_puts_it_back_and_removes_the_info() {
    let (dir, trasher) = yard();
    let file = work(&dir, "a.txt");
    fs::write(&file, "hello").unwrap();
    let held = trasher.trash(&file).unwrap().unwrap();
    trasher.restore(&held, &file).unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "hello");
    assert!(!held.item.exists());
    assert!(!held.sidecar.unwrap().exists());
}

#[test]
fn restore_refuses_a_taken_place_and_leaves_the_item() {
    let (dir, trasher) = yard();
    let file = work(&dir, "a.txt");
    fs::write(&file, "old").unwrap();
    let held = trasher.trash(&file).unwrap().unwrap();
    fs::write(&file, "new").unwrap();
    assert_eq!(trasher.restore(&held, &file), Err(BinError::Taken));
    assert_eq!(fs::read_to_string(&file).unwrap(), "new");
    assert_eq!(fs::read_to_string(&held.item).unwrap(), "old");
    assert!(held.sidecar.unwrap().exists());
}

#[test]
fn restore_notices_a_swapped_or_removed_item() {
    let (dir, trasher) = yard();
    let file = work(&dir, "a.txt");
    fs::write(&file, "old").unwrap();
    let held = trasher.trash(&file).unwrap().unwrap();
    let other = work(&dir, "other");
    fs::write(&other, "other").unwrap();
    fs::rename(&other, &held.item).unwrap();
    assert_eq!(trasher.restore(&held, &file), Err(BinError::Changed));
    assert!(!file.exists());
    assert_eq!(fs::read_to_string(&held.item).unwrap(), "other");
    fs::remove_file(&held.item).unwrap();
    assert_eq!(trasher.restore(&held, &file), Err(BinError::Gone));
}

#[test]
fn same_named_files_get_distinct_names_and_restore_to_their_own_place() {
    let (dir, trasher) = yard();
    fs::create_dir(work(&dir, "one")).unwrap();
    fs::create_dir(work(&dir, "two")).unwrap();
    let first = work(&dir, "one/x.txt");
    let second = work(&dir, "two/x.txt");
    fs::write(&first, "first").unwrap();
    fs::write(&second, "second").unwrap();
    let held_first = trasher.trash(&first).unwrap().unwrap();
    let held_second = trasher.trash(&second).unwrap().unwrap();
    assert_ne!(held_first.item, held_second.item);
    assert_ne!(held_first.sidecar, held_second.sidecar);
    trasher.restore(&held_second, &second).unwrap();
    trasher.restore(&held_first, &first).unwrap();
    assert_eq!(fs::read_to_string(&first).unwrap(), "first");
    assert_eq!(fs::read_to_string(&second).unwrap(), "second");
}

#[test]
fn a_long_name_still_fits_with_its_info_name() {
    let (dir, trasher) = yard();
    let file = work(&dir, &"n".repeat(250));
    let mut helds = Vec::new();
    for round in 0..2 {
        fs::write(&file, format!("long {round}")).unwrap();
        let held = trasher.trash(&file).unwrap().unwrap();
        let sidecar = held.sidecar.as_ref().unwrap();
        assert!(sidecar.file_name().unwrap().len() <= 255);
        assert!(held.item.file_name().unwrap().len() <= 245);
        helds.push(held);
    }
    assert_ne!(helds[0].item, helds[1].item);
    trasher.restore(&helds[1], &file).unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "long 1");
}

#[test]
fn names_are_numbered_and_shortened_on_character_boundaries() {
    assert_eq!(candidate(b"a.tar.gz", 1), "a.tar.gz");
    assert_eq!(candidate(b"a.tar.gz", 2), "a.tar 2.gz");
    assert_eq!(candidate(b"notes", 3), "notes 3");
    assert_eq!(candidate(b".bashrc", 2), ".bashrc 2");
    let wide = "\u{e9}".repeat(200);
    let name = candidate(wide.as_bytes(), 12);
    assert!(name.len() <= 245);
    assert!(name.to_str().is_some());
    assert!(name.as_bytes().ends_with(b" 12"));
}

#[test]
fn spaces_percent_and_non_ascii_are_percent_encoded() {
    assert_eq!(
        encoded(Path::new("/a b/%\u{e9}.txt")),
        "/a%20b/%25%C3%A9.txt"
    );
    assert_eq!(
        encoded(Path::new("/a-b_c.d~e!f*g'h(i)j")),
        "/a-b_c.d~e!f*g'h(i)j"
    );
    let (dir, trasher) = yard();
    let file = work(&dir, "\u{e9} %.txt");
    fs::write(&file, "").unwrap();
    let held = trasher.trash(&file).unwrap().unwrap();
    let text = fs::read_to_string(held.sidecar.unwrap()).unwrap();
    assert!(text.contains("/work/%C3%A9%20%25.txt\n"), "{text}");
}

#[test]
fn a_symlink_is_trashed_as_the_link() {
    let (dir, trasher) = yard();
    let target = work(&dir, "target");
    let link = work(&dir, "link");
    fs::write(&target, "kept").unwrap();
    symlink("target", &link).unwrap();
    let held = trasher.trash(&link).unwrap().unwrap();
    assert_eq!(held.before.kind, Kind::Link);
    assert!(fs::symlink_metadata(&held.item)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_to_string(&target).unwrap(), "kept");
    assert!(fs::symlink_metadata(&link).is_err());
    trasher.restore(&held, &link).unwrap();
    assert_eq!(fs::read_link(&link).unwrap(), Path::new("target"));
}

#[test]
fn a_folder_is_trashed_whole_and_restores_whole() {
    let (dir, trasher) = yard();
    let folder = work(&dir, "folder");
    fs::create_dir_all(folder.join("sub")).unwrap();
    fs::write(folder.join("sub/f.txt"), "inside").unwrap();
    let held = trasher.trash(&folder).unwrap().unwrap();
    assert_eq!(held.before.kind, Kind::Folder);
    assert!(!folder.exists());
    assert_eq!(
        fs::read_to_string(held.item.join("sub/f.txt")).unwrap(),
        "inside"
    );
    trasher.restore(&held, &folder).unwrap();
    assert_eq!(
        fs::read_to_string(folder.join("sub/f.txt")).unwrap(),
        "inside"
    );
}

#[test]
fn a_trash_that_cannot_be_made_leaves_the_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    fs::write(&blocker, "").unwrap();
    let file = dir.path().join("a");
    fs::write(&file, "kept").unwrap();
    let trasher = Trasher::new(Some(blocker.join("Trash")));
    assert!(matches!(trasher.trash(&file), Err(BinError::NoTrash(_))));
    assert_eq!(fs::read_to_string(&file).unwrap(), "kept");
}

#[test]
fn trashing_something_missing_fails_plainly() {
    let (dir, trasher) = yard();
    let result = trasher.trash(&work(&dir, "missing"));
    assert!(matches!(result, Err(BinError::Failed(_))));
}

#[test]
fn an_item_reached_through_a_link_or_dots_records_its_real_place() {
    let (dir, trasher) = yard();
    let real = work(&dir, "real");
    fs::create_dir(&real).unwrap();
    symlink(&real, work(&dir, "link")).unwrap();
    let place = fs::canonicalize(&real).unwrap();
    for (name, through) in [
        ("a.txt", work(&dir, "link/a.txt")),
        ("b.txt", work(&dir, "real/../real/b.txt")),
    ] {
        fs::write(real.join(name), "text").unwrap();
        let held = trasher.trash(&through).unwrap().unwrap();
        let text = fs::read_to_string(held.sidecar.unwrap()).unwrap();
        let path = format!("Path={}", encoded(&place.join(name)));
        assert!(text.lines().any(|line| line == path), "{text}");
        assert!(!real.join(name).exists());
    }
}

#[test]
fn a_drive_trash_records_the_path_from_the_top_of_the_drive() {
    let root = |top: Option<&str>| Root {
        files: PathBuf::new(),
        info: PathBuf::new(),
        top: top.map(PathBuf::from),
    };
    let drive = root(Some("/media/usb"));
    assert_eq!(
        drive.recorded(Path::new("/media/usb/dir/a.txt")),
        Path::new("dir/a.txt")
    );
    let home = root(None);
    assert_eq!(
        home.recorded(Path::new("/home/me/a.txt")),
        Path::new("/home/me/a.txt")
    );
}

#[test]
fn a_drive_trash_is_shared_only_when_the_shared_folder_is_sticky() {
    let dir = tempfile::tempdir().unwrap();
    let uid = unsafe { libc::geteuid() };
    let own = dir.path().join(format!(".Trash-{uid}"));
    assert_eq!(shared_root(dir.path()), own);
    let shared = dir.path().join(".Trash");
    fs::create_dir(&shared).unwrap();
    fs::set_permissions(&shared, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(shared_root(dir.path()), own);
    fs::set_permissions(&shared, fs::Permissions::from_mode(0o1777)).unwrap();
    assert_eq!(shared_root(dir.path()), shared.join(uid.to_string()));
}

#[test]
fn a_symlinked_shared_folder_is_not_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o1777)).unwrap();
    symlink(&real, dir.path().join(".Trash")).unwrap();
    let uid = unsafe { libc::geteuid() };
    assert_eq!(
        shared_root(dir.path()),
        dir.path().join(format!(".Trash-{uid}"))
    );
}
