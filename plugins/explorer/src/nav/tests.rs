use std::fs;
use std::path::{Path, PathBuf, MAIN_SEPARATOR_STR};
use std::time::{Duration, Instant};

use super::path::{self, PathInput};
use super::*;
use crate::listing::{self, Listing};

fn folders(names: &[&str]) -> Vec<PathBuf> {
    names
        .iter()
        .map(|name| Path::new("/root").join(name))
        .collect()
}

#[test]
fn history_restores_cursor_and_scroll_on_back_and_forward() {
    let mut history = History::new(PathBuf::from("/home"));
    history.remember(4, None, 120.0);
    history.go(PathBuf::from("/home/git"));
    history.remember(2, None, 36.0);

    let back = history.back().unwrap().clone();
    assert_eq!(
        back,
        Spot {
            folder: PathBuf::from("/home"),
            cursor: 4,
            entry: None,
            scroll: 120.0
        }
    );
    let forward = history.forward().unwrap().clone();
    assert_eq!(
        forward,
        Spot {
            folder: PathBuf::from("/home/git"),
            cursor: 2,
            entry: None,
            scroll: 36.0
        }
    );
    assert!(history.forward().is_none());
}

#[test]
fn going_somewhere_new_drops_the_forward_spots() {
    let mut history = History::new(PathBuf::from("/a"));
    history.go(PathBuf::from("/a/b"));
    history.back();
    assert!(history.can_go_forward());

    history.go(PathBuf::from("/a/c"));

    assert!(!history.can_go_forward());
    assert_eq!(history.back().unwrap().folder, PathBuf::from("/a"));
    assert!(history.back().is_none());
    assert!(!history.can_go_back());
}

#[test]
fn going_back_to_a_known_folder_recalls_its_cursor_and_scroll() {
    let mut history = History::new(PathBuf::from("/home"));
    history.remember(7, None, 252.0);
    history.go(PathBuf::from("/home/git"));

    let recalled = history.go(PathBuf::from("/home")).clone();
    assert_eq!(recalled.folder, PathBuf::from("/home"));
    assert_eq!(recalled.cursor, 7);
    assert_eq!(recalled.scroll, 252.0);
    assert!(history.can_go_back());
}

#[test]
fn a_new_folder_starts_at_the_top() {
    let mut history = History::new(PathBuf::from("/"));
    let spot = history.go(PathBuf::from("/srv")).clone();
    assert_eq!(spot, Spot::at(PathBuf::from("/srv")));
}

#[test]
fn going_to_the_current_folder_adds_no_spot() {
    let mut history = History::new(PathBuf::from("/a"));
    history.go(PathBuf::from("/a"));
    assert!(!history.can_go_back());
}

#[test]
fn picks_survive_navigation() {
    let mut history = History::new(PathBuf::from("/root"));
    let mut picks = Picks::new();
    picks.toggle(Path::new("/root/a"));
    history.go(PathBuf::from("/root/sub"));
    picks.add(Path::new("/root/sub/b"));
    history.back();

    assert!(picks.is_picked(Path::new("/root/a")));
    assert!(picks.is_picked(Path::new("/root/sub/b")));
    assert_eq!(picks.len(), 2);
}

#[test]
fn toggle_add_all_and_clear() {
    let rows = folders(&["a", "b", "c"]);
    let mut picks = Picks::new();

    picks.toggle(&rows[0]);
    assert!(picks.is_picked(&rows[0]));
    picks.toggle(&rows[0]);
    assert!(picks.is_empty());

    picks.add(&rows[1]);
    picks.add(&rows[1]);
    assert_eq!(picks.len(), 1);

    picks.all(&rows);
    assert_eq!(picks.len(), 3);
    assert!(picks.paths().all(|path| rows.iter().any(|row| row == path)));

    picks.clear();
    assert!(picks.is_empty());
}

