use std::fs;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use gpui::{Keystroke, Modifiers};

use super::counts::LiveCounts;
use super::{Browser, Drained, Opening, PathStep, REFRESH_QUIET, STALLED_AFTER};
use crate::listing::{ItemCount, ListingState};
use crate::ui::keys::Step;

fn touch(folder: &Path, name: &str) {
    fs::write(folder.join(name), name).unwrap();
}

fn make_folder(folder: &Path, name: &str) -> PathBuf {
    let path = folder.join(name);
    fs::create_dir(&path).unwrap();
    path
}

fn settle(browser: &mut Browser) -> Drained {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut settled = Drained::default();
    while browser.reading() {
        let drained = browser.drain(Instant::now());
        settled.changed |= drained.changed;
        if drained.scroll.is_some() {
            settled.scroll = drained.scroll;
            settled.cursor_shift = drained.cursor_shift;
        }
        assert!(Instant::now() < deadline, "the read never finished");
        thread::sleep(Duration::from_millis(2));
    }
    settled
}

fn wait_until(browser: &mut Browser, what: &str, done: impl Fn(&Browser) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let drained = browser.drain(Instant::now());
        if done(browser) {
            return;
        }
        assert!(Instant::now() < deadline, "{what} never happened");
        thread::sleep(drained.wake_in.unwrap_or(Duration::from_millis(2)));
    }
}

fn opened(folder: &Path) -> Browser {
    let mut browser = Browser::new(folder.to_path_buf(), folder.to_path_buf());
    settle(&mut browser);
    browser
}

fn names(browser: &Browser) -> Vec<String> {
    (0..browser.row_count())
        .map(|row| browser.entry(row).unwrap().display.clone())
        .collect()
}

fn picked(browser: &Browser) -> Vec<String> {
    (0..browser.row_count())
        .map(|row| browser.entry(row).unwrap())
        .filter(|entry| browser.is_picked(entry))
        .map(|entry| entry.display.clone())
        .collect()
}

fn key(name: &str, key_char: Option<&str>) -> Keystroke {
    Keystroke {
        modifiers: Modifiers::none(),
        key: name.to_string(),
        key_char: key_char.map(str::to_string),
    }
}

fn type_text(browser: &mut Browser, text: &str) {
    for typed in text.chars() {
        let typed = typed.to_string();
        browser.path_key(&key(&typed, Some(&typed)), || None, 0.0);
    }
}

fn abc_folder() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        touch(temp.path(), name);
    }
    temp
}

#[test]
fn a_folder_that_sends_nothing_for_a_while_counts_as_stalled() {
    let temp = abc_folder();
    let mut browser = Browser::new(temp.path().to_path_buf(), temp.path().to_path_buf());
    let now = Instant::now();
    assert!(browser.arriving());
    assert!(!browser.stalled(now));
    assert!(browser.stalled(now + STALLED_AFTER));

    settle(&mut browser);
    assert!(!browser.stalled(Instant::now() + STALLED_AFTER));
}

#[test]
fn the_cursor_moves_one_row_and_stops_at_both_ends() {
    let temp = abc_folder();
    let mut browser = opened(temp.path());
    assert_eq!(browser.cursor(), 0);
    assert!(!browser.move_cursor(Step::Up, 1));
    for expected in [1, 2, 3] {
        assert!(browser.move_cursor(Step::Down, 1));
        assert_eq!(browser.cursor(), expected);
    }
    assert!(!browser.move_cursor(Step::Down, 1));
    assert!(browser.move_cursor(Step::Up, 1));
    assert_eq!(browser.cursor(), 2);
}

#[test]
fn page_and_edge_steps_jump_the_cursor_and_stop_at_the_ends() {
    let temp = abc_folder();
    let mut browser = opened(temp.path());
    assert!(browser.move_cursor(Step::PageDown, 3));
    assert_eq!(browser.cursor(), 3);
    assert!(!browser.move_cursor(Step::PageDown, 3));
    assert!(browser.move_cursor(Step::PageUp, 2));
    assert_eq!(browser.cursor(), 1);
    assert!(browser.move_cursor(Step::PageUp, 9));
    assert_eq!(browser.cursor(), 0);
    assert!(browser.move_cursor(Step::End, 1));
    assert_eq!(browser.cursor(), 3);
    assert!(browser.move_cursor(Step::Home, 1));
    assert_eq!(browser.cursor(), 0);

    browser.move_cursor(Step::Down, 1);
    assert!(browser.extend_range(Step::End, 1));
    assert_eq!(picked(&browser), ["b.txt", "c.txt", "d.txt"]);
    assert!(browser.extend_range(Step::PageUp, 1));
    assert_eq!(picked(&browser), ["b.txt", "c.txt"]);
}

