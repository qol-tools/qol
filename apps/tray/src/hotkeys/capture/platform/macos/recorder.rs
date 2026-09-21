use super::MacCombo;
use crate::daemon::{DaemonEvent, EventBus};
use core_graphics::event::CGEventType;
use qol_hotkeys::grammar::{self, Hotkey, Key};
use qol_hotkeys::macos_keycode;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

const KEY_RIGHT_COMMAND: u16 = 0x36;
const KEY_COMMAND: u16 = 0x37;
const KEY_SHIFT: u16 = 0x38;
const KEY_CAPS_LOCK: u16 = 0x39;
const KEY_OPTION: u16 = 0x3A;
const KEY_CONTROL: u16 = 0x3B;
const KEY_RIGHT_SHIFT: u16 = 0x3C;
const KEY_RIGHT_OPTION: u16 = 0x3D;
const KEY_RIGHT_CONTROL: u16 = 0x3E;
const KEY_FUNCTION: u16 = 0x3F;

struct ActiveRecording {
    session_id: u64,
    events: Arc<EventBus>,
}

#[derive(Default)]
pub(super) struct RecorderHub {
    active: Mutex<Option<ActiveRecording>>,
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Ignore,
    Cancel,
    Complete(String),
}

impl RecorderHub {
    pub(super) fn start(&self, session_id: u64, events: Arc<EventBus>) -> bool {
        if !tap_alive() {
            return false;
        }
        *self.lock() = Some(ActiveRecording { session_id, events });
        qol_runtime::probe!("HOTKEY_RECORD_ARM", "session={session_id} backend=cgtap");
        true
    }

    pub(super) fn cancel(&self, session_id: u64) {
        let mut active = self.lock();
        if active
            .as_ref()
            .is_none_or(|recording| recording.session_id != session_id)
        {
            return;
        }
        *active = None;
        qol_runtime::probe!("HOTKEY_RECORD_CANCEL", "session={session_id}");
    }

    pub(super) fn recording(&self) -> bool {
        self.lock().is_some()
    }