#[test]
fn range_picks_grow_and_shrink_from_the_anchor() {
    let rows = folders(&["a", "b", "c", "d", "e"]);
    let mut picks = Picks::new();
    picks.add(&rows[4]);

    picks.extend_range(&rows, 1, 2);
    picks.extend_range(&rows, 2, 3);
    assert_eq!(picked(&picks, &rows), [false, true, true, true, true]);

    picks.extend_range(&rows, 3, 2);
    assert_eq!(picked(&picks, &rows), [false, true, true, false, true]);

    picks.extend_range(&rows, 2, 0);
    assert_eq!(picked(&picks, &rows), [true, true, false, false, true]);
}

#[test]
fn a_new_range_starts_from_the_current_picks() {
    let rows = folders(&["a", "b", "c", "d"]);
    let mut picks = Picks::new();
    picks.extend_range(&rows, 0, 1);
    picks.end_range();

    picks.extend_range(&rows, 3, 2);
    assert_eq!(picked(&picks, &rows), [true, true, true, true]);
}

#[test]
fn picks_are_counted_per_folder_through_every_change() {
    let rows = folders(&["a", "b", "c"]);
    let here = rows[0].parent().unwrap();
    let elsewhere = Path::new("/elsewhere");
    let mut picks = Picks::new();
    picks.add(&elsewhere.join("x"));
    picks.all(&rows);
    picks.add(&rows[0]);
    assert_eq!(picks.count_in(here), 3);
    assert_eq!(picks.count_in(elsewhere), 1);

    assert_eq!(picks.folder_count(), 2);

    picks.toggle(&rows[0]);
    picks.rename(&rows[1], elsewhere.join("y"));
    assert_eq!(picks.count_in(here), 1);
    assert_eq!(picks.count_in(elsewhere), 2);
    picks.toggle(&rows[2]);
    assert_eq!(picks.count_in(here), 0);
    assert_eq!(picks.folder_count(), 1);

    picks.extend_range(&rows, 0, 2);
    assert_eq!(picks.count_in(here), 3);
    assert_eq!(picks.folder_count(), 2);
    picks.retain(|path| path.starts_with(here));
    assert_eq!(picks.count_in(elsewhere), 0);
    assert_eq!(picks.folder_count(), 1);
    picks.clear();
    assert_eq!(picks.count_in(here), 0);
    assert_eq!(picks.folder_count(), 0);
}