#[test]
fn files_that_appear_or_vanish_on_disk_show_up_and_the_cursor_keeps_its_entry() {
    let temp = abc_folder();
    let mut browser = opened(temp.path());
    browser.set_cursor(2);

    touch(temp.path(), "e.txt");
    wait_until(&mut browser, "the new file", |browser| {
        browser.row_count() == 5
    });
    assert_eq!(
        names(&browser),
        ["a.txt", "b.txt", "c.txt", "d.txt", "e.txt"]
    );
    assert_eq!(browser.entry(browser.cursor()).unwrap().display, "c.txt");

    fs::remove_file(temp.path().join("a.txt")).unwrap();
    wait_until(&mut browser, "the removal", |browser| {
        browser.row_count() == 4
    });
    assert_eq!(browser.cursor(), 1);
    assert_eq!(browser.entry(1).unwrap().display, "c.txt");
    assert_eq!(browser.note().count, "4 items");
}

#[test]
fn a_folder_that_vanishes_while_shown_reports_it() {
    let temp = tempfile::tempdir().unwrap();
    let inner = make_folder(temp.path(), "inner");
    touch(&inner, "x.txt");
    let mut browser = opened(&inner);

    fs::remove_dir_all(&inner).unwrap();
    wait_until(&mut browser, "the failure", |browser| {
        matches!(browser.listing().state(), ListingState::Failed(_))
    });
    assert_eq!(browser.row_count(), 0);
}

#[cfg(unix)]
#[test]
fn a_folder_shown_through_a_link_reports_the_link_removed() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let real = make_folder(temp.path(), "real");
    touch(&real, "x.txt");
    let link = temp.path().join("link");
    symlink(&real, &link).unwrap();
    let mut browser = opened(&link);

    fs::remove_file(&link).unwrap();
    wait_until(&mut browser, "the failure", |browser| {
        matches!(browser.listing().state(), ListingState::Failed(_))
    });
}

#[cfg(unix)]
#[test]
fn a_folder_shown_through_a_link_follows_the_link_to_a_new_target() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let real = make_folder(temp.path(), "real");
    touch(&real, "x.txt");
    let other = make_folder(temp.path(), "other");
    touch(&other, "y.txt");
    let link = temp.path().join("link");
    symlink(&real, &link).unwrap();
    let mut browser = opened(&link);

    fs::remove_file(&link).unwrap();
    symlink(&other, &link).unwrap();
    wait_until(&mut browser, "the new target", |browser| {
        names(browser) == ["y.txt"]
    });

    touch(&other, "z.txt");
    wait_until(&mut browser, "a file in the new target", |browser| {
        browser.row_count() == 2
    });
}

#[test]
fn a_folder_recreated_while_shown_is_followed_again() {
    let temp = tempfile::tempdir().unwrap();
    let inner = make_folder(temp.path(), "inner");
    touch(&inner, "x.txt");
    let mut browser = opened(&inner);

    fs::remove_dir_all(&inner).unwrap();
    fs::create_dir(&inner).unwrap();
    touch(&inner, "y.txt");
    wait_until(&mut browser, "the new folder", |browser| {
        names(browser) == ["y.txt"]
    });

    touch(&inner, "z.txt");
    wait_until(&mut browser, "a file in the new folder", |browser| {
        browser.row_count() == 2
    });
}

#[test]
fn a_folder_recreated_long_after_it_vanished_is_followed_again() {
    let temp = tempfile::tempdir().unwrap();
    let inner = make_folder(temp.path(), "inner");
    touch(&inner, "x.txt");
    let mut browser = opened(&inner);

    fs::remove_dir_all(&inner).unwrap();
    wait_until(&mut browser, "the failure", |browser| {
        matches!(browser.listing().state(), ListingState::Failed(_))
    });
    fs::create_dir(&inner).unwrap();
    touch(&inner, "y.txt");
    wait_until(&mut browser, "the new folder", |browser| {
        names(browser) == ["y.txt"]
    });

    touch(&inner, "z.txt");
    wait_until(&mut browser, "a file in the new folder", |browser| {
        browser.row_count() == 2
    });
}

#[test]
fn picks_of_entries_that_vanish_are_dropped_and_picks_elsewhere_stay() {
    let temp = abc_folder();
    let inner = make_folder(temp.path(), "inner");
    touch(&inner, "x.txt");
    let mut browser = opened(&inner);
    browser.toggle_pick();
    browser.up(0.0);
    settle(&mut browser);
    browser.set_cursor(1);
    browser.toggle_pick();
    browser.set_cursor(2);
    browser.toggle_pick();
    assert_eq!(browser.note().count, "3 picked");

    fs::remove_file(temp.path().join("a.txt")).unwrap();
    wait_until(&mut browser, "the removal", |browser| {
        browser.row_count() == 4
    });
    assert_eq!(picked(&browser), ["b.txt"]);
    assert_eq!(browser.note().count, "2 picked");

    fs::remove_file(inner.join("x.txt")).unwrap();
    assert!(browser.go(inner.clone(), 0.0));
    settle(&mut browser);
    assert_eq!(browser.note().count, "1 picked");
}

