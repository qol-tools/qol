use super::super::components::SliderStyle;
use super::numbers::SCRUB_SLOP;
use super::{number_key, NumberEdit, NumberKey, NumberRelease, NumberSlider, NumberSpec, Scrub};

const HOLD: NumberSpec = NumberSpec {
    value: 1000.0,
    min: Some(0.0),
    max: Some(3000.0),
    step: Some(50.0),
};

fn step(direction: f64, coarse: bool) -> NumberKey {
    NumberKey::Step { direction, coarse }
}

fn typed(text: &str) -> NumberKey {
    NumberKey::Type(text.to_string())
}

fn edit_at(text: &str) -> NumberEdit {
    let mut edit = NumberEdit::with_text("");
    edit.insert(text);
    edit
}

#[test]
fn keys_step_and_reach_the_bounds_of_the_number() {
    let cases = [
        ("1000", step(1.0, false), "1050", true),
        ("1000", step(-1.0, false), "950", true),
        ("1000", step(1.0, true), "1500", true),
        ("1000", step(-1.0, true), "500", true),
        ("2950", step(1.0, true), "3000", true),
        ("3000", step(1.0, false), "3000", false),
        ("0", step(-1.0, false), "0", false),
        ("0", step(-1.0, true), "0", false),
        ("", step(1.0, false), "1050", true),
        ("1000", NumberKey::Lowest, "0", true),
        ("1000", NumberKey::Highest, "3000", true),
        ("0", NumberKey::Lowest, "0", false),
        ("3000", NumberKey::Highest, "3000", false),
        ("1000", NumberKey::Apply, "1000", false),
        ("1000", NumberKey::Restore, "1000", false),
    ];
    for (start, key, text, glows) in cases {
        let mut edit = NumberEdit::with_text(start);
        assert_eq!(edit.press(&key, &HOLD), glows, "{start} {key:?}");
        assert_eq!(edit.text(), text, "{start} {key:?}");
        assert!(!edit.capped(), "{start} {key:?}");
    }
}

#[test]
fn the_bounds_keys_land_on_a_step_inside_the_range() {
    let odd = NumberSpec {
        value: 10.0,
        min: Some(3.0),
        max: Some(99.0),
        step: Some(5.0),
    };
    let cases = [
        (NumberKey::Lowest, "3", 3.0),
        (NumberKey::Highest, "98", 98.0),
    ];
    for (key, text, value) in cases {
        let mut edit = NumberEdit::begin(odd.value);
        edit.press(&key, &odd);
        assert_eq!(edit.text(), text, "{key:?}");
        assert_eq!(edit.value(&odd), Some(value), "{key:?}");
    }
    let open = NumberSpec {
        value: 4.0,
        min: None,
        max: None,
        step: None,
    };
    for key in [NumberKey::Lowest, NumberKey::Highest] {
        let mut edit = NumberEdit::begin(open.value);
        assert!(!edit.press(&key, &open), "{key:?}");
        assert_eq!(edit.text(), "4", "{key:?}");
    }
}

#[test]
fn typing_replaces_the_selected_number_and_previews_it() {
    let cases: [(&[&str], &str, Option<f64>, bool); 8] = [
        (&["1", "5", "0", "0"], "1500", Some(1500.0), false),
        (&["0"], "0", Some(0.0), false),
        (&["2", "5"], "25", Some(50.0), false),
        (&["4", "0", "0", "0"], "3000", Some(3000.0), true),
        (&["9", "9", "9", "9", "9"], "3000", Some(3000.0), true),
        (&["a"], "1000", Some(1000.0), false),
        (&["-", "5"], "5", Some(0.0), false),
        (&["1", ".", "5"], "1.5", Some(0.0), false),
    ];
    for (keys, text, value, capped) in cases {
        let mut edit = NumberEdit::begin(HOLD.value);
        for key in keys {
            edit.press(&typed(key), &HOLD);
        }
        assert_eq!(edit.text(), text, "{keys:?}");
        assert_eq!(edit.value(&HOLD), value, "{keys:?}");
        assert_eq!(edit.capped(), capped, "{keys:?}");
    }
}

#[test]
fn typing_glows_only_when_the_preview_moves() {
    let cases = [
        ("1000", "5", true),
        ("1000", "x", false),
        ("15", "0", true),
        ("150", "0", true),
        ("300", "0", true),
        ("3000", "0", false),
    ];
    for (start, key, glows) in cases {
        let mut edit = edit_at(start);
        assert_eq!(edit.press(&typed(key), &HOLD), glows, "{start} + {key}");
    }
}

