use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use gpui::{Keystroke, Modifiers};
use qol_gpui::text_edit::MemoryClipboard;

use super::ops_text;
use super::{Browser, Held, MoveState, Stepping};
use crate::ops::history::{Action, Change, Direction, History, Outcome, Reason, Source, Undone};
use crate::ops::print::{Kind, Print};
use crate::ops::{Clash, CopyOnly, Failure, Resolve, Room, Stamp, Survey, Tools, Transfer, Verb};
use crate::relay::{Level, Message};
use crate::ui::keys::Step;

thread_local! {
    static BIN: RefCell<PathBuf> = const { RefCell::new(PathBuf::new()) };
}

fn tools() -> Tools {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let root = ROOT.get_or_init(|| tempfile::tempdir().unwrap());
    let at = root
        .path()
        .join(COUNT.fetch_add(1, Ordering::Relaxed).to_string());
    BIN.with(|bin| *bin.borrow_mut() = at.join("bin"));
    Tools::at(&at)
}

fn binned() -> Vec<PathBuf> {
    let bin = BIN.with(|bin| bin.borrow().clone());
    fs::read_dir(bin)
        .map(|entries| {
            entries
                .filter_map(|entry| Some(entry.ok()?.path()))
                .collect()
        })
        .unwrap_or_default()
}

fn key(name: &str, key_char: Option<&str>) -> Keystroke {
    Keystroke {
        modifiers: Modifiers::none(),
        key: name.to_string(),
        key_char: key_char.map(str::to_string),
    }
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
    let mut browser = Browser::new(folder.to_path_buf(), folder.to_path_buf(), false);
    browser.ops.tools = tools();
    wait_until(&mut browser, "the listing", |browser| !browser.reading());
    browser
}

fn undo(browser: &mut Browser) -> bool {
    let started = browser.undo(false);
    wait_until(browser, "the undo", |browser| !browser.undo_running());
    started
}

fn redo(browser: &mut Browser) -> bool {
    let started = browser.redo(false);
    wait_until(browser, "the redo", |browser| !browser.undo_running());
    started
}

fn notice(browser: &Browser) -> String {
    browser
        .posted()
        .last()
        .map(Message::line)
        .unwrap_or_default()
}

fn wait_notice(browser: &mut Browser, what: &str, wanted: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let drained = browser.drain(Instant::now());
        if browser
            .posted()
            .iter()
            .any(|message| message.line() == wanted)
        {
            return;
        }
        assert!(Instant::now() < deadline, "{what} never happened");
        thread::sleep(drained.wake_in.unwrap_or(Duration::from_millis(2)));
    }
}

fn visit(browser: &mut Browser, folder: &Path) {
    browser.go(folder.to_path_buf(), 0.0);
    wait_until(browser, "the listing", |browser| !browser.reading());
}

fn point_at(browser: &mut Browser, path: &Path) {
    wait_until(browser, "the row", |browser| browser.row_of(path).is_some());
    let row = browser.row_of(path).unwrap();
    assert!(browser.place_cursor(row, 0.0) || browser.cursor() == row);
}

fn finished(browser: &Browser) -> bool {
    browser
        .moves()
        .first()
        .is_some_and(|record| record.state == MoveState::Done)
}

fn write(folder: &Path, name: &str, text: &str) -> PathBuf {
    let path = folder.join(name);
    fs::write(&path, text).unwrap();
    path
}

fn two_folders() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let dest = temp.path().join("dest");
    fs::create_dir(&dest).unwrap();
    (temp, dest)
}

#[test]
fn holding_a_cut_fades_the_rows_and_tells_how_to_paste() {
    let (temp, _dest) = two_folders();
    let first = write(temp.path(), "a.txt", "a");
    write(temp.path(), "b.txt", "b");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &first);

    assert!(browser.hold(Held::Cut));
    let entry = browser.entry(browser.row_of(&first).unwrap()).unwrap();
    assert!(browser.is_cut(entry));
    assert_eq!(
        browser.note().count,
        "1 cut. Open a folder and press ctrl+v"
    );

    assert!(browser.dismiss());
    let entry = browser.entry(browser.row_of(&first).unwrap()).unwrap();
    assert!(!browser.is_cut(entry));
    assert!(browser.paste().is_none());
}

#[test]
fn the_menu_reads_what_is_held_and_how_many_rows_it_acts_on() {
    let (temp, _dest) = two_folders();
    let first = write(temp.path(), "a.txt", "a");
    let second = write(temp.path(), "b.txt", "b");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &first);
    assert!(!browser.holds());
    assert_eq!(browser.operands().len(), 1);

    browser.add_pick(browser.row_of(&first).unwrap());
    browser.add_pick(browser.row_of(&second).unwrap());
    assert_eq!(browser.operands().len(), 2);

    assert!(browser.hold(Held::Copied));
    assert!(browser.holds());
    assert!(browser.dismiss());
    assert!(!browser.holds());
}

#[test]
fn a_copy_is_not_faded_and_picks_win_over_the_cursor_row() {
    let (temp, _dest) = two_folders();
    let first = write(temp.path(), "a.txt", "a");
    let second = write(temp.path(), "b.txt", "b");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &first);
    browser.add_pick(browser.row_of(&second).unwrap());

    assert!(browser.hold(Held::Copied));
    let entry = browser.entry(browser.row_of(&second).unwrap()).unwrap();
    assert!(!browser.is_cut(entry));
    assert_eq!(
        browser.note().count,
        "1 copied. Open a folder and press ctrl+v"
    );
}

#[test]
fn nothing_to_hold_in_an_empty_folder() {
    let temp = tempfile::tempdir().unwrap();
    let mut browser = opened(temp.path());
    assert!(!browser.hold(Held::Copied));
    assert!(!browser.hold(Held::Cut));
}

#[test]
fn cut_then_paste_moves_the_file_and_undo_moves_it_back() {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "alpha");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);
    assert!(browser.hold(Held::Cut));
    visit(&mut browser, &dest);

    let transfer = browser.paste().unwrap();
    assert_eq!(transfer.verb, Verb::Move);
    assert_eq!(transfer.into, dest);
    browser.begin(transfer);
    wait_until(&mut browser, "the move", finished);

    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "alpha");
    assert!(!file.exists());
    let record = &browser.moves()[0];
    assert_eq!(record.title, "a.txt to dest");
    let arrived = notice(&browser);
    assert!(arrived.starts_with("1 moved here from "));
    assert!(arrived.ends_with("ctrl+z undoes it."));
    assert!(browser.paste().is_none());

    assert!(undo(&mut browser));
    assert!(file.exists());
    assert!(!dest.join("a.txt").exists());
    assert!(notice(&browser).starts_with("Undid the move of a.txt into dest. It is back in "));
}

#[test]
fn undo_brings_back_an_empty_folder_that_a_move_merged_into_a_new_folder() {
    let (temp, dest) = two_folders();
    let moved = temp.path().join("New folder");
    fs::create_dir(&moved).unwrap();
    let mut browser = opened(temp.path());
    visit(&mut browser, &dest);
    assert!(browser.new_folder());
    let made = dest.join("New folder");
    assert!(made.is_dir());

    visit(&mut browser, temp.path());
    point_at(&mut browser, &moved);
    assert!(browser.hold(Held::Cut));
    visit(&mut browser, &dest);
    let transfer = browser.paste().unwrap();
    browser.begin(transfer);
    wait_until(&mut browser, "the sheet", |browser| {
        browser.surveyed().is_some()
    });
    assert!(browser.choose(0, Resolve::Merge));
    assert!(browser.confirm());
    wait_until(&mut browser, "the move", finished);
    assert!(!moved.exists());

    assert!(undo(&mut browser));
    assert!(moved.is_dir());
    assert!(made.is_dir());
    assert!(undo(&mut browser));
    assert!(!made.exists());
}

#[test]
fn a_folder_moved_onto_one_with_its_name_asks_and_keeps_both_unless_you_merge() {
    let (temp, dest) = two_folders();
    let hello = temp.path().join("hello");
    fs::create_dir(&hello).unwrap();
    let mut browser = opened(temp.path());
    browser.begin(Transfer {
        verb: Verb::Copy,
        sources: vec![hello.clone()],
        into: dest.clone(),
    });
    wait_until(&mut browser, "the copy", finished);
    assert!(dest.join("hello").is_dir());

    browser.begin(Transfer {
        verb: Verb::Move,
        sources: vec![hello.clone()],
        into: dest.clone(),
    });
    wait_until(&mut browser, "the sheet", |browser| {
        browser.surveyed().is_some()
    });
    let clashes = &browser.surveyed().unwrap().clashes;
    assert_eq!(clashes.len(), 1);
    assert!(clashes[0].merges());
    assert_eq!(clashes[0].choice, Resolve::KeepBoth);
    assert!(browser.choose(0, Resolve::Replace));
    assert_eq!(
        browser.surveyed().unwrap().clashes[0].choice,
        Resolve::Merge
    );
    assert!(browser.choose(0, Resolve::KeepBoth));
    assert!(browser.confirm());
    wait_until(&mut browser, "the move", finished);
    assert!(!hello.exists());
    assert!(dest.join("hello").is_dir());
    assert!(dest.join("hello (2)").is_dir());

    assert!(undo(&mut browser));
    assert!(hello.is_dir());
    assert!(dest.join("hello").is_dir());
    assert!(!dest.join("hello (2)").exists());
    assert!(!temp.path().join("hello (2)").exists());
}