#[test]
fn a_shift_run_keeps_its_start_when_rows_appear_above_it() {
    let temp = abc_folder();
    let mut browser = opened(temp.path());
    browser.set_cursor(1);
    browser.extend_range(Step::Down, 1);
    assert_eq!(picked(&browser), ["b.txt", "c.txt"]);

    touch(temp.path(), "0.txt");
    wait_until(&mut browser, "the new file", |browser| {
        browser.row_count() == 5
    });
    browser.extend_range(Step::Down, 1);
    assert_eq!(picked(&browser), ["b.txt", "c.txt", "d.txt"]);
}

#[test]
fn coming_back_after_rows_above_the_cursor_vanished_lands_on_the_same_entry() {
    let temp = abc_folder();
    let inner = make_folder(temp.path(), "inner");
    let mut browser = opened(temp.path());
    browser.set_cursor(3);
    assert_eq!(browser.entry(3).unwrap().display, "c.txt");

    browser.go(inner, 108.0);
    settle(&mut browser);
    fs::remove_file(temp.path().join("a.txt")).unwrap();
    fs::remove_file(temp.path().join("b.txt")).unwrap();
    browser.back(0.0);
    let back = settle(&mut browser);
    assert_eq!(browser.entry(browser.cursor()).unwrap().display, "c.txt");
    assert_eq!(back.scroll, Some(108.0));
    assert_eq!(back.cursor_shift, -2);
}

#[test]
fn a_cursor_step_ends_the_typed_run() {
    let temp = abc_folder();
    let mut browser = opened(temp.path());
    let now = Instant::now();
    browser.jump('b', now);
    assert_eq!(browser.jump_run(now), "b");
    browser.move_cursor(Step::Down, 1);
    assert_eq!(browser.jump_run(now), "");
}

#[test]
fn shift_steps_pick_a_run_from_where_the_run_started() {
    let temp = abc_folder();
    let mut browser = opened(temp.path());
    browser.move_cursor(Step::Down, 1);
    browser.extend_range(Step::Down, 1);
    browser.extend_range(Step::Down, 1);
    assert_eq!(picked(&browser), ["b.txt", "c.txt", "d.txt"]);
    assert_eq!(browser.cursor(), 3);

    browser.extend_range(Step::Up, 1);
    assert_eq!(picked(&browser), ["b.txt", "c.txt"]);

    browser.end_range();
    browser.move_cursor(Step::Up, 1);
    browser.move_cursor(Step::Up, 1);
    browser.extend_range(Step::Up, 1);
    assert_eq!(picked(&browser), ["a.txt", "b.txt", "c.txt"]);
}

#[test]
fn space_toggles_ctrl_click_adds_ctrl_a_picks_all_and_esc_clears() {
    let temp = abc_folder();
    let mut browser = opened(temp.path());
    assert!(browser.toggle_pick());
    assert_eq!(picked(&browser), ["a.txt"]);
    assert!(browser.toggle_pick());
    assert!(picked(&browser).is_empty());

    assert!(browser.add_pick(2));
    assert!(browser.add_pick(2));
    assert_eq!(picked(&browser), ["c.txt"]);
    assert_eq!(browser.cursor(), 0);

    browser.pick_all();
    assert_eq!(picked(&browser).len(), 4);
    assert_eq!(browser.note().count, "4 picked");

    assert!(browser.dismiss());
    assert!(!browser.dismiss());
    assert_eq!(browser.note().count, "4 items");
}

#[test]
fn escape_and_the_next_file_opened_drop_the_red_note() {
    let temp = abc_folder();
    let mut browser = opened(temp.path());

    browser.fail("Could not open a.txt".into());
    assert!(browser.dismiss());
    assert_eq!(browser.note().failure, None);
    assert!(!browser.dismiss());

    browser.fail("Could not open a.txt".into());
    browser.set_cursor(1);
    assert_eq!(browser.open(0.0), Opening::File(temp.path().join("b.txt")));
    assert_eq!(browser.note().failure, None);
}