fn merged_set(items: &[&'static str]) -> std::collections::HashSet<&'static str> {
    items.iter().copied().collect()
}

#[test]
fn a_box_adds_to_the_picks_it_started_with_or_replaces_them() {
    let cases = [
        (&["a", "b"][..], &["b", "c"][..], true, &["a", "b", "c"][..]),
        (&["a", "b"], &["b", "c"], false, &["b", "c"]),
        (&["a", "b"], &[], true, &["a", "b"]),
        (&["a", "b"], &[], false, &[]),
        (&[], &["x"], true, &["x"]),
        (&[], &["x"], false, &["x"]),
        (&[], &[], true, &[]),
        (&["a"], &["a", "a"], false, &["a"]),
        (&["a", "a"], &["b"], true, &["a", "b"]),
    ];
    for (base, touched, additive, want) in cases {
        let got = merged(base.iter().copied(), touched.iter().copied(), additive);
        assert_eq!(got, merged_set(want), "{base:?} {touched:?} {additive}");
    }
}

fn picked(picks: &Picks, rows: &[PathBuf]) -> Vec<bool> {
    rows.iter().map(|row| picks.is_picked(row)).collect()
}

const NAMES: [&str; 6] = ["apple", "Avocado", "banana", "Berry", "blue", "cherry"];

fn type_run(jump: &mut Jump, typed: &str, start: Instant, cursor: &mut usize) -> Vec<usize> {
    typed
        .chars()
        .enumerate()
        .map(|(index, typed)| {
            let now = start + Duration::from_millis(100 * index as u64);
            if let Some(next) = jump.type_char(typed, now, &NAMES, *cursor, |name| *name) {
                *cursor = next;
            }
            *cursor
        })
        .collect()
}

#[test]
fn jump_goes_to_the_first_name_starting_with_the_run_ignoring_case() {
    let cases = [
        ("a", 0, vec![0]),
        ("av", 0, vec![0, 1]),
        ("BL", 0, vec![2, 4]),
        ("ch", 3, vec![5, 5]),
        ("z", 3, vec![3]),
    ];
    for (typed, start_cursor, expected) in cases {
        let mut jump = Jump::new();
        let mut cursor = start_cursor;
        let cursors = type_run(&mut jump, typed, Instant::now(), &mut cursor);
        assert_eq!(cursors, expected, "typed: {typed}");
    }
}

#[test]
fn jump_matches_accented_names_by_their_plain_letters() {
    let names = ["eagle", "\u{e9}clair", "E\u{301}mile", "empty"];
    for (typed, expected) in [("ec", 1), ("\u{e9}c", 1), ("em", 2), ("emp", 3)] {
        let mut jump = Jump::new();
        let now = Instant::now();
        let mut found = None;
        for letter in typed.chars() {
            found = jump.type_char(letter, now, &names, 0, |name| *name);
        }
        assert_eq!(found, Some(expected), "typed: {typed}");
    }
}

#[test]
fn a_repeated_letter_cycles_through_names_with_that_letter_and_wraps() {
    let mut jump = Jump::new();
    let mut cursor = 5;
    let cursors = type_run(&mut jump, "bbbb", Instant::now(), &mut cursor);
    assert_eq!(cursors, [2, 3, 4, 2]);
}

#[test]
fn a_repeated_letter_prefers_a_name_matching_the_whole_run() {
    let names = ["aardvark", "apple", "avocado"];
    let mut jump = Jump::new();
    let start = Instant::now();
    assert_eq!(jump.type_char('a', start, &names, 2, |name| *name), Some(0));
    assert_eq!(
        jump.type_char('a', start + Duration::from_millis(50), &names, 0, |name| {
            *name
        }),
        Some(0)
    );
    assert_eq!(
        jump.type_char('a', start + Duration::from_millis(90), &names, 0, |name| {
            *name
        }),
        Some(1)
    );
}

#[test]
fn the_run_resets_after_one_second_without_typing() {
    let mut jump = Jump::new();
    let start = Instant::now();
    assert_eq!(jump.type_char('b', start, &NAMES, 0, |name| *name), Some(2));

    let just_in_time = start + RUN_RESET - Duration::from_millis(1);
    assert_eq!(
        jump.type_char('e', just_in_time, &NAMES, 2, |name| *name),
        Some(3)
    );
    assert_eq!(jump.run(just_in_time), "be");

    let idle = just_in_time + RUN_RESET;
    assert_eq!(jump.run(idle), "");
    assert_eq!(jump.type_char('c', idle, &NAMES, 3, |name| *name), Some(5));
    assert_eq!(jump.run(idle), "c");
}

#[test]
fn a_letter_that_matches_nothing_is_left_out_of_the_run() {
    let mut jump = Jump::new();
    let start = Instant::now();
    assert_eq!(jump.type_char('b', start, &NAMES, 0, |name| *name), Some(2));
    assert_eq!(jump.type_char('x', start, &NAMES, 2, |name| *name), None);
    assert_eq!(jump.run(start), "b");

    let mut jump = Jump::new();
    let mut cursor = 3;
    assert_eq!(type_run(&mut jump, "xa", start, &mut cursor), [3, 0]);
    assert_eq!(type_run(&mut jump, "v", start, &mut cursor), [1]);
}

#[test]
fn a_space_continues_a_run_only_when_a_name_continues_with_it() {
    let names = ["my file", "my other", "notes"];
    let mut jump = Jump::new();
    let now = Instant::now();
    assert!(!jump.continues(' ', now, &names, |name| *name));
    for typed in "my".chars() {
        jump.type_char(typed, now, &names, 0, |name| *name);
    }
    assert!(jump.continues(' ', now, &names, |name| *name));
    assert_eq!(jump.type_char(' ', now, &names, 0, |name| *name), Some(0));
    assert_eq!(jump.type_char('o', now, &names, 0, |name| *name), Some(1));
    assert!(!jump.continues(' ', now, &names, |name| *name));
    assert!(!jump.continues(' ', now + RUN_RESET, &names, |name| *name));
}

#[test]
fn a_typed_accent_matches_only_that_accent() {
    let names = ["alpha", "\u{c4}pfel", "\u{e4}rger.txt", "apple.txt"];
    let cases = [
        ("\u{e4}", 1),
        ("a\u{308}", 1),
        ("a", 0),
        ("ar", 2),
        ("\u{e4}p", 1),
        ("ap", 1),
        ("app", 3),
    ];
    for (typed, expected) in cases {
        let mut jump = Jump::new();
        let now = Instant::now();
        let mut found = None;
        for letter in typed.chars() {
            found = jump
                .type_char(letter, now, &names, 0, |name| *name)
                .or(found);
        }
        assert_eq!(found, Some(expected), "typed: {typed}");
    }
}

#[test]
fn jump_in_an_empty_list_finds_nothing() {
    let mut jump = Jump::new();
    let empty: [&str; 0] = [];
    let start = Instant::now();
    assert_eq!(jump.type_char('a', start, &empty, 0, |name| *name), None);
    assert_eq!(jump.type_char('a', start, &empty, 0, |name| *name), None);
}

#[test]
fn reset_clears_the_run() {
    let mut jump = Jump::new();
    let start = Instant::now();
    jump.type_char('a', start, &NAMES, 0, |name| *name);
    jump.reset();
    assert_eq!(jump.run(start), "");
}

fn sep(text: &str) -> String {
    text.replace('/', MAIN_SEPARATOR_STR)
}

#[test]
fn tilde_expands_to_home() {
    let home = Path::new("/home/me");
    let cases = [
        ("~", home.to_path_buf()),
        ("~/", home.to_path_buf()),
        ("~/Git", home.join("Git")),
        ("~/Git/qol", home.join("Git").join("qol")),
        ("~other", PathBuf::from("~other")),
        ("/srv", PathBuf::from("/srv")),
    ];
    for (input, expected) in cases {
        assert_eq!(
            path::expand_home(&sep(input), home),
            expected,
            "input: {input}"
        );
    }
}

#[test]
fn relative_paths_resolve_against_the_current_folder() {
    let home = Path::new("/home/me");
    let cwd = Path::new("/work");
    assert_eq!(path::resolve("docs", home, cwd), cwd.join("docs"));
    assert_eq!(path::resolve("~", home, cwd), home.to_path_buf());
}

#[test]
fn dots_collapse_when_a_path_resolves() {
    let home = Path::new("/home/me");
    let cwd = Path::new("/work/a");
    let cases = [
        ("..", PathBuf::from("/work")),
        ("../b", PathBuf::from("/work/b")),
        (".", PathBuf::from("/work/a")),
        ("./c/../d", PathBuf::from("/work/a/d")),
        ("~/..", PathBuf::from("/home")),
        ("/..", PathBuf::from("/")),
    ];
    for (input, expected) in cases {
        assert_eq!(
            path::resolve(&sep(input), home, cwd),
            expected,
            "input: {input}"
        );
    }
}

#[test]
fn home_is_shown_as_tilde() {
    let home = Path::new("/home/me");
    assert_eq!(path::display_with_home(home, home), "~");
    assert_eq!(
        path::display_with_home(&home.join("Git"), home),
        sep("~/Git")
    );
    assert_eq!(path::display_with_home(Path::new("/srv"), home), "/srv");
}

#[test]
fn the_child_toward_a_folder_is_the_next_step_down() {
    let home = Path::new("/home/me");
    assert_eq!(
        path::child_toward(home, &home.join("Git").join("qol")),
        Some(home.join("Git"))
    );
    assert_eq!(path::child_toward(home, home), None);
    assert_eq!(path::child_toward(home, Path::new("/srv")), None);
}

#[test]
fn path_input_splits_into_parent_and_last_segment() {
    let home = Path::new("/home/me");
    let cases = [
        ("~/Git/qo", Some(home.join("Git")), "qo"),
        ("~/Git/", Some(home.join("Git")), ""),
        ("~/", Some(home.to_path_buf()), ""),
        ("~", None, "~"),
        ("/srv/da", Some(PathBuf::from("/srv/")), "da"),
        ("loose", None, "loose"),
    ];
    for (input, parent, segment) in cases {
        assert_eq!(
            PathInput::parse(&sep(input), home),
            PathInput {
                parent,
                segment: segment.to_string()
            },
            "input: {input}"
        );
    }
}

fn read_listing(folder: &Path) -> Listing {
    let read = listing::start(folder.to_path_buf(), listing::Pass::NamesOnly, || {});
    let mut listing = Listing::new(folder.to_path_buf());
    while let Ok(event) = read.events().recv() {
        listing.apply(event);
    }
    listing
}

#[test]
fn suggestions_are_folders_starting_with_the_segment() {
    let temp = tempfile::tempdir().unwrap();
    for folder in ["Documents", "Downloads", "desk", "Music"] {
        fs::create_dir(temp.path().join(folder)).unwrap();
    }
    fs::write(temp.path().join("Docs.txt"), "").unwrap();
    let listing = read_listing(temp.path());

    assert_eq!(
        path::suggestions(&listing, "do", false),
        ["Documents", "Downloads"]
    );
    assert_eq!(path::suggestions(&listing, "DE", false), ["desk"]);
    assert_eq!(
        path::suggestions(&listing, "", false),
        ["desk", "Documents", "Downloads", "Music"]
    );
    assert!(path::suggestions(&listing, "x", true).is_empty());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn hidden_folders_are_suggested_only_when_shown() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join(".config")).unwrap();
    let listing = read_listing(temp.path());

    assert!(path::suggestions(&listing, "", false).is_empty());
    assert_eq!(path::suggestions(&listing, "", true), [".config"]);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn a_dot_segment_suggests_hidden_folders() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join(".config")).unwrap();
    let listing = read_listing(temp.path());

    assert_eq!(path::suggestions(&listing, ".c", false), [".config"]);
}