#[test]
fn copy_then_paste_leaves_the_original_and_undo_trashes_the_copy() {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "alpha");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);
    assert!(browser.hold(Held::Copied));
    visit(&mut browser, &dest);

    let transfer = browser.paste().unwrap();
    assert_eq!(transfer.verb, Verb::Copy);
    browser.begin(transfer);
    wait_until(&mut browser, "the copy", finished);

    assert!(file.exists());
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "alpha");
    let arrived = notice(&browser);
    assert!(arrived.starts_with("1 copied here from "));
    assert!(arrived.ends_with("ctrl+z undoes it."));
    assert!(browser.paste().is_some());

    assert!(undo(&mut browser));
    assert!(!dest.join("a.txt").exists());
    assert_eq!(fs::read_to_string(&file).unwrap(), "alpha");
    assert_eq!(binned().len(), 1);
    assert_eq!(
        notice(&browser),
        "Undid the copy of a.txt into dest. The copy went to the trash. ctrl+shift+z redoes it."
    );
}

#[test]
fn pasting_a_cut_into_its_own_folder_does_nothing_and_says_so() {
    let temp = tempfile::tempdir().unwrap();
    let file = write(temp.path(), "a.txt", "a");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);
    browser.hold(Held::Cut);
    assert!(browser.paste().is_none());
    assert_eq!(
        browser.posted(),
        [Message::warn("Already in this folder", "")]
    );
}

#[test]
fn a_folder_cannot_be_pasted_into_itself() {
    let (temp, dest) = two_folders();
    let mut browser = opened(temp.path());
    point_at(&mut browser, &dest);
    browser.hold(Held::Copied);
    visit(&mut browser, &dest);
    assert!(browser.paste().is_none());
}

fn row(browser: &Browser, name: &str) -> usize {
    browser.row_of(&browser.folder().join(name)).unwrap()
}

fn copy_a(browser: &mut Browser) {
    browser.click_to(row(browser, "a.txt"), 0.0);
    assert!(browser.hold(Held::Copied));
}

fn paste_folders() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    write(temp.path(), "a.txt", "a");
    write(temp.path(), "b.txt", "b");
    for folder in ["inbox", "inbox/nested", "outbox"] {
        fs::create_dir(temp.path().join(folder)).unwrap();
    }
    temp
}

#[test]
fn ctrl_v_pastes_into_the_folder_you_chose_or_else_where_you_are() {
    type Choose = fn(&mut Browser);
    let cases: [(&str, Choose, &str); 13] = [
        (
            "a clicked folder",
            |browser| {
                copy_a(browser);
                browser.click_to(row(browser, "inbox"), 0.0);
            },
            "inbox",
        ),
        (
            "a folder reached with the arrow keys",
            |browser| {
                copy_a(browser);
                assert!(browser.move_cursor(Step::Up, 1, 1));
            },
            "outbox",
        ),
        (
            "a folder reached by typing its name",
            |browser| {
                copy_a(browser);
                browser.jump('o', Instant::now());
            },
            "outbox",
        ),
        (
            "a folder picked on its own",
            |browser| {
                copy_a(browser);
                browser.add_pick(row(browser, "inbox"));
            },
            "inbox",
        ),
        (
            "two folders picked",
            |browser| {
                copy_a(browser);
                browser.add_pick(row(browser, "inbox"));
                browser.add_pick(row(browser, "outbox"));
            },
            "",
        ),
        (
            "a file under the cursor",
            |browser| {
                copy_a(browser);
                browser.click_to(row(browser, "b.txt"), 0.0);
            },
            "",
        ),
        (
            "a file picked on its own",
            |browser| {
                copy_a(browser);
                browser.add_pick(row(browser, "b.txt"));
            },
            "",
        ),
        (
            "the copied folder itself, so the copy lands beside it",
            |browser| {
                browser.click_to(row(browser, "inbox"), 0.0);
                assert!(browser.hold(Held::Copied));
            },
            "",
        ),
        (
            "picked files cut, then the arrow keys onto a folder",
            |browser| {
                browser.add_pick(row(browser, "a.txt"));
                browser.add_pick(row(browser, "b.txt"));
                assert!(browser.hold(Held::Cut));
                assert!(browser.move_cursor(Step::Down, 1, 1));
            },
            "outbox",
        ),
        (
            "a click on empty space after choosing a folder",
            |browser| {
                copy_a(browser);
                browser.click_to(row(browser, "inbox"), 0.0);
                browser.clear_selection();
            },
            "",
        ),
        (
            "a folder just opened, with a folder under the cursor",
            |browser| {
                copy_a(browser);
                let inbox = browser.folder().join("inbox");
                visit(browser, &inbox);
                assert_eq!(browser.cursor(), row(browser, "nested"));
            },
            "inbox",
        ),
        (
            "the parent, with the cursor on the folder you left",
            |browser| {
                copy_a(browser);
                let parent = browser.folder().to_path_buf();
                visit(browser, &parent.join("outbox"));
                visit(browser, &parent);
                assert_eq!(browser.cursor(), row(browser, "outbox"));
            },
            "",
        ),
        (
            "a new folder",
            |browser| {
                copy_a(browser);
                assert!(browser.new_folder());
                wait_until(browser, "the new folder", |browser| {
                    browser.rename_field().is_some()
                });
                browser.rename_key(&key("enter", None), &MemoryClipboard::default());
            },
            "New folder",
        ),
    ];
    for (what, choose, into) in cases {
        assert_pastes_into(what, choose, into);
    }
}

#[cfg(unix)]
#[test]
fn ctrl_v_pastes_into_a_chosen_folder_after_it_is_renamed() {
    assert_pastes_into(
        "a chosen folder renamed",
        |browser| {
            copy_a(browser);
            browser.click_to(row(browser, "inbox"), 0.0);
            assert!(browser.rename_open());
            for letter in ["b", "o", "x"] {
                browser.rename_key(&key(letter, Some(letter)), &MemoryClipboard::default());
            }
            browser.rename_key(&key("enter", None), &MemoryClipboard::default());
            let renamed = browser.folder().join("box");
            wait_until(browser, "the renamed folder", |browser| {
                browser.row_of(&renamed) == Some(browser.cursor())
            });
        },
        "box",
    );
}

fn assert_pastes_into(what: &str, choose: fn(&mut Browser), into: &str) {
    let temp = paste_folders();
    let mut browser = opened(temp.path());
    choose(&mut browser);
    let transfer = browser
        .paste()
        .unwrap_or_else(|| panic!("{what}: nothing pasted"));
    assert_eq!(transfer.into, temp.path().join(into), "{what}");
}

#[test]
fn the_note_names_the_folder_and_the_list_menu_pastes_where_you_are() {
    let temp = paste_folders();
    let mut browser = opened(temp.path());
    browser.click_to(row(&browser, "a.txt"), 0.0);
    assert!(browser.hold(Held::Cut));
    assert_eq!(
        browser.note().count,
        "1 cut. Open a folder and press ctrl+v"
    );

    browser.click_to(row(&browser, "inbox"), 0.0);
    assert_eq!(
        browser.note().count,
        "1 cut. Press ctrl+v to paste into inbox"
    );
    assert!(browser.paste_here().is_none());
    assert_eq!(
        browser.posted(),
        [Message::warn("Already in this folder", "")]
    );

    let inbox = temp.path().join("inbox");
    let moved = browser.paste().unwrap();
    assert_eq!(moved.into, inbox);
    browser.begin(moved);
    wait_until(&mut browser, "the move", finished);
    assert_eq!(browser.posted().len(), 1);

    visit(&mut browser, &inbox);
    browser.click_to(row(&browser, "a.txt"), 0.0);
    assert!(browser.hold(Held::Cut));
    visit(&mut browser, temp.path());
    browser.click_to(row(&browser, "inbox"), 0.0);
    assert!(browser.paste().is_none());
    assert_eq!(browser.posted(), [Message::warn("Already in inbox", "")]);
}

#[test]
fn dropping_moves_on_one_drive_and_copies_when_asked() {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "a");
    let browser = opened(temp.path());

    let moved = browser.drop_on(vec![file.clone()], dest.clone(), false);
    assert_eq!(moved.verb, Verb::Move);
    assert_eq!(moved.sources, vec![file.clone()]);
    assert_eq!(moved.into, dest);

    let copied = browser.drop_on(vec![file], dest, true);
    assert_eq!(copied.verb, Verb::Copy);
}

#[test]
fn a_name_clash_waits_for_the_sheet_and_keep_leaves_the_existing_file() {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "incoming");
    let existing = write(&dest, "a.txt", "existing");
    let mut browser = opened(temp.path());

    browser.begin(Transfer {
        verb: Verb::Copy,
        sources: vec![file],
        into: dest,
    });
    wait_until(&mut browser, "the survey", |browser| {
        browser.surveyed().is_some()
    });
    assert_eq!(browser.surveyed().unwrap().clashes.len(), 1);
    assert!(browser.moves().is_empty());

    assert!(browser.choose(0, Resolve::Keep));
    assert!(!browser.choose(5, Resolve::Keep));
    assert!(browser.confirm());
    assert!(browser.surveyed().is_none());
    wait_until(&mut browser, "the copy", finished);
    assert_eq!(fs::read_to_string(existing).unwrap(), "existing");
}

#[test]
fn choosing_keep_both_for_every_row_adds_a_numbered_copy() {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "incoming");
    write(&dest, "a.txt", "existing");
    let mut browser = opened(temp.path());

    browser.begin(Transfer {
        verb: Verb::Copy,
        sources: vec![file],
        into: dest.clone(),
    });
    wait_until(&mut browser, "the survey", |browser| {
        browser.surveyed().is_some()
    });
    assert!(browser.choose_all(Resolve::KeepBoth));
    assert!(browser.confirm());
    wait_until(&mut browser, "the copy", finished);
    assert_eq!(
        fs::read_to_string(dest.join("a (2).txt")).unwrap(),
        "incoming"
    );
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "existing");
}

#[test]
fn cancelling_the_sheet_starts_nothing() {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "incoming");
    write(&dest, "a.txt", "existing");
    let mut browser = opened(temp.path());

    browser.begin(Transfer {
        verb: Verb::Move,
        sources: vec![file.clone()],
        into: dest.clone(),
    });
    wait_until(&mut browser, "the survey", |browser| {
        browser.surveyed().is_some()
    });
    assert!(browser.cancel_survey());
    assert!(browser.surveyed().is_none());
    assert!(!browser.cancel_survey());
    assert!(!browser.confirm());
    assert!(browser.moves().is_empty());
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "existing");
    assert!(file.exists());
}