#[test]
fn leaving_and_coming_back_restores_the_cursor_and_the_scroll() {
    let temp = abc_folder();
    let inner = make_folder(temp.path(), "inner");
    touch(&inner, "x.txt");
    let mut browser = opened(temp.path());
    browser.set_cursor(3);

    assert!(browser.go(inner.clone(), 72.0));
    let arrived = settle(&mut browser);
    assert_eq!(browser.folder(), inner);
    assert_eq!(browser.cursor(), 0);
    assert_eq!(arrived.scroll, Some(0.0));

    assert!(browser.back(0.0));
    assert_eq!(browser.cursor(), 0, "the cursor waits for the listing");
    let back = settle(&mut browser);
    assert_eq!(browser.cursor(), 3);
    assert_eq!(back.scroll, Some(72.0));

    assert!(browser.forward(36.0));
    settle(&mut browser);
    assert_eq!(browser.folder(), inner);
    assert!(browser.up(0.0));
    let up = settle(&mut browser);
    assert_eq!(browser.folder(), temp.path());
    assert_eq!(names(&browser)[browser.cursor()], "inner");
    assert_eq!(up.scroll, Some(36.0));
}

#[test]
fn leaving_a_folder_before_it_loads_keeps_its_saved_spot() {
    let temp = abc_folder();
    let inner = make_folder(temp.path(), "inner");
    let mut browser = opened(temp.path());
    browser.set_cursor(3);
    browser.go(inner, 36.0);
    settle(&mut browser);

    assert!(browser.back(0.0));
    assert!(browser.forward(0.0));
    settle(&mut browser);
    assert!(browser.back(0.0));
    let back = settle(&mut browser);

    assert_eq!(browser.cursor(), 3);
    assert_eq!(back.scroll, Some(36.0));
}

#[test]
fn going_somewhere_closes_the_path_field() {
    let temp = abc_folder();
    let inner = make_folder(temp.path(), "inner");
    let mut browser = opened(temp.path());
    assert!(browser.edit_path());

    assert!(browser.go(inner, 0.0));
    assert!(!browser.path_open());
}

#[test]
fn going_to_the_current_folder_does_nothing() {
    let temp = abc_folder();
    let mut browser = opened(temp.path());
    assert!(!browser.go(temp.path().to_path_buf(), 0.0));
    assert!(!browser.reading());
}

#[test]
fn enter_opens_a_folder_here_and_hands_a_file_to_the_caller() {
    let temp = abc_folder();
    let inner = make_folder(temp.path(), "inner");
    let mut browser = opened(temp.path());
    assert_eq!(names(&browser)[0], "inner");
    browser.set_cursor(1);
    assert_eq!(browser.open(0.0), Opening::File(temp.path().join("a.txt")));
    assert_eq!(browser.folder(), temp.path());

    browser.set_cursor(0);
    assert_eq!(browser.open(0.0), Opening::Folder);
    assert_eq!(browser.folder(), inner);
    settle(&mut browser);
    assert_eq!(browser.open(0.0), Opening::Nothing);
}

#[test]
fn hidden_files_come_and_go_and_the_cursor_stays_on_its_entry() {
    let temp = abc_folder();
    touch(temp.path(), ".hidden");
    let mut browser = opened(temp.path());
    assert_eq!(names(&browser), ["a.txt", "b.txt", "c.txt", "d.txt"]);
    browser.set_cursor(2);

    browser.toggle_hidden();
    assert_eq!(browser.row_count(), 5);
    assert_eq!(browser.entry(browser.cursor()).unwrap().display, "c.txt");

    browser.toggle_hidden();
    assert_eq!(browser.row_count(), 4);
    assert_eq!(browser.entry(browser.cursor()).unwrap().display, "c.txt");
}

#[test]
fn typing_jumps_to_the_first_name_that_starts_with_the_run() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["apple", "banana", "Blueberry", "cherry"] {
        touch(temp.path(), name);
    }
    let mut browser = opened(temp.path());
    let now = Instant::now();
    browser.jump('b', now);
    assert_eq!(browser.cursor(), 1);
    browser.jump('l', now);
    assert_eq!(browser.cursor(), 2);
    assert_eq!(browser.jump_run(now), "bl");
    assert_eq!(browser.jump_run(now + Duration::from_secs(1)), "");

    browser.jump('C', now + Duration::from_secs(2));
    assert_eq!(browser.cursor(), 3);
}

#[test]
fn a_folder_that_cannot_be_read_settles_at_the_top() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");
    let mut browser = Browser::new(missing, temp.path().to_path_buf());
    let settled = settle(&mut browser);
    assert_eq!(settled.scroll, Some(0.0));
    assert_eq!(browser.row_count(), 0);
    assert_eq!(browser.note().count, "");
}