#[test]
fn erasing_drops_a_digit_and_clears_the_cap() {
    let mut edit = NumberEdit::begin(HOLD.value);
    for key in ["9", "9", "9", "9"] {
        edit.press(&typed(key), &HOLD);
    }
    assert!(edit.capped());
    assert!(edit.press(&NumberKey::Erase, &HOLD));
    assert_eq!(edit.text(), "300");
    assert!(!edit.capped());
    let mut selected = NumberEdit::begin(1500.0);
    assert!(selected.press(&NumberKey::Erase, &HOLD));
    assert_eq!(selected.text(), "");
    assert_eq!(selected.value(&HOLD), None);
    assert_eq!(selected.preview(&HOLD), HOLD.value);
}

#[test]
fn an_edit_shows_a_caret_once_the_selection_is_gone() {
    let mut edit = NumberEdit::begin(1500.0);
    assert!(edit.selects_all());
    assert_eq!(edit.display(), "1500");
    edit.press(&typed("2"), &HOLD);
    assert!(!edit.selects_all());
    assert_eq!(edit.display(), "2_");
    edit.select_all();
    assert!(edit.selects_all());
    assert_eq!(edit.display(), "2");
    assert_eq!(edit.clone().into_text(), "2");
}

#[test]
fn applying_reads_the_typed_value_and_restoring_keeps_the_old_one() {
    let cases: [(&str, Option<f64>); 7] = [
        ("1500", Some(1500.0)),
        ("0", Some(0.0)),
        ("1520", Some(1500.0)),
        ("1530", Some(1550.0)),
        ("99999", Some(3000.0)),
        ("", None),
        ("abc", None),
    ];
    for (text, applied) in cases {
        let edit = NumberEdit::with_text(text);
        assert_eq!(edit.value(&HOLD), applied, "{text}");
        assert_eq!(edit.preview(&HOLD), applied.unwrap_or(HOLD.value), "{text}");
    }
}

#[test]
fn keys_map_to_number_keys() {
    let cases = [
        ("left", None, false, Some(step(-1.0, false))),
        ("right", None, false, Some(step(1.0, false))),
        ("left", None, true, Some(step(-1.0, true))),
        ("right", None, true, Some(step(1.0, true))),
        ("home", None, false, Some(NumberKey::Lowest)),
        ("end", None, true, Some(NumberKey::Highest)),
        ("enter", None, false, Some(NumberKey::Apply)),
        ("return", None, false, Some(NumberKey::Apply)),
        ("escape", None, false, Some(NumberKey::Restore)),
        ("esc", None, false, Some(NumberKey::Restore)),
        ("backspace", None, false, Some(NumberKey::Erase)),
        ("5", Some("5"), false, Some(typed("5"))),
        (".", Some("."), false, Some(typed("."))),
        ("up", None, false, None),
        ("down", None, false, None),
        ("tab", None, false, None),
        ("space", Some(""), false, None),
    ];
    for (key, key_char, shift, expected) in cases {
        assert_eq!(number_key(key, key_char, shift), expected, "{key}");
    }
}

#[test]
fn a_press_becomes_a_scrub_only_past_the_slop() {
    let width = SliderStyle::Compact.track_width();
    let near = 0.5 + (SCRUB_SLOP - 1.0) / width;
    let far = 0.5 + (SCRUB_SLOP + 1.0) / width;
    let mut slider = NumberSlider::<usize>::default();
    assert_eq!(slider.scrub(far), Scrub::Still, "no press");
    slider.press(0.5);
    assert_eq!(slider.scrub(near), Scrub::Still);
    assert_eq!(slider.scrub(far), Scrub::Began);
    assert_eq!(slider.scrub(near), Scrub::Moving);
    assert_eq!(slider.scrub(0.0), Scrub::Moving);
    slider.clear();
    assert_eq!(slider.scrub(far), Scrub::Still, "cleared");
}