#[test]
fn a_clean_survey_starts_without_asking() {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "x");
    let mut browser = opened(temp.path());
    browser.begin(Transfer {
        verb: Verb::Copy,
        sources: vec![file],
        into: dest.clone(),
    });
    wait_until(&mut browser, "the copy", finished);
    assert!(browser.surveyed().is_none());
    assert!(dest.join("a.txt").exists());
}

#[test]
fn the_moves_list_opens_and_closes() {
    let temp = tempfile::tempdir().unwrap();
    let mut browser = opened(temp.path());
    assert!(!browser.moves_open());
    assert!(browser.toggle_moves());
    assert!(browser.moves_open());
    assert!(browser.dismiss());
    assert!(!browser.moves_open());
}

#[test]
fn rename_selects_the_stem_and_enter_renames_the_file() {
    let temp = tempfile::tempdir().unwrap();
    let file = write(temp.path(), "a.txt", "a");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);

    assert!(browser.rename_open());
    assert!(!browser.rename_open());
    let field = browser.rename_field().unwrap().field();
    assert_eq!(field.text(), "a.txt");
    assert_eq!(field.selected_range(), Some((0, 1)));
    let entry = browser.entry(browser.row_of(&file).unwrap()).unwrap();
    assert!(browser.renaming(entry));

    assert!(browser.rename_key(&key("b", Some("b")), &MemoryClipboard::default()));
    assert!(browser.rename_key(&key("enter", None), &MemoryClipboard::default()));
    assert!(browser.rename_field().is_none());
    assert!(temp.path().join("b.txt").exists());
    assert!(!file.exists());

    assert!(undo(&mut browser));
    assert!(file.exists());
    assert!(!temp.path().join("b.txt").exists());
    assert_eq!(
        notice(&browser),
        "Undid the rename to b.txt. Renamed back to a.txt. ctrl+shift+z redoes it."
    );
}

#[test]
fn a_folder_name_is_selected_whole() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path().join("my.dir");
    fs::create_dir(&folder).unwrap();
    let mut browser = opened(temp.path());
    point_at(&mut browser, &folder);
    assert!(browser.rename_open());
    assert_eq!(
        browser.rename_field().unwrap().field().selected_range(),
        Some((0, 6))
    );
}

#[test]
fn a_taken_name_keeps_the_field_open_and_says_why() {
    let temp = tempfile::tempdir().unwrap();
    let file = write(temp.path(), "a.txt", "a");
    write(temp.path(), "b.txt", "b");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);
    browser.rename_open();
    browser.rename_key(&key("b", Some("b")), &MemoryClipboard::default());
    browser.rename_key(&key("enter", None), &MemoryClipboard::default());

    assert!(browser.rename_field().is_some());
    assert_eq!(
        browser.posted(),
        [Message::warn(
            "Could not rename to b.txt",
            "b.txt already exists here"
        )]
    );
    assert!(file.exists());
}

#[test]
fn escape_closes_the_rename_field_and_an_unchanged_name_renames_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let file = write(temp.path(), "a.txt", "a");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);

    browser.rename_open();
    assert!(browser.rename_key(&key("escape", None), &MemoryClipboard::default()));
    assert!(browser.rename_field().is_none());

    browser.rename_open();
    assert!(browser.rename_key(&key("enter", None), &MemoryClipboard::default()));
    assert!(browser.rename_field().is_none());
    assert!(file.exists());
    assert!(!browser.rename_key(&key("enter", None), &MemoryClipboard::default()));
}

#[test]
fn a_new_folder_appears_named_and_opens_for_rename() {
    let temp = tempfile::tempdir().unwrap();
    let mut browser = opened(temp.path());

    assert!(browser.new_folder());
    wait_until(&mut browser, "the rename field", |browser| {
        browser.rename_field().is_some()
    });
    assert!(temp.path().join("New folder").is_dir());
    assert_eq!(browser.rename_field().unwrap().text(), "New folder");

    browser.rename_key(&key("escape", None), &MemoryClipboard::default());
    assert!(browser.new_folder());
    wait_until(&mut browser, "the second field", |browser| {
        browser.rename_field().is_some()
    });
    assert!(temp.path().join("New folder (2)").is_dir());
}

#[test]
fn undo_removes_an_empty_new_folder_and_then_says_there_is_nothing_to_undo() {
    let temp = tempfile::tempdir().unwrap();
    let mut browser = opened(temp.path());
    assert!(!undo(&mut browser));
    assert_eq!(notice(&browser), "Nothing to undo");

    browser.new_folder();
    assert!(undo(&mut browser));
    assert!(!temp.path().join("New folder").exists());
    assert_eq!(
        notice(&browser),
        "Undid the new folder New folder. Removed the new folder New folder, which was empty. ctrl+shift+z redoes it."
    );
    assert!(!undo(&mut browser));
    assert_eq!(notice(&browser), "Nothing to undo");
}

#[test]
fn leaving_the_folder_closes_the_rename_field() {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "a");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);
    browser.rename_open();
    visit(&mut browser, &dest);
    assert!(browser.rename_field().is_none());
}

#[test]
fn wording_matches_the_design() {
    assert_eq!(
        ops_text::held_note(Held::Cut, 4, None),
        "4 cut. Open a folder and press ctrl+v"
    );
    assert_eq!(
        ops_text::held_note(Held::Copied, 1, Some("Photos")),
        "1 copied. Press ctrl+v to paste into Photos"
    );
    assert_eq!(
        ops_text::drop_hint(false),
        "Drop to move. Hold ctrl to copy."
    );
    assert_eq!(
        ops_text::of_total(3_100_000_000, 8_500_000_000),
        "3.1 of 8.5 GB"
    );
    assert_eq!(
        ops_text::of_total(900_000_000, 8_500_000_000),
        "900 MB of 8.5 GB"
    );
    assert_eq!(
        ops_text::running_progress(
            Verb::Move,
            "virtualbox",
            "WD_SN850X",
            3_100_000_000,
            8_500_000_000,
            Duration::from_secs(31)
        ),
        "Moving virtualbox from WD_SN850X · 3.1 of 8.5 GB · 54 s left"
    );
    assert_eq!(
        ops_text::arrived_note(Verb::Move, 4, true, "dest", "plans", true).line(),
        "4 moved here from plans. ctrl+z undoes it."
    );
    assert_eq!(
        ops_text::arrived_note(Verb::Copy, 2, false, "dest", "plans", false).line(),
        "2 copied to dest from plans"
    );
    assert_eq!(
        ops_text::trashed_note(3, true).line(),
        "3 moved to the trash. ctrl+z puts them back."
    );
    assert_eq!(
        ops_text::trashed_note(1, true).line(),
        "1 moved to the trash. ctrl+z puts it back."
    );
    assert_eq!(
        ops_text::trashed_note(2, false).line(),
        "2 moved to the trash"
    );
    assert_eq!(
        ops_text::nothing_changed(Path::new("/a"), Path::new("/a")).line(),
        "Nothing changed here"
    );
    assert_eq!(
        ops_text::nothing_changed(Path::new("/a/dest"), Path::new("/a")).line(),
        "Nothing changed in dest"
    );
    assert_eq!(
        ops_text::stopped_detail(0),
        "Stopped before it finished. Nothing was placed."
    );
    assert_eq!(
        ops_text::stopped_detail(1),
        "Stopped before it finished. 1 item arrived, and ctrl+z undoes it."
    );
    assert_eq!(
        ops_text::stopped_detail(3),
        "Stopped before it finished. 3 items arrived, and ctrl+z undoes them."
    );
    let refusals = [
        (
            Direction::Undo,
            "when the copy ends",
            "Nothing was undone. Press ctrl+z again when the copy ends.",
        ),
        (
            Direction::Redo,
            "when the move ends",
            "Nothing was redone. Press ctrl+shift+z again when the move ends.",
        ),
        (
            Direction::Redo,
            "once the folder has loaded",
            "Nothing was redone. Press ctrl+shift+z again once the folder has loaded.",
        ),
    ];
    for (direction, when, expected) in refusals {
        let refused = ops_text::refused(direction, when);
        assert_eq!(refused.line(), expected);
        assert_eq!(refused.level, Level::Warn);
    }
    assert_eq!(
        ops_text::undoing_note(Direction::Undo, "copy of a.txt"),
        "Undoing the copy of a.txt. Esc stops it."
    );
    assert_eq!(
        ops_text::undoing_note(Direction::Redo, "copy of a.txt"),
        "Redoing the copy of a.txt. Esc stops it."
    );
    assert_eq!(
        ops_text::nothing_to(Direction::Undo),
        Message::info("Nothing to undo", "")
    );
    assert_eq!(
        ops_text::nothing_to(Direction::Redo),
        Message::info("Nothing to redo", "")
    );
    assert_eq!(
        ops_text::asked_off(Direction::Undo),
        Message::info("Nothing was undone", "ctrl+z asks again.")
    );
    assert_eq!(
        ops_text::asked_off(Direction::Redo),
        Message::info("Nothing was redone", "ctrl+shift+z asks again.")
    );
}

#[test]
fn a_failure_names_what_could_not_happen_with_the_count_and_the_first_reason() {
    let failure = |name: &str| Failure {
        path: PathBuf::from("/d").join(name),
        reason: "it is busy".to_string(),
    };
    let cases = [
        (
            "move",
            " to the trash",
            vec![failure("a")],
            "Could not move 1 item to the trash. a: it is busy",
        ),
        (
            "delete",
            "",
            vec![failure("a"), failure("b")],
            "Could not delete 2 items. a: it is busy (1 more)",
        ),
        (
            "copy",
            "",
            vec![failure("a"), failure("b"), failure("c")],
            "Could not copy 3 items. a: it is busy (2 more)",
        ),
    ];
    for (verb, after, failures, expected) in cases {
        let message = ops_text::failure_text(verb, after, &failures).unwrap();
        assert_eq!(message.line(), expected);
        assert_eq!(message.level, Level::Error);
    }
    assert_eq!(ops_text::failure_text("copy", "", &[]), None);
}