#[test]
fn the_path_field_suggests_the_folders_of_the_typed_parent() {
    let temp = tempfile::tempdir().unwrap();
    make_folder(temp.path(), "Games");
    make_folder(temp.path(), "Git");
    make_folder(temp.path(), "Music");
    touch(temp.path(), "Gnotes.txt");
    let mut browser = opened(temp.path());

    assert!(browser.edit_path());
    assert!(!browser.edit_path());
    assert_eq!(
        browser.path_edit().unwrap().text(),
        format!("~{MAIN_SEPARATOR}")
    );
    settle(&mut browser);
    assert_eq!(
        browser.path_edit().unwrap().suggestions(),
        ["Games", "Git", "Music"]
    );
    assert_eq!(browser.path_edit().unwrap().selected(), None);
    type_text(&mut browser, "G");
    assert_eq!(browser.path_edit().unwrap().suggestions(), ["Games", "Git"]);

    type_text(&mut browser, "i");
    assert_eq!(browser.path_edit().unwrap().suggestions(), ["Git"]);
}

#[test]
fn submitting_a_folder_goes_there_and_anything_else_stays_in_the_field() {
    let temp = tempfile::tempdir().unwrap();
    let inner = make_folder(temp.path(), "inner");
    touch(temp.path(), "file.txt");
    let mut browser = opened(temp.path());

    browser.edit_path();
    type_text(&mut browser, "file.txt");
    let step = browser.path_key(&key("enter", None), || None, 0.0);
    assert_eq!(step, PathStep::Stay { changed: true });
    assert!(browser.path_open());
    let failure = browser.note().failure.unwrap();
    assert!(failure.ends_with("is not a folder"), "{failure}");

    type_text(&mut browser, "x");
    assert_eq!(browser.note().failure, None);
    browser.path_key(&key("enter", None), || None, 0.0);
    assert!(browser.note().failure.unwrap().ends_with("does not exist"));

    for _ in 0.."file.txtx".len() {
        browser.path_key(&key("backspace", None), || None, 0.0);
    }
    type_text(&mut browser, "inner");
    let step = browser.path_key(&key("enter", None), || None, 0.0);
    assert_eq!(step, PathStep::Arrived);
    assert!(!browser.path_open());
    assert_eq!(browser.folder(), inner);
}

#[test]
fn backspacing_to_the_folder_level_makes_enter_go_to_the_typed_folder() {
    let temp = tempfile::tempdir().unwrap();
    let inner = make_folder(temp.path(), "inner");
    let mut browser = opened(temp.path());
    browser.go(inner, 0.0);
    settle(&mut browser);

    browser.edit_path();
    assert_eq!(
        browser.path_edit().unwrap().text(),
        format!("~{MAIN_SEPARATOR}inner{MAIN_SEPARATOR}")
    );
    browser.path_key(&key("backspace", None), || None, 0.0);
    settle(&mut browser);
    assert_eq!(browser.path_edit().unwrap().suggestions(), ["inner"]);
    assert_eq!(browser.path_edit().unwrap().selected(), Some(0));
    for _ in 0.."inner".len() {
        browser.path_key(&key("backspace", None), || None, 0.0);
    }
    assert_eq!(browser.path_edit().unwrap().suggestions(), ["inner"]);
    assert_eq!(browser.path_edit().unwrap().selected(), None);
    let step = browser.path_key(&key("enter", None), || None, 0.0);
    assert_eq!(step, PathStep::Arrived);
    assert_eq!(browser.folder(), temp.path());
}

#[cfg(unix)]
#[test]
fn dots_through_a_link_suggest_the_folder_that_enter_opens() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let a = make_folder(temp.path(), "a");
    make_folder(&a, "b");
    symlink(a.join("b"), temp.path().join("link")).unwrap();
    let mut browser = opened(temp.path());

    browser.edit_path();
    type_text(
        &mut browser,
        &format!("link{MAIN_SEPARATOR}..{MAIN_SEPARATOR}"),
    );
    settle(&mut browser);
    assert_eq!(browser.path_edit().unwrap().suggestions(), ["a", "link"]);

    type_text(&mut browser, "a");
    let step = browser.path_key(&key("enter", None), || None, 0.0);
    assert_eq!(step, PathStep::Arrived);
    assert_eq!(browser.folder(), a);
}

#[test]
fn a_folder_made_while_the_path_field_is_open_is_suggested() {
    let temp = tempfile::tempdir().unwrap();
    let inner = make_folder(temp.path(), "inner");
    make_folder(&inner, "dirt");
    let mut browser = opened(temp.path());

    browser.edit_path();
    type_text(&mut browser, &format!("inner{MAIN_SEPARATOR}d"));
    settle(&mut browser);
    assert_eq!(browser.path_edit().unwrap().suggestions(), ["dirt"]);

    make_folder(&inner, "dirq");
    wait_until(&mut browser, "the new folder to be suggested", |browser| {
        browser.path_edit().unwrap().suggestions() == ["dirq", "dirt"]
    });
}

