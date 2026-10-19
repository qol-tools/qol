use gpui::{Keystroke, Modifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
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
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    Cursor,
    AddPick,
    Open,
}

pub fn action(keystroke: &Keystroke, path_open: bool) -> Action {
    if path_open {
        return Action::PathField;
    }
    let modifiers = &keystroke.modifiers;
    let secondary = modifiers.secondary();
    let chord = (secondary, modifiers.alt, modifiers.shift);
    match (keystroke.key.as_str(), chord) {
        ("up", (false, false, false)) => Action::Cursor(Step::Up),
        ("down", (false, false, false)) => Action::Cursor(Step::Down),
        ("pageup", (false, false, false)) => Action::Cursor(Step::PageUp),
        ("pagedown", (false, false, false)) => Action::Cursor(Step::PageDown),
        ("home", (false, false, false)) => Action::Cursor(Step::Home),
        ("end", (false, false, false)) => Action::Cursor(Step::End),
        ("up", (false, false, true)) => Action::Range(Step::Up),
        ("down", (false, false, true)) => Action::Range(Step::Down),
        ("pageup", (false, false, true)) => Action::Range(Step::PageUp),
        ("pagedown", (false, false, true)) => Action::Range(Step::PageDown),
        ("home", (false, false, true)) => Action::Range(Step::Home),
        ("end", (false, false, true)) => Action::Range(Step::End),
        ("enter", (false, false, false)) => Action::Open,
        ("backspace", (false, false, false)) | ("up", (false, true, false)) => Action::Up,
        ("left", (false, true, false)) => Action::Back,
        ("right", (false, true, false)) => Action::Forward,
        ("space", (false, false, false)) => Action::Pick,
        ("escape", (false, false, false)) => Action::Unpick,
        ("a", (true, false, false)) => Action::PickAll,
        ("h", (true, false, false)) => Action::ToggleHidden,
        ("g", (true, false, false)) => Action::EditPath,
        _ => typed(keystroke).map_or(Action::Nothing, Action::Jump),
    }
}

pub fn click(modifiers: &Modifiers, clicks: usize) -> Click {
    if modifiers.secondary() {
        return Click::AddPick;
    }
    if clicks >= 2 {
        return Click::Open;
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

    use super::{action, click, Action, Click, Step};

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
            key("left", Modifiers::none(), None),
            key("right", Modifiers::none(), None),
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
    fn every_key_goes_to_the_path_field_while_it_is_open() {
        for keystroke in [
            plain("up"),
            plain("escape"),
            plain("enter"),
            key("a", Modifiers::none(), Some("a")),
            key("g", Modifiers::secondary_key(), Some("g")),
        ] {
            assert_eq!(action(&keystroke, true), Action::PathField, "{keystroke:?}");
        }
    }

    #[test]
    fn clicks_move_the_cursor_add_a_pick_or_open() {
        assert_eq!(click(&Modifiers::none(), 1), Click::Cursor);
        assert_eq!(click(&Modifiers::none(), 2), Click::Open);
        assert_eq!(click(&Modifiers::secondary_key(), 1), Click::AddPick);
        assert_eq!(click(&Modifiers::secondary_key(), 2), Click::AddPick);
        assert_eq!(click(&shift(), 1), Click::Cursor);
    }
}