#[test]
fn time_left_waits_for_a_rate_and_switches_to_minutes() {
    let second = Duration::from_secs(1);
    assert_eq!(ops_text::time_left(0, 100, second), None);
    assert_eq!(ops_text::time_left(100, 100, second), None);
    assert_eq!(
        ops_text::time_left(10, 100, Duration::from_millis(500)),
        None
    );
    assert_eq!(
        ops_text::time_left(100, 600, Duration::from_secs(10)).as_deref(),
        Some("50 s left")
    );
    assert_eq!(
        ops_text::time_left(100, 6_100, Duration::from_secs(10)).as_deref(),
        Some("10 min left")
    );
}

fn survey_of(sources: &[&str], room: Room, copy_only: &[&str]) -> Survey {
    Survey {
        transfer: Transfer {
            verb: Verb::Move,
            sources: sources.iter().map(PathBuf::from).collect(),
            into: PathBuf::from("/documents"),
        },
        clashes: Vec::new(),
        same_inside: 0,
        bytes: 0,
        room,
        copy_only: copy_only
            .iter()
            .map(|path| CopyOnly {
                path: PathBuf::from(path),
                reason: "belongs to the user ollama".to_string(),
            })
            .collect(),
        crosses_volume: false,
    }
}

#[test]
fn the_sheet_notes_name_the_problem_in_plain_words() {
    let short = survey_of(
        &["/vm/windows-11"],
        Room::Short {
            need: 42_200_000_000,
            free: 23_000_000_000,
        },
        &[],
    );
    assert_eq!(
        ops_text::short_note(&short, "This computer").as_deref(),
        Some("windows-11 is 42 GB. This computer has 23 GB free.")
    );
    assert_eq!(
        ops_text::not_started_detail(&short),
        "Not started. It is 42 GB and only 23 GB was free."
    );

    let owned = survey_of(&["/ai/ollama"], Room::Enough, &["/ai/ollama"]);
    assert_eq!(
        ops_text::copy_only_note(&owned).as_deref(),
        Some("ollama belongs to the user ollama. It can be copied, not moved.")
    );
    assert_eq!(ops_text::short_note(&owned, "This computer"), None);
    assert_eq!(ops_text::copy_only_note(&short), None);
}

#[test]
fn the_kept_sentence_counts_newer_files_and_identical_ones() {
    let stamp = |modified| Stamp {
        bytes: 1,
        modified: Some(modified),
        folder: false,
    };
    let newer = SystemTime::now();
    let older = SystemTime::UNIX_EPOCH;
    let clash = |here, incoming, choice| Clash {
        source: PathBuf::from("a"),
        existing: PathBuf::from("b"),
        here: stamp(here),
        incoming: stamp(incoming),
        same: false,
        choice,
    };
    let kept = vec![
        clash(newer, older, Resolve::Keep),
        clash(newer, older, Resolve::Keep),
    ];
    assert_eq!(
        ops_text::kept_sentence(&kept, 26),
        "Kept the 2 newer files there. The other 26 were already the same."
    );
    let mixed = vec![clash(older, newer, Resolve::Keep)];
    assert_eq!(ops_text::kept_sentence(&mixed, 0), "Kept 1 file there.");
    let replaced = vec![clash(older, newer, Resolve::Replace)];
    assert_eq!(
        ops_text::kept_sentence(&replaced, 3),
        "The 3 were already the same."
    );
    assert_eq!(ops_text::kept_sentence(&replaced, 0), "");

    let folder = |choice| Clash {
        existing: PathBuf::from("docs"),
        here: Stamp {
            folder: true,
            ..stamp(newer)
        },
        incoming: Stamp {
            folder: true,
            ..stamp(older)
        },
        ..clash(newer, older, choice)
    };
    let inside = |choice| Clash {
        existing: PathBuf::from("docs/a"),
        ..clash(newer, older, choice)
    };
    let cases = [
        (vec![folder(Resolve::Keep)], 0, "Kept 1 folder there."),
        (
            vec![folder(Resolve::KeepBoth), inside(Resolve::Keep)],
            3,
            "",
        ),
        (
            vec![folder(Resolve::Merge), inside(Resolve::Keep)],
            3,
            "Kept the 1 newer file there. The other 3 were already the same.",
        ),
        (
            vec![folder(Resolve::Keep), clash(newer, older, Resolve::Keep)],
            0,
            "Kept 2 items there.",
        ),
    ];
    for (clashes, same_inside, expected) in cases {
        assert_eq!(ops_text::kept_sentence(&clashes, same_inside), expected);
    }
}

#[test]
fn a_new_file_appears_empty_opens_for_rename_and_undo_sends_it_to_the_trash() {
    let temp = tempfile::tempdir().unwrap();
    let mut browser = opened(temp.path());

    assert!(browser.new_file());
    wait_until(&mut browser, "the rename field", |browser| {
        browser.rename_field().is_some()
    });
    let made = temp.path().join("New file");
    assert!(made.is_file());
    assert_eq!(fs::metadata(&made).unwrap().len(), 0);
    assert_eq!(browser.rename_field().unwrap().text(), "New file");

    browser.rename_key(&key("escape", None), &MemoryClipboard::default());
    assert!(undo(&mut browser));
    assert!(!made.exists());
    assert_eq!(binned().len(), 1);
    assert_eq!(
        notice(&browser),
        "Undid the new file New file. The new file New file went to the trash. ctrl+shift+z redoes it."
    );
}

#[test]
fn duplicating_copies_beside_the_original_and_counts_up() {
    let temp = tempfile::tempdir().unwrap();
    let file = write(temp.path(), "a.txt", "text");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);

    assert!(browser.duplicate_picks());
    wait_until(&mut browser, "the copy", finished);
    assert_eq!(
        fs::read_to_string(temp.path().join("a copy.txt")).unwrap(),
        "text"
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), "text");

    point_at(&mut browser, &file);
    assert!(browser.duplicate_picks());
    wait_until(&mut browser, "the second copy", |_| {
        temp.path().join("a copy 2.txt").exists()
    });
}

#[test]
fn deleting_permanently_asks_first_and_undo_says_it_cannot_be_undone() {
    let temp = tempfile::tempdir().unwrap();
    let file = write(temp.path(), "a.txt", "text");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);

    assert!(browser.erase_ask());
    assert_eq!(browser.erasing().unwrap(), std::slice::from_ref(&file));
    assert!(browser.erase_cancel());
    assert!(browser.erasing().is_none());
    assert!(file.exists());

    assert!(browser.erase_ask());
    assert!(browser.erase_confirm());
    wait_until(&mut browser, "the delete", |_| !file.exists());
    wait_notice(&mut browser, "the note", "1 item deleted for good");

    assert!(undo(&mut browser));
    assert_eq!(
        notice(&browser),
        "Cannot undo the permanent delete of a.txt. Delete permanently cannot be undone. Next is the permanent delete of a.txt, which changed nothing."
    );
    assert!(undo(&mut browser));
    assert_eq!(
        notice(&browser),
        "The permanent delete of a.txt changed nothing. Nothing is left to undo."
    );
    assert!(!undo(&mut browser));
    assert_eq!(notice(&browser), "Nothing to undo");
}

#[test]
fn deleting_permanently_skips_picks_held_in_another_folder() {
    let temp = tempfile::tempdir().unwrap();
    let sub = temp.path().join("sub");
    fs::create_dir(&sub).unwrap();
    let far = write(&sub, "far.txt", "text");
    let mut browser = opened(temp.path());
    visit(&mut browser, &sub);
    point_at(&mut browser, &far);
    assert!(browser.toggle_pick());
    visit(&mut browser, temp.path());

    assert!(browser.erase_ask());
    assert!(browser.erasing().is_none());
    assert_eq!(browser.posted(), [ops_text::erase_elsewhere_note()]);
    assert!(far.exists());
}

#[test]
fn leaving_the_folder_drops_the_delete_question() {
    let temp = tempfile::tempdir().unwrap();
    let file = write(temp.path(), "a.txt", "text");
    fs::create_dir(temp.path().join("sub")).unwrap();
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);
    assert!(browser.erase_ask());
    visit(&mut browser, &temp.path().join("sub"));
    assert!(browser.erasing().is_none());
}

#[test]
fn the_delete_question_names_the_count_and_says_it_cannot_be_undone() {
    let one = ops_text::erase_lines(&[PathBuf::from("/x/a.txt")]);
    assert_eq!(one[0], "Delete 1 item permanently? a.txt");
    let many = ops_text::erase_lines(&[PathBuf::from("/x/a"), PathBuf::from("/x/b")]);
    assert_eq!(many[0], "Delete 2 items permanently?");
    assert!(many[1].contains("cannot be undone"));
}

fn copy_into(browser: &mut Browser, file: &Path, dest: &Path) {
    browser.begin(Transfer {
        verb: Verb::Copy,
        sources: vec![file.to_path_buf()],
        into: dest.to_path_buf(),
    });
    wait_until(browser, "the copy", finished);
}