#[test]
fn a_typed_parent_deleted_and_made_again_is_suggested_from_again() {
    let temp = tempfile::tempdir().unwrap();
    let alpha = make_folder(temp.path(), "alpha");
    make_folder(&alpha, "aa");
    let mut browser = opened(temp.path());

    browser.edit_path();
    type_text(&mut browser, &format!("alpha{MAIN_SEPARATOR}"));
    settle(&mut browser);
    assert_eq!(browser.path_edit().unwrap().suggestions(), ["aa"]);

    fs::remove_dir_all(&alpha).unwrap();
    wait_until(&mut browser, "the menu to empty", |browser| {
        browser.path_edit().unwrap().suggestions().is_empty()
    });
    make_folder(temp.path(), "alpha");
    make_folder(&alpha, "newdir");
    wait_until(&mut browser, "the new folder to be suggested", |browser| {
        browser.path_edit().unwrap().suggestions() == ["newdir"]
    });
}

#[test]
fn a_typed_parent_made_after_it_was_typed_is_suggested_from() {
    let temp = tempfile::tempdir().unwrap();
    let mut browser = opened(temp.path());

    browser.edit_path();
    type_text(&mut browser, &format!("later{MAIN_SEPARATOR}"));
    settle(&mut browser);
    assert!(browser.path_edit().unwrap().suggestions().is_empty());

    let later = make_folder(temp.path(), "later");
    make_folder(&later, "kid");
    wait_until(&mut browser, "the new folder to be suggested", |browser| {
        browser.path_edit().unwrap().suggestions() == ["kid"]
    });
}

#[cfg(target_os = "linux")]
#[test]
fn the_path_field_reaches_a_folder_whose_name_shows_a_mark() {
    let temp = tempfile::tempdir().unwrap();
    let odd = make_folder(temp.path(), "dir\nnl");
    let sub = make_folder(&odd, "sub");
    let mut browser = opened(temp.path());
    browser.go(odd.clone(), 0.0);
    settle(&mut browser);

    browser.edit_path();
    assert_eq!(
        browser.path_edit().unwrap().text(),
        format!("~{MAIN_SEPARATOR}dir\u{FFFD}nl{MAIN_SEPARATOR}")
    );
    settle(&mut browser);
    assert_eq!(browser.path_edit().unwrap().suggestions(), ["sub"]);
    let step = browser.path_key(&key("enter", None), || None, 0.0);
    assert_eq!(step, PathStep::Closed);
    assert_eq!(browser.note().failure, None);
    assert_eq!(browser.folder(), odd);

    browser.edit_path();
    settle(&mut browser);
    type_text(&mut browser, "s");
    let step = browser.path_key(&key("enter", None), || None, 0.0);
    assert_eq!(step, PathStep::Arrived);
    assert_eq!(browser.folder(), sub);
}

#[test]
fn escape_in_the_path_field_puts_the_trail_back() {
    let temp = abc_folder();
    let mut browser = opened(temp.path());
    browser.edit_path();
    type_text(&mut browser, "zzz");
    browser.path_key(&key("enter", None), || None, 0.0);
    assert!(browser.note().failure.is_some());
    let step = browser.path_key(&key("escape", None), || None, 0.0);
    assert_eq!(step, PathStep::Closed);
    assert!(!browser.path_open());
    assert_eq!(browser.note().failure, None);
    assert_eq!(browser.folder(), temp.path());
}

#[test]
fn going_up_or_to_an_ancestor_lands_on_the_folder_left() {
    let temp = abc_folder();
    make_folder(temp.path(), "early");
    let inner = make_folder(temp.path(), "inner");
    make_folder(&inner, "before");
    let deeper = make_folder(&inner, "deeper");
    let mut browser = Browser::new(deeper, temp.path().to_path_buf());
    settle(&mut browser);

    assert!(browser.up(0.0));
    settle(&mut browser);
    assert_eq!(browser.folder(), inner);
    assert_eq!(names(&browser)[browser.cursor()], "deeper");

    assert!(browser.go(temp.path().to_path_buf(), 0.0));
    settle(&mut browser);
    assert_eq!(names(&browser)[browser.cursor()], "inner");
}

#[test]
fn a_refresh_reports_how_far_the_cursor_row_moved() {
    let temp = abc_folder();
    let mut browser = opened(temp.path());
    browser.set_cursor(3);
    make_folder(temp.path(), "x");
    make_folder(temp.path(), "y");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut shift = 0;
    while browser.row_count() != 6 {
        let drained = browser.drain(Instant::now());
        shift += drained.cursor_shift;
        assert!(Instant::now() < deadline, "the folders never showed");
        thread::sleep(drained.wake_in.unwrap_or(Duration::from_millis(2)));
    }
    assert_eq!(names(&browser)[browser.cursor()], "d.txt");
    assert_eq!(shift, 2);
}

#[test]
fn the_count_waits_for_the_listing() {
    let temp = abc_folder();
    let mut browser = Browser::new(temp.path().to_path_buf(), temp.path().to_path_buf());
    assert_eq!(browser.note().count, "");
    settle(&mut browser);
    assert_eq!(browser.note().count, "4 items");
}

