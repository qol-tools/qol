use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use gpui::{Keystroke, Modifiers};

use super::ops_text;
use super::{Browser, Held, MoveState};
use crate::ops::{Clash, CopyOnly, Resolve, Room, Stamp, Survey, Transfer, Verb};

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
    wait_until(&mut browser, "the listing", |browser| !browser.reading());
    browser
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
    assert_eq!(browser.operand_count(), 1);

    browser.add_pick(browser.row_of(&first).unwrap());
    browser.add_pick(browser.row_of(&second).unwrap());
    assert_eq!(browser.operand_count(), 2);

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
    assert!(browser.note().count.starts_with("1 moved here from "));
    assert!(browser.note().count.ends_with("ctrl+z undoes it."));
    assert!(browser.paste().is_none());

    assert!(browser.undo());
    wait_until(&mut browser, "the undo", |_| file.exists());
    assert!(!dest.join("a.txt").exists());
}

#[test]
fn copy_then_paste_leaves_the_original_and_undo_has_a_journal_entry() {
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
    assert!(browser.note().count.starts_with("1 copied here from "));
    assert!(browser.paste().is_some());
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
        browser.note().failure.as_deref(),
        Some("Already in this folder")
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

    assert!(browser.rename_key(&key("b", Some("b"))));
    assert!(browser.rename_key(&key("enter", None)));
    assert!(browser.rename_field().is_none());
    assert!(temp.path().join("b.txt").exists());
    assert!(!file.exists());

    assert!(browser.undo());
    assert!(file.exists());
    assert!(!temp.path().join("b.txt").exists());
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
    browser.rename_key(&key("b", Some("b")));
    browser.rename_key(&key("enter", None));

    assert!(browser.rename_field().is_some());
    assert!(browser
        .note()
        .failure
        .is_some_and(|failure| failure.contains("already exists")));
    assert!(file.exists());
}

#[test]
fn escape_closes_the_rename_field_and_an_unchanged_name_renames_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let file = write(temp.path(), "a.txt", "a");
    let mut browser = opened(temp.path());
    point_at(&mut browser, &file);

    browser.rename_open();
    assert!(browser.rename_key(&key("escape", None)));
    assert!(browser.rename_field().is_none());

    browser.rename_open();
    assert!(browser.rename_key(&key("enter", None)));
    assert!(browser.rename_field().is_none());
    assert!(file.exists());
    assert!(!browser.rename_key(&key("enter", None)));
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

    browser.rename_key(&key("escape", None));
    assert!(browser.new_folder());
    wait_until(&mut browser, "the second field", |browser| {
        browser.rename_field().is_some()
    });
    assert!(temp.path().join("New folder (2)").is_dir());
}

#[test]
fn undo_removes_an_empty_new_folder_and_says_so_when_there_is_nothing_to_undo() {
    let temp = tempfile::tempdir().unwrap();
    let mut browser = opened(temp.path());
    assert!(!browser.undo());
    assert_eq!(browser.note().failure.as_deref(), Some("Nothing to undo"));

    browser.new_folder();
    assert!(browser.undo());
    assert!(!temp.path().join("New folder").exists());
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
        ops_text::held_note(Held::Cut, 4),
        "4 cut. Open a folder and press ctrl+v"
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
        ops_text::arrived_note(Verb::Move, 4, true, "dest", "plans", true),
        "4 moved here from plans. ctrl+z undoes it."
    );
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
}