#[test]
fn keep_this_changes_nothing_and_undo_does_not_reach_past_it_to_trash_a_visible_file() {
    let (temp, dest) = two_folders();
    let hello = write(temp.path(), "hello", "hello");
    let mut browser = opened(temp.path());
    copy_into(&mut browser, &hello, &dest);
    assert_eq!(fs::read_to_string(dest.join("hello")).unwrap(), "hello");

    browser.begin(Transfer {
        verb: Verb::Move,
        sources: vec![hello.clone()],
        into: dest.clone(),
    });
    wait_until(&mut browser, "the sheet", |browser| {
        browser.surveyed().is_some()
    });
    assert!(browser.choose(0, Resolve::Keep));
    assert!(browser.confirm());
    wait_notice(&mut browser, "the move", "Nothing changed in dest");
    assert!(hello.exists());
    assert!(dest.join("hello").exists());

    assert!(undo(&mut browser));
    assert_eq!(
        notice(&browser),
        "The move of hello into dest changed nothing. Next, ctrl+z undoes the copy of hello into dest."
    );
    assert!(hello.exists());
    assert!(dest.join("hello").exists());
    assert!(binned().is_empty());

    assert!(undo(&mut browser));
    assert!(hello.exists());
    assert!(!dest.join("hello").exists());
    assert_eq!(binned().len(), 1);
    assert_eq!(
        notice(&browser),
        "Undid the copy of hello into dest. The copy went to the trash. ctrl+shift+z redoes it."
    );
}

#[test]
fn escape_on_the_paste_sheet_leaves_an_empty_step_that_undo_reports() {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "incoming");
    write(&dest, "a.txt", "existing");
    let mut browser = opened(temp.path());
    browser.begin(Transfer {
        verb: Verb::Copy,
        sources: vec![file],
        into: dest.clone(),
    });
    wait_until(&mut browser, "the sheet", |browser| {
        browser.surveyed().is_some()
    });
    assert!(browser.cancel_survey());

    assert!(undo(&mut browser));
    assert_eq!(
        notice(&browser),
        "The copy of a.txt into dest changed nothing. Nothing is left to undo."
    );
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "existing");
    assert!(!undo(&mut browser));
    assert_eq!(notice(&browser), "Nothing to undo");
}

#[test]
fn a_copy_edited_after_it_landed_stays() {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "alpha");
    let mut browser = opened(temp.path());
    copy_into(&mut browser, &file, &dest);
    fs::write(dest.join("a.txt"), "alpha, then edited").unwrap();

    assert!(undo(&mut browser));
    assert_eq!(
        notice(&browser),
        "Could not undo the copy of a.txt into dest. Nothing was undone: a.txt changed, so it stays. Nothing is left to undo."
    );
    assert_eq!(
        fs::read_to_string(dest.join("a.txt")).unwrap(),
        "alpha, then edited"
    );
    assert!(binned().is_empty());
}

fn moved_away_and_replaced() -> (tempfile::TempDir, PathBuf, Browser) {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "old");
    let mut browser = opened(temp.path());
    browser.begin(Transfer {
        verb: Verb::Move,
        sources: vec![file],
        into: dest.clone(),
    });
    wait_until(&mut browser, "the move", finished);
    write(temp.path(), "a.txt", "new");
    (temp, dest, browser)
}

fn asked(browser: &mut Browser) {
    assert!(browser.undo(false));
    wait_until(browser, "the undo sheet", |browser| browser.undo_asking());
}

#[test]
fn undo_never_replaces_so_it_asks_and_keep_both_adds_a_number() {
    let (temp, dest, mut browser) = moved_away_and_replaced();

    asked(&mut browser);
    assert_eq!(
        browser.undo_lines().unwrap(),
        [
            "Undo: a.txt is already here.",
            "Undo never replaces anything. Keep both puts it back as a (2).txt."
        ]
    );
    assert_eq!(
        browser.surveyed().unwrap().clashes[0].choice,
        Resolve::KeepBoth
    );
    assert!(!browser.choose(0, Resolve::Replace));
    assert!(!browser.choose_all(Resolve::Merge));
    assert!(browser.undo(false));

    assert!(browser.cancel_survey());
    assert!(!browser.undo_asking());
    assert_eq!(notice(&browser), "Nothing was undone. ctrl+z asks again.");
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "old");
    assert_eq!(
        fs::read_to_string(temp.path().join("a.txt")).unwrap(),
        "new"
    );

    asked(&mut browser);
    assert!(browser.confirm());
    wait_until(&mut browser, "the undo", |browser| !browser.undo_running());
    assert!(!dest.join("a.txt").exists());
    assert_eq!(
        fs::read_to_string(temp.path().join("a.txt")).unwrap(),
        "new"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("a (2).txt")).unwrap(),
        "old"
    );
    assert_eq!(
        notice(&browser),
        "Undid the move of a.txt into dest. It is back here. ctrl+shift+z redoes it."
    );
}

#[test]
fn keep_this_on_the_undo_sheet_leaves_the_item_and_drops_the_step() {
    let (temp, dest, mut browser) = moved_away_and_replaced();

    asked(&mut browser);
    assert!(browser.choose(0, Resolve::Keep));
    assert!(browser.confirm());
    wait_until(&mut browser, "the undo", |browser| !browser.undo_running());
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "old");
    assert_eq!(
        fs::read_to_string(temp.path().join("a.txt")).unwrap(),
        "new"
    );
    assert!(!temp.path().join("a (2).txt").exists());
    assert_eq!(
        notice(&browser),
        "Could not undo the move of a.txt into dest. Nothing was undone: a.txt stays where it is, as you chose. Nothing is left to undo."
    );
    assert!(!undo(&mut browser));
    assert_eq!(notice(&browser), "Nothing to undo");
}

#[test]
fn trash_then_undo_puts_the_exact_item_back_from_the_bin() {
    let temp = tempfile::tempdir().unwrap();
    let file = write(temp.path(), "a.txt", "text");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);

    assert!(browser.trash_picks());
    wait_notice(
        &mut browser,
        "the trash",
        "1 moved to the trash. ctrl+z puts it back.",
    );
    assert!(!file.exists());
    assert_eq!(binned().len(), 1);

    assert!(undo(&mut browser));
    assert_eq!(fs::read_to_string(&file).unwrap(), "text");
    assert_eq!(
        notice(&browser),
        "Undid the move of a.txt to the trash. Put a.txt back here. ctrl+shift+z redoes it."
    );
}

#[test]
fn another_windows_job_refuses_the_undo_and_touches_nothing() {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "text");
    let mut browser = opened(temp.path());
    copy_into(&mut browser, &file, &dest);

    let other_window = browser.ops.tools.gate.share();
    assert!(other_window.is_some());
    assert!(browser.undo(false));
    assert!(!browser.undo_running());
    assert_eq!(
        notice(&browser),
        "Nothing was undone. Press ctrl+z again when the move in another window ends."
    );
    assert!(dest.join("a.txt").exists());
    assert!(binned().is_empty());

    drop(other_window);
    assert!(undo(&mut browser));
    assert!(!dest.join("a.txt").exists());
}

#[test]
fn undo_waits_for_the_folder_to_load_and_never_queues() {
    let temp = tempfile::tempdir().unwrap();
    let mut browser = opened(temp.path());
    assert!(browser.new_folder());
    let made = temp.path().join("New folder");

    assert!(browser.undo(true));
    assert_eq!(
        notice(&browser),
        "Nothing was undone. Press ctrl+z again once the folder has loaded."
    );
    assert!(!browser.undo_running());
    assert!(made.is_dir());
}

fn print() -> Print {
    Print {
        id: None,
        kind: Kind::File,
        len: 0,
        modified: None,
        born: None,
    }
}

fn moved_pair() -> (History, crate::ops::history::Step) {
    let mut history = History::default();
    let id = history.open(Action::Transfer {
        verb: Verb::Move,
        sources: vec![PathBuf::from("/d/a"), PathBuf::from("/d/b")],
        into: PathBuf::from("/x/dest"),
    });
    for name in ["a", "b"] {
        history.record(
            id,
            Change::Moved {
                from: PathBuf::from("/d").join(name),
                to: PathBuf::from("/x/dest").join(name),
                at: print(),
            },
        );
    }
    history.close(id);
    let step = history.step(id).cloned().unwrap();
    (history, step)
}

fn note_of(
    step: &crate::ops::history::Step,
    outcomes: &[(usize, Outcome)],
    direction: Direction,
    stopped: bool,
    reversible: bool,
) -> String {
    let next = match direction {
        Direction::Undo => "Nothing is left to undo.",
        Direction::Redo => "Nothing is left to redo.",
    };
    ops_text::report_note(
        step,
        outcomes,
        &ops_text::Report {
            direction,
            stopped,
            here: Path::new("/d"),
            next,
            reversible,
        },
    )
}

#[test]
fn the_undo_note_names_what_came_back_what_stayed_and_what_to_press() {
    let (_history, step) = moved_pair();
    let back = |name: &str| {
        Outcome::Undone(Undone::MovedBack {
            to: PathBuf::from("/d").join(name),
            at: print(),
        })
    };
    type Case<'a> = (Vec<(usize, Outcome)>, bool, bool, &'a str);
    let cases: Vec<Case<'_>> = vec![
        (
            vec![(1, back("b")), (0, back("a"))],
            false,
            true,
            "They are back here. ctrl+shift+z redoes it.",
        ),
        (
            vec![(1, back("b")), (0, back("a"))],
            false,
            false,
            "They are back here.",
        ),
        (
            vec![
                (1, back("b")),
                (0, Outcome::Leaves(Reason::Changed(PathBuf::from("/x/dest/a")))),
            ],
            false,
            true,
            "It is back here. a changed, so it stays. ctrl+shift+z redoes it.",
        ),
        (
            vec![
                (1, back("b")),
                (
                    0,
                    Outcome::Stays(Reason::Failed(
                        PathBuf::from("/x/dest/a"),
                        "it is open in another app".to_string(),
                    )),
                ),
            ],
            false,
            false,
            "It is back here. a stays where it is: it is open in another app. ctrl+z tries again.",
        ),
        (
            vec![
                (1, back("b")),
                (0, Outcome::Leaves(Reason::Taken(PathBuf::from("/d/a")))),
            ],
            false,
            false,
            "It is back here. The name a is taken, so it stays where it is.",
        ),
        (
            vec![(1, back("b")), (0, Outcome::Stays(Reason::Stopped))],
            false,
            false,
            "It is back here. The undo stopped before it finished. ctrl+z undoes the rest.",
        ),
        (
            vec![(
                1,
                Outcome::Leaves(Reason::OriginalGone(PathBuf::from("/d/b"))),
            )],
            false,
            false,
            "Nothing was undone: the original b is gone, so the copy stays. Nothing is left to undo.",
        ),
        (
            vec![
                (1, Outcome::Leaves(Reason::Gone(PathBuf::from("/x/dest/b")))),
                (0, Outcome::Leaves(Reason::Gone(PathBuf::from("/x/dest/a")))),
            ],
            false,
            false,
            "Nothing was undone: b is gone. 1 more stays as it is. Nothing is left to undo.",
        ),
        (
            Vec::new(),
            true,
            false,
            "Nothing was undone. The undo stopped before it finished. ctrl+z undoes the rest.",
        ),
    ];
    for (outcomes, stopped, reversible, expected) in cases {
        assert_eq!(
            note_of(&step, &outcomes, Direction::Undo, stopped, reversible),
            expected,
            "{outcomes:?}"
        );
    }
}