#[test]
fn a_release_finishes_a_scrub_and_edits_after_a_press() {
    let edit = |commit, begin| NumberRelease::Edit { commit, begin };
    let far = 0.5 + (SCRUB_SLOP + 1.0) / SliderStyle::Compact.track_width();
    let cases = [
        (true, 2, 2, false, NumberRelease::Finish),
        (true, 2, 1, true, NumberRelease::Finish),
        (false, 2, 2, false, edit(false, true)),
        (false, 2, 2, true, edit(false, false)),
        (false, 2, 1, true, edit(true, true)),
        (false, 2, 1, false, edit(true, true)),
    ];
    for (scrubbed, row, selected, editing, expected) in cases {
        let mut slider = NumberSlider::<usize>::default();
        slider.press(0.5);
        if scrubbed {
            slider.scrub(far);
        }
        slider.follow(Some(far));
        assert_eq!(
            slider.release(row, selected, editing),
            expected,
            "scrubbed {scrubbed} row {row} selected {selected} editing {editing}"
        );
        assert_eq!(slider.scrub, None);
        assert_eq!(slider.press, None);
    }
}

#[test]
fn a_shift_drag_pulls_four_times_from_where_it_started() {
    let bounds = (0.0, 3000.0, 50.0);
    let mut slider = NumberSlider::<usize>::default();
    assert_eq!(slider.drag_value(&1, 0.5, true, bounds), 1500.0, "no start");
    slider.begin_drag(Some((1, 1000.0)));
    let cases = [
        (1, 0.5, false, 1500.0),
        (1, 0.5, true, 3000.0),
        (1, 0.35, true, 1200.0),
        (1, 0.0, true, 0.0),
        (2, 0.5, true, 1500.0),
        (1, 0.01, false, 50.0),
    ];
    for (key, fraction, coarse, value) in cases {
        assert_eq!(
            slider.drag_value(&key, fraction, coarse, bounds),
            value,
            "key {key} fraction {fraction} coarse {coarse}"
        );
    }
    assert_eq!(slider.drag_start(), Some(&(1, 1000.0)));
    assert_eq!(slider.take_drag_start(), Some((1, 1000.0)));
    assert_eq!(slider.take_drag_start(), None);
}

#[test]
fn the_pointer_marks_one_field_and_only_its_own_leave_clears_it() {
    let mut slider = NumberSlider::<usize>::default();
    slider.pointer_over(1, true);
    slider.pointer_over(2, false);
    assert_eq!(slider.hover, Some(1));
    slider.pointer_over(2, true);
    slider.pointer_over(1, false);
    assert_eq!(slider.hover, Some(2));
    slider.pointer_over(2, false);
    assert_eq!(slider.hover, None);
}

#[test]
fn only_the_newest_key_glow_ends_it() {
    let mut slider = NumberSlider::<usize>::default();
    assert!(!slider.end_glow(0));
    slider.key_glow = Some((4, 2));
    assert!(slider.glowing(&4));
    assert!(!slider.glowing(&3));
    assert!(!slider.end_glow(1));
    assert!(slider.glowing(&4));
    assert!(slider.end_glow(2));
    assert!(!slider.glowing(&4));
    slider.key_glow = Some((4, 3));
    slider.clear();
    assert!(!slider.glowing(&4));
}

#[test]
fn a_host_drag_scrubs_from_the_pressed_value_and_follows_it() {
    let near = 0.5 + (SCRUB_SLOP - 1.0) / SliderStyle::Compact.track_width();
    let mut slider = NumberSlider::<usize>::default();
    assert_eq!(slider.drag(4, 0.75, false, &HOLD), None, "no press");
    slider.press(0.5);
    assert_eq!(slider.drag(4, near, false, &HOLD), None, "inside the slop");
    assert_eq!(slider.drag_start(), None);
    let cases = [
        (0.75, false, Scrub::Began, 2250.0, 0.75),
        (0.3, true, Scrub::Moving, 600.0, 0.2),
        (1.0, false, Scrub::Moving, 3000.0, 1.0),
        (0.0, false, Scrub::Moving, 0.0, 0.0),
    ];
    for (fraction, coarse, scrub, value, follows) in cases {
        assert_eq!(
            slider.drag(4, fraction, coarse, &HOLD),
            Some((scrub, value)),
            "fraction {fraction} coarse {coarse}"
        );
        assert_eq!(slider.scrub, Some(follows), "fraction {fraction}");
        assert_eq!(slider.drag_start(), Some(&(4, HOLD.value)));
    }
    let open = NumberSpec {
        value: 4.0,
        min: None,
        max: Some(10.0),
        step: None,
    };
    let mut unbounded = NumberSlider::<usize>::default();
    unbounded.press(0.5);
    assert_eq!(unbounded.drag(4, 1.0, false, &open), None);
}