#[test]
fn picking_a_suggestion_goes_there() {
    let temp = tempfile::tempdir().unwrap();
    make_folder(temp.path(), "Games");
    let git = make_folder(temp.path(), "Git");
    let mut browser = opened(temp.path());
    browser.edit_path();
    settle(&mut browser);
    assert_eq!(browser.path_edit().unwrap().suggestions(), ["Games", "Git"]);
    assert_eq!(
        browser.pick_suggestion(5, 0.0),
        PathStep::Stay { changed: false }
    );
    assert_eq!(browser.pick_suggestion(1, 0.0), PathStep::Arrived);
    assert_eq!(browser.folder(), git);
    assert!(!browser.path_open());
}

#[cfg(unix)]
#[test]
fn a_linked_folder_count_follows_files_added_inside_its_target() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let real = make_folder(temp.path(), "real");
    let link = temp.path().join("link");
    symlink(&real, &link).unwrap();
    let mut browser = opened(temp.path());
    wait_until(&mut browser, "the count watch", Browser::follows_counts);

    touch(&real, "one");
    let one = Some(ItemCount::Items { all: 1, shown: 1 });
    wait_until(&mut browser, "both counts", |browser| {
        browser.listing().item_count(&real) == one && browser.listing().item_count(&link) == one
    });
}

#[test]
fn a_subfolder_changed_before_its_watch_is_up_is_counted_again() {
    let temp = tempfile::tempdir().unwrap();
    let sub = make_folder(temp.path(), "sub");
    let counted_from = SystemTime::now();
    touch(&sub, "late");
    let mut counts = LiveCounts::default();
    counts.follow(vec![sub.clone()], counted_from, || {});

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut now = Instant::now();
    loop {
        counts.recount_when_due(now, temp.path(), || {});
        let recounted = counts.drain().into_iter().find(|count| count.path == sub);
        if let Some(recounted) = recounted {
            assert_eq!(recounted.count, ItemCount::Items { all: 1, shown: 1 });
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the changed subfolder was not recounted"
        );
        thread::sleep(Duration::from_millis(10));
        now += REFRESH_QUIET;
    }
}

#[test]
fn a_subfolder_is_recounted_again_only_after_its_last_recount_reports() {
    let temp = tempfile::tempdir().unwrap();
    let sub = make_folder(temp.path(), "sub");
    let counted_from = SystemTime::now();
    touch(&sub, "late");
    let mut counts = LiveCounts::default();
    counts.follow(vec![sub.clone()], counted_from, || {});
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut now = Instant::now();
    let mut due = |counts: &mut LiveCounts| {
        now += REFRESH_QUIET;
        counts.recount_when_due(now, temp.path(), || {});
        assert!(Instant::now() < deadline, "the recounts did not settle");
        thread::sleep(Duration::from_millis(10));
    };
    while counts.recounting() == 0 {
        due(&mut counts);
    }

    touch(&sub, "later");
    while !counts.is_stale(&sub) {
        due(&mut counts);
    }
    due(&mut counts);
    due(&mut counts);
    assert_eq!(counts.recounting(), 1);
    assert!(counts.is_stale(&sub));

    while !counts.drain().iter().any(|count| count.path == sub) {
        due(&mut counts);
    }
    let two = ItemCount::Items { all: 2, shown: 2 };
    while !counts
        .drain()
        .iter()
        .any(|count| count.path == sub && count.count == two)
    {
        due(&mut counts);
    }
}

#[cfg(unix)]
#[test]
fn a_renamed_hard_link_keeps_the_cursor_and_its_pick_off_its_twin() {
    let temp = abc_folder();
    fs::hard_link(temp.path().join("a.txt"), temp.path().join("e.txt")).unwrap();
    let mut browser = opened(temp.path());
    browser.set_cursor(4);
    browser.toggle_pick();

    fs::rename(temp.path().join("e.txt"), temp.path().join("f.txt")).unwrap();
    wait_until(&mut browser, "the rename", |browser| {
        names(browser) == ["a.txt", "b.txt", "c.txt", "d.txt", "f.txt"]
    });
    assert_eq!(browser.cursor(), 4);
    assert_eq!(picked(&browser), ["f.txt"]);
}

#[cfg(unix)]
#[test]
fn two_picked_hard_links_renamed_together_both_stay_picked() {
    let temp = abc_folder();
    fs::hard_link(temp.path().join("a.txt"), temp.path().join("e.txt")).unwrap();
    let mut browser = opened(temp.path());
    browser.toggle_pick();
    browser.set_cursor(4);
    browser.toggle_pick();

    fs::rename(temp.path().join("a.txt"), temp.path().join("x.txt")).unwrap();
    fs::rename(temp.path().join("e.txt"), temp.path().join("y.txt")).unwrap();
    wait_until(&mut browser, "both renames", |browser| {
        names(browser) == ["b.txt", "c.txt", "d.txt", "x.txt", "y.txt"]
    });
    assert_eq!(picked(&browser), ["x.txt", "y.txt"]);
}