fn copied_trio() -> crate::ops::history::Step {
    let mut history = History::default();
    let id = history.open(Action::Transfer {
        verb: Verb::Copy,
        sources: ["a", "b", "notes.txt"]
            .iter()
            .map(|name| PathBuf::from("/d").join(name))
            .collect(),
        into: PathBuf::from("/x/dest"),
    });
    for name in ["a", "b", "notes.txt"] {
        history.record(
            id,
            Change::Copied {
                from: PathBuf::from("/d").join(name),
                to: PathBuf::from("/x/dest").join(name),
                tree: tree(),
                original: tree(),
            },
        );
    }
    history.close(id);
    history.step(id).cloned().unwrap()
}

fn tree() -> crate::ops::print::TreePrint {
    crate::ops::print::TreePrint {
        root: print(),
        entries: 0,
        bytes: 0,
        shape: 0,
        objects: 0,
    }
}

#[test]
fn undoing_a_copy_names_the_trash_the_stragglers_and_the_redo_key() {
    let step = copied_trio();
    let binned = Outcome::Undone(Undone::Binned(None));
    let one = [(0, binned.clone())];
    assert_eq!(
        note_of(&step, &one, Direction::Undo, false, true),
        "The copy went to the trash. ctrl+shift+z redoes it."
    );
    let two_of_three = [
        (
            2,
            Outcome::Leaves(Reason::Changed(PathBuf::from("/x/dest/notes.txt"))),
        ),
        (1, binned.clone()),
        (0, binned),
    ];
    assert_eq!(
        note_of(&step, &two_of_three, Direction::Undo, false, true),
        "The copies went to the trash. notes.txt changed, so it stays. ctrl+shift+z redoes it."
    );
}

fn moved_again(name: &str) -> Outcome {
    Outcome::Redone(Change::Moved {
        from: PathBuf::from("/d").join(name),
        to: PathBuf::from("/x/dest").join(name),
        at: print(),
    })
}

#[test]
fn the_redo_note_says_where_it_landed_what_stayed_and_what_undoes_it() {
    let (_history, step) = moved_pair();
    type Case<'a> = (Vec<(usize, Outcome)>, bool, bool, &'a str);
    let cases: Vec<Case<'_>> = vec![
        (
            vec![(0, moved_again("a"))],
            false,
            true,
            "It is in dest again. ctrl+z undoes it.",
        ),
        (
            vec![(0, moved_again("a")), (1, moved_again("b"))],
            false,
            true,
            "They are in dest again. ctrl+z undoes it.",
        ),
        (
            vec![(0, moved_again("a")), (1, moved_again("b"))],
            false,
            false,
            "They are in dest again.",
        ),
        (
            vec![
                (0, moved_again("a")),
                (1, Outcome::Leaves(Reason::Changed(PathBuf::from("/d/b")))),
            ],
            false,
            true,
            "It is in dest again. b changed, so it stays. ctrl+z undoes it.",
        ),
        (
            vec![
                (0, moved_again("a")),
                (1, Outcome::Stays(Reason::Unplugged(PathBuf::from("/d/b")))),
            ],
            false,
            false,
            "It is in dest again. The drive of b is not plugged in, so it stays for now. ctrl+shift+z tries again.",
        ),
        (
            vec![(0, moved_again("a")), (1, Outcome::Stays(Reason::Stopped))],
            false,
            false,
            "It is in dest again. The redo stopped before it finished. ctrl+shift+z redoes the rest.",
        ),
        (
            vec![(0, Outcome::Leaves(Reason::Taken(PathBuf::from("/x/dest/a"))))],
            false,
            false,
            "Nothing was redone: the name a is taken, so it stays where it is. Nothing is left to redo.",
        ),
        (
            Vec::new(),
            true,
            false,
            "Nothing was redone. The redo stopped before it finished. ctrl+shift+z redoes the rest.",
        ),
    ];
    for (outcomes, stopped, reversible, expected) in cases {
        assert_eq!(
            note_of(&step, &outcomes, Direction::Redo, stopped, reversible),
            expected,
            "{outcomes:?}"
        );
    }
}

#[test]
fn a_redo_names_each_kind_of_step_in_plain_words() {
    let at = |name: &str| PathBuf::from("/d").join(name);
    let copy = |name: &str| Change::Copied {
        from: at(name),
        to: PathBuf::from("/x/dest").join(name),
        tree: tree(),
        original: tree(),
    };
    let one = |action: Action, change: Change| {
        let mut history = History::default();
        let id = history.open(action);
        history.record(id, change.clone());
        history.close(id);
        (
            history.step(id).cloned().unwrap(),
            vec![(0, Outcome::Redone(change))],
        )
    };
    let cases = [
        (
            one(
                Action::Transfer {
                    verb: Verb::Copy,
                    sources: vec![at("hello")],
                    into: PathBuf::from("/x/dest"),
                },
                copy("hello"),
            ),
            "The copy is in dest again. ctrl+z undoes it.",
        ),
        (
            one(
                Action::Transfer {
                    verb: Verb::Move,
                    sources: vec![at("hello")],
                    into: PathBuf::from("/x/dest"),
                },
                Change::Crossed {
                    from: at("hello"),
                    to: PathBuf::from("/x/dest/hello"),
                    tree: tree(),
                    source: Source::Partly,
                    from_device: None,
                },
            ),
            "It is in dest again. The original hello in d stays too, so it is in both places. ctrl+z undoes it.",
        ),
        (
            one(
                Action::Transfer {
                    verb: Verb::Duplicate,
                    sources: vec![at("hello")],
                    into: PathBuf::from("/d"),
                },
                Change::Copied {
                    from: at("hello"),
                    to: at("hello copy"),
                    tree: tree(),
                    original: tree(),
                },
            ),
            "The duplicate is here again. ctrl+z undoes it.",
        ),
        (
            one(
                Action::Rename {
                    from: at("a"),
                    to: at("b"),
                },
                Change::Moved {
                    from: at("a"),
                    to: at("b"),
                    at: print(),
                },
            ),
            "It is named b again. ctrl+z undoes it.",
        ),
        (
            one(
                Action::Trash(vec![at("a")]),
                Change::Trashed {
                    from: at("a"),
                    held: None,
                },
            ),
            "It is in the trash again. ctrl+z undoes it.",
        ),
        (
            one(
                Action::NewFolder(at("New folder")),
                Change::Made {
                    path: at("New folder"),
                    at: print(),
                },
            ),
            "The new folder New folder is back. ctrl+z undoes it.",
        ),
        (
            one(
                Action::NewFile(at("New file")),
                Change::Made {
                    path: at("New file"),
                    at: print(),
                },
            ),
            "The new file New file is back. ctrl+z undoes it.",
        ),
    ];
    for ((step, outcomes), expected) in cases {
        assert_eq!(
            note_of(&step, &outcomes, Direction::Redo, false, true),
            expected,
            "{:?}",
            step.action
        );
    }
}

#[test]
fn every_reason_reads_in_plain_words_for_an_undo_and_for_a_redo() {
    let (_history, step) = moved_pair();
    let path = || PathBuf::from("/x/dest/a");
    let cases: Vec<(Reason, &str, &str)> = vec![
        (Reason::Gone(path()), "a is gone.", "a is gone."),
        (
            Reason::Changed(path()),
            "a changed, so it stays.",
            "a changed, so it stays.",
        ),
        (
            Reason::OriginalGone(path()),
            "the original a is gone, so the copy stays.",
            "the original a is gone, so it is not copied again.",
        ),
        (
            Reason::OriginalChanged(path()),
            "the original a changed, so the copy stays.",
            "the original a changed, so it is not copied again.",
        ),
        (
            Reason::NotInTrash(path()),
            "a is no longer in the trash.",
            "a is no longer in the trash.",
        ),
        (
            Reason::Taken(path()),
            "the name a is taken, so it stays where it is.",
            "the name a is taken, so it stays where it is.",
        ),
        (
            Reason::Kept(path()),
            "a stays where it is, as you chose.",
            "a stays where it is, as you chose.",
        ),
        (
            Reason::Partly(path()),
            "a was only partly moved, so its files stay in both places.",
            "a was only partly moved, so its files stay in both places.",
        ),
        (
            Reason::CannotUndo(path()),
            "a cannot be put back from here, but your desktop's trash can restore it.",
            "a cannot be redone from here.",
        ),
        (
            Reason::NoTrash(path(), "there is no trash".to_string()),
            "a stays where it is: there is no trash.",
            "a stays where it is: there is no trash.",
        ),
        (
            Reason::Failed(path(), "it is busy".to_string()),
            "a stays where it is: it is busy.",
            "a stays where it is: it is busy.",
        ),
        (
            Reason::Unplugged(path()),
            "the drive of a is not plugged in, so it stays for now.",
            "the drive of a is not plugged in, so it stays for now.",
        ),
    ];
    for (reason, undo_text, redo_text) in cases {
        let retry = matches!(
            reason,
            Reason::NoTrash(..) | Reason::Failed(..) | Reason::Unplugged(_)
        );
        let outcome = |reason: &Reason| {
            if retry {
                Outcome::Stays(reason.clone())
            } else {
                Outcome::Leaves(reason.clone())
            }
        };
        let outcomes = [(0, outcome(&reason))];
        for (direction, text, done, key) in [
            (Direction::Undo, undo_text, "undone", "ctrl+z"),
            (Direction::Redo, redo_text, "redone", "ctrl+shift+z"),
        ] {
            let tail = if retry {
                format!(" {key} tries again.")
            } else {
                format!(" Nothing is left to {}.", ops_text::word(direction))
            };
            assert_eq!(
                note_of(&step, &outcomes, direction, false, false),
                format!("Nothing was {done}: {text}{tail}"),
                "{reason:?} {direction:?}"
            );
        }
    }
}

