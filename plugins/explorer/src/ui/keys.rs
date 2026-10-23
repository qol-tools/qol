use gpui::{Keystroke, Modifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resize {
    Bigger,
    Smaller,
    To(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Cursor(Step),
    Range(Step),
    Open,
    Up,
    Back,
    Forward,
    Jump(char),
    Pick,
    PickAll,
    Unpick,
    ToggleHidden,
    EditPath,
    PathField,
    TogglePin,
    Settings,
    Resize(Resize),
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    Cursor,
    Range,
    AddPick,
    Open,
}

pub fn action(keystroke: &Keystroke, path_open: bool) -> Action {
    let modifiers = &keystroke.modifiers;
    let secondary = modifiers.secondary();
    let chord = (secondary, modifiers.alt, modifiers.shift);
    if path_open {
        return match (keystroke.key.as_str(), chord) {
            ("h", (true, false, false)) => Action::ToggleHidden,
            _ => Action::PathField,
        };
    }
    if let Some(resize) = resize(keystroke.key.as_str(), chord) {
        return Action::Resize(resize);
    }
    match (keystroke.key.as_str(), chord) {
        ("up", (false, false, false)) => Action::Cursor(Step::Up),
        ("down", (false, false, false)) => Action::Cursor(Step::Down),
        ("pageup", (false, false, false)) => Action::Cursor(Step::PageUp),
        ("pagedown", (false, false, false)) => Action::Cursor(Step::PageDown),
        ("home", (false, false, false)) => Action::Cursor(Step::Home),
        ("end", (false, false, false)) => Action::Cursor(Step::End),
        ("left", (false, false, false)) => Action::Cursor(Step::Left),
        ("right", (false, false, false)) => Action::Cursor(Step::Right),
        ("up", (false, false, true)) => Action::Range(Step::Up),
        ("down", (false, false, true)) => Action::Range(Step::Down),
        ("pageup", (false, false, true)) => Action::Range(Step::PageUp),
        ("pagedown", (false, false, true)) => Action::Range(Step::PageDown),
        ("home", (false, false, true)) => Action::Range(Step::Home),
        ("end", (false, false, true)) => Action::Range(Step::End),
        ("left", (false, false, true)) => Action::Range(Step::Left),
        ("right", (false, false, true)) => Action::Range(Step::Right),
        ("enter", (false, false, false)) => Action::Open,
        ("backspace", (false, false, false)) | ("up", (false, true, false)) => Action::Up,
        ("left", (false, true, false)) => Action::Back,
        ("right", (false, true, false)) => Action::Forward,
        ("space", (false, false, false)) => Action::Pick,
        ("escape", (false, false, false)) => Action::Unpick,
        ("a", (true, false, false)) => Action::PickAll,
        ("h", (true, false, false)) => Action::ToggleHidden,
        ("g", (true, false, false)) => Action::EditPath,
        ("d", (true, false, false)) => Action::TogglePin,
        (",", (true, false, false)) => Action::Settings,
        _ => typed(keystroke).map_or(Action::Nothing, Action::Jump),
    }
}

fn resize(key: &str, chord: (bool, bool, bool)) -> Option<Resize> {
    match (key, chord) {
        ("=" | "+", (true, false, _)) => Some(Resize::Bigger),
        ("-", (true, false, false)) => Some(Resize::Smaller),
        (digit, (true, false, false)) => match digit.parse::<usize>() {
            Ok(stop @ 1..=5) => Some(Resize::To(stop - 1)),
            _ => None,
        },
        _ => None,
    }
}

pub fn leaves(action: Action) -> bool {
    matches!(action, Action::Up | Action::Back | Action::Forward)
}

pub fn waits(action: Action, arriving: bool, stalled: bool, queued: bool) -> bool {
    if stalled && leaves(action) {
        return false;
    }
    if queued {
        return action != Action::Nothing;
    }
    arriving
        && matches!(
            action,
            Action::Cursor(_)
                | Action::Range(_)
                | Action::Open
                | Action::Jump(_)
                | Action::Pick
                | Action::PickAll
        )
}

pub fn click(modifiers: &Modifiers, clicks: usize) -> Click {
    if modifiers.secondary() {
        return Click::AddPick;
    }
    if clicks >= 2 {
        return Click::Open;
    }
    if modifiers.shift {
        return Click::Range;
    }
    Click::Cursor
}

fn typed(keystroke: &Keystroke) -> Option<char> {
    let modifiers = &keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return None;
    }
    let mut chars = keystroke.key_char.as_deref()?.chars();
    let typed = chars.next()?;
    if chars.next().is_some() || typed.is_control() || typed.is_whitespace() {
        return None;
    }
    Some(typed)
}

#[cfg(test)]
mod tests {
    use gpui::{Keystroke, Modifiers};

    use super::{action, click, leaves, waits, Action, Click, Resize, Step};

    fn key(key: &str, modifiers: Modifiers, key_char: Option<&str>) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: key_char.map(str::to_string),
        }
    }

    fn plain(name: &str) -> Keystroke {
        key(name, Modifiers::none(), None)
    }

    fn shift() -> Modifiers {
        Modifiers {
            shift: true,
            ..Modifiers::none()
        }
    }

    fn alt() -> Modifiers {
        Modifiers {
            alt: true,
            ..Modifiers::none()
        }
    }

    fn control() -> Modifiers {
        Modifiers {
            control: true,
            ..Modifiers::none()
        }
    }

    #[test]
    fn every_browse_binding_maps_to_its_action() {
        let cases = [
            (plain("up"), Action::Cursor(Step::Up)),
            (plain("down"), Action::Cursor(Step::Down)),
            (key("up", shift(), None), Action::Range(Step::Up)),
            (key("down", shift(), None), Action::Range(Step::Down)),
            (plain("pageup"), Action::Cursor(Step::PageUp)),
            (plain("pagedown"), Action::Cursor(Step::PageDown)),
            (plain("home"), Action::Cursor(Step::Home)),
            (plain("end"), Action::Cursor(Step::End)),
            (key("pageup", shift(), None), Action::Range(Step::PageUp)),
            (
                key("pagedown", shift(), None),
                Action::Range(Step::PageDown),
            ),
            (key("home", shift(), None), Action::Range(Step::Home)),
            (key("end", shift(), None), Action::Range(Step::End)),
            (plain("left"), Action::Cursor(Step::Left)),
            (plain("right"), Action::Cursor(Step::Right)),
            (key("left", shift(), None), Action::Range(Step::Left)),
            (key("right", shift(), None), Action::Range(Step::Right)),
            (plain("enter"), Action::Open),
            (plain("backspace"), Action::Up),
            (key("up", alt(), None), Action::Up),
            (key("left", alt(), None), Action::Back),
            (key("right", alt(), None), Action::Forward),
            (key("space", Modifiers::none(), Some(" ")), Action::Pick),
            (plain("escape"), Action::Unpick),
            (
                key("a", Modifiers::secondary_key(), Some("a")),
                Action::PickAll,
            ),
            (
                key("h", Modifiers::secondary_key(), Some("h")),
                Action::ToggleHidden,
            ),
            (
                key("g", Modifiers::secondary_key(), Some("g")),
                Action::EditPath,
            ),
            (
                key("d", Modifiers::secondary_key(), Some("d")),
                Action::TogglePin,
            ),
            (
                key(",", Modifiers::secondary_key(), Some(",")),
                Action::Settings,
            ),
        ];
        for (keystroke, expected) in cases {
            assert_eq!(action(&keystroke, false), expected, "{keystroke:?}");
        }
    }

    #[test]
    fn printable_characters_type_to_jump() {
        let cases = [
            (key("a", Modifiers::none(), Some("a")), Action::Jump('a')),
            (key("a", shift(), Some("A")), Action::Jump('A')),
            (key("2", Modifiers::none(), Some("2")), Action::Jump('2')),
            (key(".", Modifiers::none(), Some(".")), Action::Jump('.')),
            (key("e", Modifiers::none(), Some("é")), Action::Jump('é')),
        ];
        for (keystroke, expected) in cases {
            assert_eq!(action(&keystroke, false), expected, "{keystroke:?}");
        }
    }

    #[test]
    fn chords_and_unbound_keys_do_nothing() {
        let cases = [
            key("x", control(), Some("x")),
            key("x", alt(), Some("x")),
            key("tab", Modifiers::none(), Some("\t")),
            key("f1", Modifiers::none(), None),
            key("enter", shift(), None),
            key("escape", shift(), None),
        ];
        for keystroke in cases {
            assert_eq!(action(&keystroke, false), Action::Nothing, "{keystroke:?}");
        }
    }

    #[test]
    fn size_keys_step_or_pick_a_stop_and_plain_digits_still_jump() {
        let secondary = Modifiers::secondary_key;
        let secondary_shift = || Modifiers {
            shift: true,
            ..Modifiers::secondary_key()
        };
        let cases = [
            (
                key("=", secondary(), Some("=")),
                Action::Resize(Resize::Bigger),
            ),
            (
                key("=", secondary_shift(), Some("+")),
                Action::Resize(Resize::Bigger),
            ),
            (
                key("+", secondary_shift(), Some("+")),
                Action::Resize(Resize::Bigger),
            ),
            (
                key("+", secondary(), Some("+")),
                Action::Resize(Resize::Bigger),
            ),
            (
                key("-", secondary(), Some("-")),
                Action::Resize(Resize::Smaller),
            ),
            (
                key("1", secondary(), Some("1")),
                Action::Resize(Resize::To(0)),
            ),
            (
                key("3", secondary(), Some("3")),
                Action::Resize(Resize::To(2)),
            ),
            (
                key("5", secondary(), Some("5")),
                Action::Resize(Resize::To(4)),
            ),
            (key("0", secondary(), Some("0")), Action::Nothing),
            (key("6", secondary(), Some("6")), Action::Nothing),
            (key("-", secondary_shift(), Some("_")), Action::Nothing),
            (key("3", Modifiers::none(), Some("3")), Action::Jump('3')),
            (key("=", Modifiers::none(), Some("=")), Action::Jump('=')),
            (key("-", Modifiers::none(), Some("-")), Action::Jump('-')),
        ];
        for (keystroke, expected) in cases {
            assert_eq!(action(&keystroke, false), expected, "{keystroke:?}");
        }
        assert_eq!(
            action(&key("=", secondary(), Some("=")), true),
            Action::PathField
        );
    }

    #[test]
    fn every_key_but_ctrl_h_goes_to_the_path_field_while_it_is_open() {
        for keystroke in [
            plain("up"),
            plain("escape"),
            plain("enter"),
            key("a", Modifiers::none(), Some("a")),
            key("h", Modifiers::none(), Some("h")),
            key("g", Modifiers::secondary_key(), Some("g")),
            key("d", Modifiers::secondary_key(), Some("d")),
            key(",", Modifiers::secondary_key(), Some(",")),
        ] {
            assert_eq!(action(&keystroke, true), Action::PathField, "{keystroke:?}");
        }
        assert_eq!(
            action(&key("h", Modifiers::secondary_key(), Some("h")), true),
            Action::ToggleHidden
        );
    }

    #[test]
    fn keys_on_rows_wait_for_an_arriving_folder_and_everything_waits_behind_them() {
        let on_rows = [
            Action::Cursor(Step::Down),
            Action::Range(Step::End),
            Action::Open,
            Action::Jump('f'),
            Action::Pick,
            Action::PickAll,
        ];
        let leaving = [
            Action::Up,
            Action::Back,
            Action::Forward,
            Action::EditPath,
            Action::Unpick,
            Action::ToggleHidden,
            Action::PathField,
            Action::TogglePin,
            Action::Settings,
        ];
        for action in on_rows {
            assert!(waits(action, true, false, false), "{action:?}");
            assert!(!waits(action, false, false, false), "{action:?}");
        }
        for action in leaving {
            assert!(!waits(action, true, false, false), "{action:?}");
            assert!(waits(action, true, false, true), "{action:?}");
            assert!(waits(action, false, false, true), "{action:?}");
        }
        assert!(!waits(Action::Nothing, true, false, true));
    }

    #[test]
    fn leaving_keys_skip_the_queue_of_a_stalled_folder() {
        for action in [Action::Up, Action::Back, Action::Forward] {
            assert!(leaves(action), "{action:?}");
            assert!(!waits(action, true, true, true), "{action:?}");
        }
        for action in [Action::Open, Action::Cursor(Step::Down), Action::Unpick] {
            assert!(!leaves(action), "{action:?}");
            assert!(waits(action, true, true, true), "{action:?}");
        }
        assert!(waits(Action::Open, true, true, false));
    }

    #[test]
    fn clicks_move_the_cursor_pick_a_range_add_a_pick_or_open() {
        assert_eq!(click(&Modifiers::none(), 1), Click::Cursor);
        assert_eq!(click(&Modifiers::none(), 2), Click::Open);
        assert_eq!(click(&Modifiers::secondary_key(), 1), Click::AddPick);
        assert_eq!(click(&Modifiers::secondary_key(), 2), Click::AddPick);
        assert_eq!(click(&shift(), 1), Click::Range);
        assert_eq!(click(&shift(), 2), Click::Open);
    }
}