    pub(super) fn handle_event(&self, event_type: CGEventType, observed: &MacCombo) {
        let mut active = self.lock();
        let Some(recording) = active.as_ref() else {
            return;
        };
        match decide(event_type, observed) {
            Decision::Ignore => {}
            Decision::Cancel => {
                let (session_id, events) = (recording.session_id, recording.events.clone());
                *active = None;
                events.send(DaemonEvent::HotkeyRecordingCanceled { session_id });
                qol_runtime::probe!("HOTKEY_RECORD_CANCEL", "session={session_id} source=escape");
            }
            Decision::Complete(key) => {
                let (session_id, events) = (recording.session_id, recording.events.clone());
                *active = None;
                qol_runtime::probe!(
                    "HOTKEY_RECORD_COMPLETE",
                    "session={session_id} key={key} backend=cgtap"
                );
                events.send(DaemonEvent::HotkeyRecorded { session_id, key });
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<ActiveRecording>> {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn tap_alive() -> bool {
    super::TAP_PORT.get().is_some() && !super::TAP_RELEASED.load(Ordering::SeqCst)
}

fn decide(event_type: CGEventType, observed: &MacCombo) -> Decision {
    if !matches!(event_type, CGEventType::KeyDown) {
        return Decision::Ignore;
    }
    if is_modifier_keycode(observed.key) {
        return Decision::Ignore;
    }
    if observed.key == macos_keycode::ESCAPE {
        return Decision::Cancel;
    }
    let Some(key) = keycode_to_key(observed.key) else {
        return Decision::Ignore;
    };
    match grammar::format(&Hotkey {
        mods: observed.mods.clone(),
        key,
    }) {
        Some(formatted) => Decision::Complete(formatted),
        None => Decision::Ignore,
    }
}

fn keycode_to_key(code: u16) -> Option<Key> {
    if let Some(key) = grammar::parse_key(macos_keycode::key_name(code)) {
        return Some(key);
    }
    let symbol = match code {
        macos_keycode::ANSI_GRAVE => '`',
        macos_keycode::ANSI_MINUS => '-',
        macos_keycode::ANSI_EQUAL => '+',
        macos_keycode::ANSI_LEFT_BRACKET => '[',
        macos_keycode::ANSI_RIGHT_BRACKET => ']',
        macos_keycode::ANSI_BACKSLASH => '\\',
        macos_keycode::ANSI_SEMICOLON => ';',
        macos_keycode::ANSI_QUOTE => '\'',
        macos_keycode::ANSI_COMMA => ',',
        macos_keycode::ANSI_PERIOD => '.',
        macos_keycode::ANSI_SLASH => '/',
        _ => return None,
    };
    Some(Key::Symbol(symbol))
}

fn is_modifier_keycode(code: u16) -> bool {
    matches!(
        code,
        KEY_RIGHT_COMMAND
            | KEY_COMMAND
            | KEY_SHIFT
            | KEY_CAPS_LOCK
            | KEY_OPTION
            | KEY_CONTROL
            | KEY_RIGHT_SHIFT
            | KEY_RIGHT_OPTION
            | KEY_RIGHT_CONTROL
            | KEY_FUNCTION
    )
}

pub(super) fn global() -> &'static RecorderHub {
    static RECORDER: OnceLock<RecorderHub> = OnceLock::new();
    RECORDER.get_or_init(RecorderHub::default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_hotkeys::grammar::{Modifier, NamedKey};
    use std::collections::BTreeSet;

    fn combo(mods: &[Modifier], key: u16) -> MacCombo {
        MacCombo {
            mods: mods.iter().copied().collect(),
            key,
        }
    }

    #[test]
    fn decides_between_complete_cancel_and_ignore_for_tap_events() {
        let cases = [
            (
                CGEventType::KeyDown,
                combo(&[], macos_keycode::ANSI_R),
                Decision::Complete("R".into()),
            ),
            (
                CGEventType::KeyDown,
                combo(&[Modifier::Shift, Modifier::Super], macos_keycode::ANSI_R),
                Decision::Complete("Shift+Super+R".into()),
            ),
            (
                CGEventType::KeyDown,
                combo(&[Modifier::Super], macos_keycode::ANSI_5),
                Decision::Complete("Super+5".into()),
            ),
            (
                CGEventType::KeyDown,
                combo(&[Modifier::Ctrl], macos_keycode::F8),
                Decision::Complete("Ctrl+F8".into()),
            ),
            (
                CGEventType::KeyDown,
                combo(&[Modifier::Super], macos_keycode::LEFT_ARROW),
                Decision::Complete("Super+Left".into()),
            ),
            (
                CGEventType::KeyDown,
                combo(&[Modifier::Ctrl, Modifier::Alt], macos_keycode::SPACE),
                Decision::Complete("Ctrl+Alt+Space".into()),
            ),
            (
                CGEventType::KeyDown,
                combo(&[Modifier::Super], macos_keycode::ANSI_EQUAL),
                Decision::Complete("Super+Plus".into()),
            ),
            (
                CGEventType::KeyDown,
                combo(&[Modifier::Super], macos_keycode::ANSI_MINUS),
                Decision::Complete("Super+-".into()),
            ),
            (
                CGEventType::KeyDown,
                combo(&[Modifier::Shift], macos_keycode::ANSI_COMMA),
                Decision::Complete("Shift+,".into()),
            ),
            (
                CGEventType::KeyDown,
                combo(&[Modifier::Alt], macos_keycode::ANSI_PERIOD),
                Decision::Complete("Alt+.".into()),
            ),
            (
                CGEventType::KeyDown,
                combo(&[], macos_keycode::ESCAPE),
                Decision::Cancel,
            ),
            (
                CGEventType::KeyDown,
                combo(&[Modifier::Ctrl], macos_keycode::ESCAPE),
                Decision::Cancel,
            ),
            (
                CGEventType::KeyDown,
                combo(&[Modifier::Super], KEY_SHIFT),
                Decision::Ignore,
            ),
            (
                CGEventType::KeyDown,
                combo(&[Modifier::Super], KEY_CAPS_LOCK),
                Decision::Ignore,
            ),
            (
                CGEventType::KeyDown,
                combo(&[Modifier::Super], 0x48),
                Decision::Ignore,
            ),
            (
                CGEventType::KeyDown,
                combo(&[Modifier::Super], u16::MAX),
                Decision::Ignore,
            ),
            (
                CGEventType::KeyUp,
                combo(&[Modifier::Super], macos_keycode::ANSI_R),
                Decision::Ignore,
            ),
            (
                CGEventType::FlagsChanged,
                combo(&[Modifier::Shift], KEY_SHIFT),
                Decision::Ignore,
            ),
        ];
        for (event_type, observed, expected) in cases {
            assert_eq!(
                decide(event_type, &observed),
                expected,
                "key: {} mods: {:?} event: {event_type:?}",
                observed.key,
                observed.mods
            );
        }
    }

    #[test]
    fn records_back_every_key_the_matcher_parse_path_binds() {
        let named = [
            NamedKey::Space,
            NamedKey::Enter,
            NamedKey::Tab,
            NamedKey::Backspace,
            NamedKey::Delete,
            NamedKey::Escape,
            NamedKey::Home,
            NamedKey::End,
            NamedKey::PageUp,
            NamedKey::PageDown,
            NamedKey::Up,
            NamedKey::Down,
            NamedKey::Left,
            NamedKey::Right,
        ];
        let keys = (0u8..26)
            .map(Key::Letter)
            .chain((0u8..10).map(Key::Digit))
            .chain((1u8..=12).map(Key::Function))
            .chain(named.into_iter().map(Key::Named));
        for key in keys {
            let Some(code) = macos_keycode::key_to_keycode(key) else {
                continue;
            };
            assert_eq!(keycode_to_key(code), Some(key), "code: {code}");
        }
    }

    #[test]
    fn symbol_keycodes_record_as_the_characters_the_grammar_names() {
        let cases = [
            (macos_keycode::ANSI_GRAVE, '`'),
            (macos_keycode::ANSI_MINUS, '-'),
            (macos_keycode::ANSI_EQUAL, '+'),
            (macos_keycode::ANSI_LEFT_BRACKET, '['),
            (macos_keycode::ANSI_RIGHT_BRACKET, ']'),
            (macos_keycode::ANSI_BACKSLASH, '\\'),
            (macos_keycode::ANSI_SEMICOLON, ';'),
            (macos_keycode::ANSI_QUOTE, '\''),
            (macos_keycode::ANSI_COMMA, ','),
            (macos_keycode::ANSI_PERIOD, '.'),
            (macos_keycode::ANSI_SLASH, '/'),
        ];
        for (code, symbol) in cases {
            assert_eq!(
                keycode_to_key(code),
                Some(Key::Symbol(symbol)),
                "code: {code}"
            );
        }
    }

    #[test]
    fn symbol_recordings_round_trip_through_the_grammar() {
        let cases = [
            (macos_keycode::ANSI_EQUAL, "Super+Plus"),
            (macos_keycode::ANSI_MINUS, "Super+-"),
            (macos_keycode::ANSI_COMMA, "Super+,"),
        ];
        for (code, expected) in cases {
            let key = keycode_to_key(code).expect("symbol keycode must record");
            let formatted = grammar::format(&Hotkey {
                mods: BTreeSet::from([Modifier::Super]),
                key,
            })
            .expect("symbol recordings format");
            assert_eq!(formatted, expected, "code: {code}");
            assert_eq!(
                grammar::parse(&formatted)
                    .expect("recorded string parses")
                    .key,
                key,
                "code: {code}"
            );
        }
    }
}