#[test]
fn step_labels_and_the_next_line_name_the_folder_only_when_it_is_not_this_one() {
    let (mut history, step) = moved_pair();
    assert_eq!(
        ops_text::step_label(&step.action, Path::new("/x/dest")),
        "move of 2 items"
    );
    assert_eq!(
        ops_text::step_label(&step.action, Path::new("/d")),
        "move of 2 items into dest"
    );
    assert_eq!(
        ops_text::next_text(history.top(), Path::new("/d")),
        "Next, ctrl+z undoes the move of 2 items into dest."
    );
    history.drop_step(step.id);
    assert_eq!(
        ops_text::next_text(history.top(), Path::new("/d")),
        "Nothing is left to undo."
    );
    let rename = Action::Rename {
        from: PathBuf::from("/d/a"),
        to: PathBuf::from("/d/b"),
    };
    assert_eq!(
        ops_text::step_label(&rename, Path::new("/d")),
        "rename to b"
    );
    assert_eq!(
        ops_text::step_label(&Action::Trash(vec![PathBuf::from("/d/a")]), Path::new("/d")),
        "move of a to the trash"
    );
}

fn copied_then_undone() -> (tempfile::TempDir, PathBuf, PathBuf, Browser) {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "alpha");
    let mut browser = opened(temp.path());
    copy_into(&mut browser, &file, &dest);
    assert!(undo(&mut browser));
    (temp, dest, file, browser)
}

#[test]
fn redo_brings_the_copy_back_and_undo_takes_it_away_again() {
    let (_temp, dest, file, mut browser) = copied_then_undone();
    assert!(!dest.join("a.txt").exists());
    assert_eq!(
        notice(&browser),
        "Undid the copy of a.txt into dest. The copy went to the trash. ctrl+shift+z redoes it."
    );

    assert!(redo(&mut browser));
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "alpha");
    assert_eq!(fs::read_to_string(&file).unwrap(), "alpha");
    assert_eq!(
        notice(&browser),
        "Redid the copy of a.txt into dest. The copy is in dest again. ctrl+z undoes it."
    );

    assert!(undo(&mut browser));
    assert!(!dest.join("a.txt").exists());
    assert!(redo(&mut browser));
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "alpha");
    assert!(!redo(&mut browser));
    assert_eq!(notice(&browser), "Nothing to redo");
}

#[test]
fn redo_moves_the_file_again_and_undo_brings_it_back() {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "alpha");
    let mut browser = opened(temp.path());
    browser.begin(Transfer {
        verb: Verb::Move,
        sources: vec![file.clone()],
        into: dest.clone(),
    });
    wait_until(&mut browser, "the move", finished);

    assert!(undo(&mut browser));
    assert!(file.exists());
    assert!(redo(&mut browser));
    assert!(!file.exists());
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "alpha");
    assert_eq!(
        notice(&browser),
        "Redid the move of a.txt into dest. It is in dest again. ctrl+z undoes it."
    );

    assert!(undo(&mut browser));
    assert!(file.exists());
    assert_eq!(
        notice(&browser),
        "Undid the move of a.txt into dest. It is back here. ctrl+shift+z redoes it."
    );
}

#[test]
fn redo_renames_again() {
    let temp = tempfile::tempdir().unwrap();
    let file = write(temp.path(), "a.txt", "a");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);
    browser.rename_open();
    browser.rename_key(&key("b", Some("b")), &MemoryClipboard::default());
    browser.rename_key(&key("enter", None), &MemoryClipboard::default());
    assert!(temp.path().join("b.txt").exists());

    assert!(undo(&mut browser));
    assert!(file.exists());
    assert!(redo(&mut browser));
    assert!(!file.exists());
    assert!(temp.path().join("b.txt").exists());
    assert_eq!(
        notice(&browser),
        "Redid the rename to b.txt. It is named b.txt again. ctrl+z undoes it."
    );
}

#[test]
fn redo_trashes_again_and_makes_new_items_again() {
    let temp = tempfile::tempdir().unwrap();
    let file = write(temp.path(), "a.txt", "text");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);
    assert!(browser.trash_picks());
    wait_notice(
        &mut browser,
        "the trash",
        "1 moved to the trash. ctrl+z puts it back.",
    );
    assert!(undo(&mut browser));
    assert!(file.exists());
    assert!(redo(&mut browser));
    assert!(!file.exists());
    assert_eq!(
        notice(&browser),
        "Redid the move of a.txt to the trash. It is in the trash again. ctrl+z undoes it."
    );

    assert!(browser.new_folder());
    let made = temp.path().join("New folder");
    assert!(undo(&mut browser));
    assert!(!made.exists());
    assert!(redo(&mut browser));
    assert!(made.is_dir());
    assert_eq!(
        notice(&browser),
        "Redid the new folder New folder. The new folder New folder is back. ctrl+z undoes it."
    );
}

#[test]
fn there_is_nothing_to_redo_until_something_was_undone_and_a_new_step_clears_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut browser = opened(temp.path());
    assert!(!redo(&mut browser));
    assert_eq!(notice(&browser), "Nothing to redo");

    assert!(browser.new_folder());
    assert!(!redo(&mut browser));
    assert_eq!(notice(&browser), "Nothing to redo");
    assert!(undo(&mut browser));
    assert!(browser.new_folder());
    assert!(!redo(&mut browser));
    assert_eq!(notice(&browser), "Nothing to redo");
}

#[test]
fn another_windows_job_refuses_the_redo_and_touches_nothing() {
    let (_temp, dest, _file, mut browser) = copied_then_undone();

    let other_window = browser.ops.tools.gate.share();
    assert!(other_window.is_some());
    assert!(browser.redo(false));
    assert!(!browser.undo_running());
    assert_eq!(
        notice(&browser),
        "Nothing was redone. Press ctrl+shift+z again when the move in another window ends."
    );
    assert!(!dest.join("a.txt").exists());

    drop(other_window);
    assert!(redo(&mut browser));
    assert!(dest.join("a.txt").exists());
}

#[test]
fn redo_waits_for_the_folder_to_load_and_never_queues() {
    let (_temp, dest, _file, mut browser) = copied_then_undone();

    assert!(browser.redo(true));
    assert!(!browser.undo_running());
    assert_eq!(
        notice(&browser),
        "Nothing was redone. Press ctrl+shift+z again once the folder has loaded."
    );
    assert!(!dest.join("a.txt").exists());
}

#[test]
fn undo_and_redo_refuse_while_the_other_one_runs_and_name_which_one() {
    let (_temp, _dest, _file, mut browser) = copied_then_undone();

    assert!(browser.redo(false));
    assert!(browser.undo_running());
    assert!(browser.undo(false));
    assert_eq!(
        notice(&browser),
        "Nothing was undone. Press ctrl+z again when the redo ends."
    );
    assert!(browser.redo(false));
    assert_eq!(
        notice(&browser),
        "Nothing was redone. Press ctrl+shift+z again when the redo ends."
    );
    assert_eq!(
        browser.note().count,
        "Redoing the copy of a.txt into dest. Esc stops it."
    );
    wait_until(&mut browser, "the redo", |browser| !browser.undo_running());

    assert!(browser.undo(false));
    assert!(browser.undo_running());
    assert!(browser.redo(false));
    assert_eq!(
        notice(&browser),
        "Nothing was redone. Press ctrl+shift+z again when the undo ends."
    );
    wait_until(&mut browser, "the undo", |browser| !browser.undo_running());
}

fn moved_undone_and_blocked() -> (tempfile::TempDir, PathBuf, Browser) {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "old");
    let mut browser = opened(temp.path());
    browser.begin(Transfer {
        verb: Verb::Move,
        sources: vec![file],
        into: dest.clone(),
    });
    wait_until(&mut browser, "the move", finished);
    assert!(undo(&mut browser));
    write(&dest, "a.txt", "new");
    (temp, dest, browser)
}

fn asked_again(browser: &mut Browser) {
    assert!(browser.redo(false));
    wait_until(browser, "the redo sheet", |browser| browser.undo_asking());
}

#[test]
fn redo_never_replaces_so_it_asks_and_esc_leaves_the_step_for_the_next_press() {
    let (temp, dest, mut browser) = moved_undone_and_blocked();

    asked_again(&mut browser);
    assert_eq!(
        browser.undo_lines().unwrap(),
        [
            "Redo: a.txt is already in dest.",
            "Redo never replaces anything. Keep both puts it there as a (2).txt."
        ]
    );
    assert!(browser.redo(false));
    assert!(browser.cancel_survey());
    assert!(!browser.undo_asking());
    assert_eq!(
        notice(&browser),
        "Nothing was redone. ctrl+shift+z asks again."
    );
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "new");
    assert!(temp.path().join("a.txt").exists());

    asked_again(&mut browser);
    assert!(browser.confirm());
    wait_until(&mut browser, "the redo", |browser| !browser.undo_running());
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "new");
    assert_eq!(fs::read_to_string(dest.join("a (2).txt")).unwrap(), "old");
    assert!(!temp.path().join("a.txt").exists());
    assert_eq!(
        notice(&browser),
        "Redid the move of a.txt into dest. It is in dest again. ctrl+z undoes it."
    );
}

