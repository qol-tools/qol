use std::fs;
use std::path::{Path, PathBuf, MAIN_SEPARATOR};
use std::thread;
use std::time::{Duration, Instant};

use gpui::{Keystroke, Modifiers};

use super::{Browser, Drained, Opening, PathStep};
use crate::listing::ListingState;
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
        settled.scroll = drained.scroll.or(settled.scroll);
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

    assert!(browser.clear_picks());
    assert!(!browser.clear_picks());
    assert_eq!(browser.note().count, "4 items");
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
    assert_eq!(browser.cursor(), 3);
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
    assert_eq!(browser.path_edit().unwrap().text(), "~");
    type_text(&mut browser, &format!("{MAIN_SEPARATOR}G"));
    settle(&mut browser);
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
    type_text(&mut browser, &format!("{MAIN_SEPARATOR}file.txt"));
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
        format!("~{MAIN_SEPARATOR}inner")
    );
    browser.path_key(&key("backspace", None), || None, 0.0);
    settle(&mut browser);
    assert_eq!(browser.path_edit().unwrap().suggestions(), ["inner"]);
    assert_eq!(browser.path_edit().unwrap().selected(), Some(0));
    for _ in 0.."inne".len() {
        browser.path_key(&key("backspace", None), || None, 0.0);
    }
    assert_eq!(browser.path_edit().unwrap().suggestions(), ["inner"]);
    assert_eq!(browser.path_edit().unwrap().selected(), None);
    let step = browser.path_key(&key("enter", None), || None, 0.0);
    assert_eq!(step, PathStep::Arrived);
    assert_eq!(browser.folder(), temp.path());
}

#[test]
fn escape_in_the_path_field_puts_the_trail_back() {
    let temp = abc_folder();
    let mut browser = opened(temp.path());
    browser.edit_path();
    type_text(&mut browser, "zzz");
    let step = browser.path_key(&key("escape", None), || None, 0.0);
    assert_eq!(step, PathStep::Closed);
    assert!(!browser.path_open());
    assert_eq!(browser.folder(), temp.path());
}