#[cfg(target_os = "linux")]
#[test]
fn folders_whose_names_are_not_text_are_not_suggested() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join(OsStr::from_bytes(b"caf\xe9"))).unwrap();
    fs::create_dir(temp.path().join("cafe")).unwrap();
    let listing = read_listing(temp.path());

    assert_eq!(path::suggestions(&listing, "caf", false), ["cafe"]);
}

#[cfg(target_os = "linux")]
#[test]
fn folders_whose_names_show_a_mark_are_not_suggested() {
    let temp = tempfile::tempdir().unwrap();
    for folder in ["dir\nnl", "dir\u{1b}", "dir\u{202E}txt", "dirt"] {
        fs::create_dir(temp.path().join(folder)).unwrap();
    }
    let listing = read_listing(temp.path());

    assert_eq!(path::suggestions(&listing, "dir", false), ["dirt"]);
}

#[test]
fn a_name_contains_the_typed_text_anywhere_ignoring_case_and_accents() {
    let cases = [
        ("Report final.txt", "final", true),
        ("Report final.txt", "REPORT", true),
        ("Report final.txt", "t f", true),
        ("R\u{e9}sum\u{e9}.md", "resume", true),
        ("Resume.md", "r\u{e9}sum\u{e9}", false),
        ("R\u{e9}sum\u{e9}.md", "r\u{e9}s", true),
        ("notes.txt", "", true),
        ("notes.txt", "notes.txt.bak", false),
        ("notes.txt", "xt", true),
        ("notes.txt", "xn", false),
        ("", "a", false),
        ("", "", true),
    ];
    for (name, typed, expected) in cases {
        assert_eq!(
            contains_ignoring_case(name, typed),
            expected,
            "{name:?} {typed:?}"
        );
    }
}