#[test]
fn keep_this_on_the_redo_sheet_leaves_the_item_and_drops_the_step() {
    let (temp, dest, mut browser) = moved_undone_and_blocked();

    asked_again(&mut browser);
    assert!(browser.choose(0, Resolve::Keep));
    assert!(browser.confirm());
    wait_until(&mut browser, "the redo", |browser| !browser.undo_running());
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "new");
    assert!(temp.path().join("a.txt").exists());
    assert_eq!(
        notice(&browser),
        "Could not redo the move of a.txt into dest. Nothing was redone: a.txt stays where it is, as you chose. Nothing is left to redo."
    );
    assert!(!redo(&mut browser));
    assert_eq!(notice(&browser), "Nothing to redo");
}

fn rows(browser: &Browser) -> Vec<(String, String, MoveState)> {
    browser
        .moves()
        .iter()
        .map(|record| (record.title.clone(), record.detail.clone(), record.state))
        .collect()
}

#[test]
fn every_undo_and_redo_adds_a_row_to_the_moves_list_with_the_full_note() {
    let (_temp, _dest, _file, mut browser) = copied_then_undone();
    let [undone, copied] = <[_; 2]>::try_from(rows(&browser)).unwrap();
    assert_eq!(copied.0, "a.txt to dest");
    assert_eq!(
        undone,
        (
            "Undid the copy of a.txt into dest".to_string(),
            "The copy went to the trash. ctrl+shift+z redoes it.".to_string(),
            MoveState::Done
        )
    );

    assert!(redo(&mut browser));
    assert_eq!(
        rows(&browser)[0],
        (
            "Redid the copy of a.txt into dest".to_string(),
            "The copy is in dest again. ctrl+z undoes it.".to_string(),
            MoveState::Done
        )
    );
    assert_eq!(rows(&browser).len(), 3);
}

#[test]
fn an_undo_posts_one_info_message_titled_like_its_moves_row() {
    let (_temp, _dest, _file, browser) = copied_then_undone();
    let title = rows(&browser)[0].0.clone();

    let posted = browser.posted();

    assert_eq!(posted.len(), 2);
    assert_eq!(posted[0].level, Level::Info);
    assert_eq!(
        posted[1],
        Message::info(title, "The copy went to the trash. ctrl+shift+z redoes it.")
    );
}

#[test]
fn a_trash_that_fails_posts_an_error_and_nothing_about_success() {
    let temp = tempfile::tempdir().unwrap();
    let file = write(temp.path(), "a.txt", "text");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);
    fs::remove_file(&file).unwrap();

    assert!(browser.trash_picks());
    wait_until(&mut browser, "the trash", |browser| {
        browser.ops.trashing == 0
    });

    let posted = browser.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].level, Level::Error);
    assert_eq!(posted[0].title, "Could not move 1 item to the trash");
    assert!(posted[0].body.starts_with("a.txt: "));
}

#[test]
fn an_undo_that_changed_nothing_still_leaves_a_row() {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "alpha");
    let mut browser = opened(temp.path());
    copy_into(&mut browser, &file, &dest);
    fs::write(dest.join("a.txt"), "alpha, then edited").unwrap();

    assert!(undo(&mut browser));
    assert_eq!(
        rows(&browser)[0],
        (
            "Could not undo the copy of a.txt into dest".to_string(),
            "Nothing was undone: a.txt changed, so it stays. Nothing is left to undo.".to_string(),
            MoveState::Failed
        )
    );
}

#[test]
fn dropping_a_step_that_changed_nothing_and_a_delete_that_cannot_be_undone_leave_rows() {
    let temp = tempfile::tempdir().unwrap();
    let file = write(temp.path(), "a.txt", "text");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);
    assert!(browser.erase_ask());
    assert!(browser.erase_cancel());
    assert!(browser.erase_ask());
    assert!(browser.erase_confirm());
    wait_until(&mut browser, "the delete", |_| !file.exists());
    wait_notice(&mut browser, "the note", "1 item deleted for good");

    assert!(undo(&mut browser));
    assert_eq!(
        rows(&browser)[0],
        (
            "Cannot undo the permanent delete of a.txt".to_string(),
            "Delete permanently cannot be undone. Next is the permanent delete of a.txt, which changed nothing."
                .to_string(),
            MoveState::Failed
        )
    );
    assert!(undo(&mut browser));
    assert_eq!(
        rows(&browser)[0],
        (
            "The permanent delete of a.txt changed nothing".to_string(),
            "The permanent delete of a.txt changed nothing. Nothing is left to undo.".to_string(),
            MoveState::Done
        )
    );
}

#[test]
fn the_menu_words_follow_the_history() {
    let temp = tempfile::tempdir().unwrap();
    let mut browser = opened(temp.path());
    assert_eq!(browser.stepping(), Stepping::default());

    assert!(browser.new_folder());
    assert_eq!(
        browser.stepping(),
        Stepping {
            undo: Some("new folder"),
            redo: None
        }
    );
    assert!(undo(&mut browser));
    assert_eq!(
        browser.stepping(),
        Stepping {
            undo: None,
            redo: Some("new folder")
        }
    );
    assert!(redo(&mut browser));
    assert_eq!(
        browser.stepping(),
        Stepping {
            undo: Some("new folder"),
            redo: None
        }
    );
}

#[test]
fn each_kind_of_step_has_a_word_for_the_menu() {
    let at = |name: &str| PathBuf::from("/d").join(name);
    let transfer = |verb| Action::Transfer {
        verb,
        sources: vec![at("a")],
        into: at("b"),
    };
    let cases = [
        (transfer(Verb::Copy), "copy"),
        (transfer(Verb::Move), "move"),
        (transfer(Verb::Duplicate), "duplicate"),
        (
            Action::Rename {
                from: at("a"),
                to: at("b"),
            },
            "rename",
        ),
        (Action::Trash(vec![at("a")]), "move to trash"),
        (Action::NewFolder(at("a")), "new folder"),
        (Action::NewFile(at("a")), "new file"),
        (Action::Erase(vec![at("a")]), "permanent delete"),
    ];
    for (action, word) in cases {
        assert_eq!(ops_text::menu_word(&action), word, "{action:?}");
    }
}

fn steps_in(browser: &Browser) -> usize {
    browser.ops.history.steps().len()
}

#[test]
fn a_copy_then_a_drag_onto_the_same_folder_leaves_one_empty_step_so_undo_names_the_copy() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("Test 1");
    let target = temp.path().join("Untitled Folder");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    let mut browser = opened(temp.path());

    browser.click_to(row(&browser, "Test 1"), 0.0);
    assert!(browser.hold(Held::Copied));
    browser.click_to(row(&browser, "Untitled Folder"), 0.0);
    let pasted = browser.paste().unwrap();
    assert_eq!(pasted.into, target);
    browser.begin(pasted);
    wait_until(&mut browser, "the copy", finished);
    assert!(target.join("Test 1").is_dir());
    assert_eq!(steps_in(&browser), 1);

    let dropped = browser.drop_on(vec![source.clone()], target.clone(), false);
    assert_eq!(dropped.verb, Verb::Move);
    browser.begin(dropped);
    wait_until(&mut browser, "the sheet", |browser| {
        browser.surveyed().is_some()
    });
    assert_eq!(steps_in(&browser), 2);
    assert!(browser.choose(0, Resolve::Keep));
    assert!(browser.confirm());
    wait_notice(
        &mut browser,
        "the move",
        "Nothing changed in Untitled Folder",
    );
    assert!(source.is_dir());
    assert!(target.join("Test 1").is_dir());
    assert_eq!(steps_in(&browser), 2);

    assert!(undo(&mut browser));
    assert_eq!(
        notice(&browser),
        "The move of Test 1 into Untitled Folder changed nothing. Next, ctrl+z undoes the copy of Test 1 into Untitled Folder."
    );
    assert_eq!(steps_in(&browser), 1);
    assert!(source.is_dir());
    assert!(target.join("Test 1").is_dir());
    assert!(binned().is_empty());
}

#[test]
fn a_second_begin_before_the_first_is_answered_replaces_it_and_leaves_no_stray_step() {
    let (temp, dest) = two_folders();
    let file = write(temp.path(), "a.txt", "x");
    let mut browser = opened(temp.path());
    let copy = |file: &Path| Transfer {
        verb: Verb::Copy,
        sources: vec![file.to_path_buf()],
        into: dest.clone(),
    };
    browser.begin(copy(&file));
    browser.begin(copy(&file));
    assert_eq!(steps_in(&browser), 1);
    wait_until(&mut browser, "the copy", finished);
    assert_eq!(steps_in(&browser), 1);
    assert_eq!(browser.moves().len(), 1);
}

#[test]
fn a_new_begin_replaces_a_sheet_nobody_answered_without_leaving_a_step() {
    let (temp, dest) = two_folders();
    let first = write(temp.path(), "a.txt", "incoming");
    let second = write(temp.path(), "b.txt", "other");
    write(&dest, "a.txt", "existing");
    let mut browser = opened(temp.path());
    browser.begin(Transfer {
        verb: Verb::Copy,
        sources: vec![first],
        into: dest.clone(),
    });
    wait_until(&mut browser, "the sheet", |browser| {
        browser.surveyed().is_some()
    });
    assert_eq!(steps_in(&browser), 1);

    browser.begin(Transfer {
        verb: Verb::Copy,
        sources: vec![second],
        into: dest.clone(),
    });
    assert!(browser.surveyed().is_none());
    wait_until(&mut browser, "the copy", finished);
    assert_eq!(steps_in(&browser), 1);
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "existing");
    assert!(dest.join("b.txt").exists());
}