#[cfg(unix)]
#[test]
fn a_picked_hard_link_renamed_with_its_unpicked_twin_stays_picked() {
    let temp = abc_folder();
    fs::hard_link(temp.path().join("a.txt"), temp.path().join("e.txt")).unwrap();
    let mut browser = opened(temp.path());
    browser.toggle_pick();

    fs::rename(temp.path().join("a.txt"), temp.path().join("x.txt")).unwrap();
    fs::rename(temp.path().join("e.txt"), temp.path().join("y.txt")).unwrap();
    wait_until(&mut browser, "both renames", |browser| {
        names(browser) == ["b.txt", "c.txt", "d.txt", "x.txt", "y.txt"]
    });
    assert_eq!(picked(&browser), ["x.txt"]);
}

#[cfg(unix)]
#[test]
fn a_picked_hard_link_renamed_as_its_twin_is_removed_stays_picked() {
    let temp = abc_folder();
    fs::hard_link(temp.path().join("a.txt"), temp.path().join("e.txt")).unwrap();
    let mut browser = opened(temp.path());
    browser.toggle_pick();

    fs::remove_file(temp.path().join("e.txt")).unwrap();
    fs::rename(temp.path().join("a.txt"), temp.path().join("x.txt")).unwrap();
    wait_until(&mut browser, "the rename", |browser| {
        names(browser) == ["b.txt", "c.txt", "d.txt", "x.txt"]
    });
    assert_eq!(picked(&browser), ["x.txt"]);
}

#[cfg(unix)]
#[test]
fn a_renamed_entry_keeps_the_cursor_and_its_pick() {
    let temp = abc_folder();
    let mut browser = opened(temp.path());
    browser.set_cursor(1);
    browser.toggle_pick();
    browser.set_cursor(2);

    fs::rename(temp.path().join("c.txt"), temp.path().join("0.txt")).unwrap();
    fs::rename(temp.path().join("b.txt"), temp.path().join("z.txt")).unwrap();
    wait_until(&mut browser, "both renames", |browser| {
        names(browser) == ["0.txt", "a.txt", "d.txt", "z.txt"]
    });
    assert_eq!(browser.cursor(), 0);
    assert_eq!(picked(&browser), ["z.txt"]);

    fs::rename(temp.path().join("0.txt"), temp.path().join("c.txt")).unwrap();
    wait_until(&mut browser, "the rename back", |browser| {
        names(browser) == ["a.txt", "c.txt", "d.txt", "z.txt"]
    });
    assert_eq!(browser.cursor(), 1);
}

#[cfg(unix)]
#[test]
fn a_link_count_follows_the_folder_the_link_was_pointed_at_since() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let old = make_folder(temp.path(), "old");
    touch(&old, "one");
    let new = make_folder(temp.path(), "new");
    touch(&new, "one");
    touch(&new, "two");
    let link = temp.path().join("link");
    symlink(&old, &link).unwrap();
    let mut browser = opened(temp.path());
    let count = |all| Some(ItemCount::Items { all, shown: all });
    wait_until(&mut browser, "the count watch", Browser::follows_counts);

    fs::remove_file(&link).unwrap();
    symlink(&new, &link).unwrap();
    wait_until(&mut browser, "the count of the new target", |browser| {
        browser.listing().item_count(&link) == count(2)
    });
    wait_until(&mut browser, "the count watch", Browser::follows_counts);
    touch(&new, "three");
    wait_until(&mut browser, "the count after a file appeared", |browser| {
        browser.listing().item_count(&link) == count(3)
    });
}

#[test]
fn a_subfolder_count_follows_files_added_and_removed_inside_it() {
    let temp = tempfile::tempdir().unwrap();
    let sub = make_folder(temp.path(), "sub");
    touch(&sub, "one");
    let mut browser = opened(temp.path());
    let count = |all, shown| Some(ItemCount::Items { all, shown });
    assert_eq!(browser.listing().item_count(&sub), count(1, 1));
    wait_until(&mut browser, "the count watch", Browser::follows_counts);

    touch(&sub, "two");
    touch(&sub, ".three");
    wait_until(&mut browser, "the count after files appeared", |browser| {
        browser.listing().item_count(&sub) == count(3, 2)
    });
    fs::remove_file(sub.join("one")).unwrap();
    wait_until(&mut browser, "the count after a file vanished", |browser| {
        browser.listing().item_count(&sub) == count(2, 1)
    });
}
