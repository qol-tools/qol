use crate::daemon::{DaemonEvent, EventBus};
use crossbeam_channel::{bounded, Receiver, Sender};
use qol_hotkeys::grammar::{self, Hotkey, Key, Modifier, NamedKey};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, GrabMode, GrabStatus};
use x11rb::protocol::Event;

const READY_TIMEOUT: Duration = Duration::from_secs(2);
const STOP_TIMEOUT: Duration = Duration::from_millis(500);
const POLL_INTERVAL: Duration = Duration::from_millis(8);

const XK_BACKSPACE: u32 = 0xff08;
const XK_TAB: u32 = 0xff09;
const XK_RETURN: u32 = 0xff0d;
const XK_PAUSE: u32 = 0xff13;
const XK_ESCAPE: u32 = 0xff1b;
const XK_HOME: u32 = 0xff50;
const XK_LEFT: u32 = 0xff51;
const XK_UP: u32 = 0xff52;
const XK_RIGHT: u32 = 0xff53;
const XK_DOWN: u32 = 0xff54;
const XK_PAGE_UP: u32 = 0xff55;
const XK_PAGE_DOWN: u32 = 0xff56;
const XK_END: u32 = 0xff57;
const XK_PRINT: u32 = 0xff61;
const XK_INSERT: u32 = 0xff63;
const XK_DELETE: u32 = 0xffff;
const XK_F1: u32 = 0xffbe;
const XK_F12: u32 = 0xffc9;
const XK_SHIFT_L: u32 = 0xffe1;
const XK_SHIFT_R: u32 = 0xffe2;
const XK_CONTROL_L: u32 = 0xffe3;
const XK_CONTROL_R: u32 = 0xffe4;
const XK_META_L: u32 = 0xffe7;
const XK_META_R: u32 = 0xffe8;
const XK_ALT_L: u32 = 0xffe9;
const XK_ALT_R: u32 = 0xffea;
const XK_SUPER_L: u32 = 0xffeb;
const XK_SUPER_R: u32 = 0xffec;

#[derive(Default)]
struct RecorderState {
    active: Option<ActiveRecording>,
}

struct ActiveRecording {
    session_id: u64,
    cancel: Sender<()>,
    done: Receiver<()>,
}

#[derive(Default)]
pub(super) struct RecorderHub {
    state: Mutex<RecorderState>,
}

impl RecorderHub {
    pub(super) fn start(&self, session_id: u64, events: Arc<EventBus>) -> bool {
        let mut state = self.lock();
        stop_active(state.active.take());
        let (cancel_tx, cancel_rx) = bounded(1);
        let (ready_tx, ready_rx) = bounded(1);
        let (done_tx, done_rx) = bounded(1);
        let spawned = std::thread::Builder::new()
            .name("hotkey-recorder-x11".into())
            .spawn(move || run_x11_recording(session_id, events, cancel_rx, ready_tx, done_tx));
        if spawned.is_err() || ready_rx.recv_timeout(READY_TIMEOUT) != Ok(true) {
            return false;
        }
        state.active = Some(ActiveRecording {
            session_id,
            cancel: cancel_tx,
            done: done_rx,
        });
        qol_runtime::probe!("HOTKEY_RECORD_ARM", "session={session_id} backend=x11");
        true
    }

    pub(super) fn cancel(&self, session_id: u64) {
        let mut state = self.lock();
        if state
            .active
            .as_ref()
            .is_none_or(|active| active.session_id != session_id)
        {
            return;
        }
        stop_active(state.active.take());
        qol_runtime::probe!("HOTKEY_RECORD_CANCEL", "session={session_id}");
    }

    fn lock(&self) -> MutexGuard<'_, RecorderState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn stop_active(active: Option<ActiveRecording>) {
    let Some(active) = active else { return };
    let _ = active.cancel.try_send(());
    let _ = active.done.recv_timeout(STOP_TIMEOUT);
}

fn run_x11_recording(
    session_id: u64,
    events: Arc<EventBus>,
    cancel: Receiver<()>,
    ready: Sender<bool>,
    done: Sender<()>,
) {
    let result = run_x11_recording_inner(session_id, events, cancel, &ready);
    if let Err(error) = result {
        log::warn!("X11 hotkey recording failed: {error:#}");
        let _ = ready.try_send(false);
    }
    let _ = done.send(());
}

fn run_x11_recording_inner(
    session_id: u64,
    events: Arc<EventBus>,
    cancel: Receiver<()>,
    ready: &Sender<bool>,
) -> anyhow::Result<()> {
    let (connection, screen_number) = x11rb::connect(None)?;
    let root = connection.setup().roots[screen_number].root;
    let keymap = KeyboardMap::load(&connection)?;
    let reply = connection
        .grab_keyboard(
            false,
            root,
            x11rb::CURRENT_TIME,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )?
        .reply()?;
    if reply.status != GrabStatus::SUCCESS {
        anyhow::bail!(
            "X11 keyboard grab returned status {}",
            u8::from(reply.status)
        );
    }
    connection.flush()?;
    let _guard = KeyboardGrabGuard(&connection);
    let _ = ready.send(true);
    let mut modifiers = BTreeSet::new();

    loop {
        if cancel.try_recv().is_ok() {
            break;
        }
        let Some(event) = connection.poll_for_event()? else {
            std::thread::sleep(POLL_INTERVAL);
            continue;
        };
        match event {
            Event::KeyPress(event) => {
                let Some(keysym) = keymap.keysym(event.detail) else {
                    continue;
                };
                if let Some(modifier) = keysym_to_modifier(keysym) {
                    modifiers.insert(modifier);
                    continue;
                }
                if keysym == XK_ESCAPE {
                    events.send(DaemonEvent::HotkeyRecordingCanceled { session_id });
                    qol_runtime::probe!(
                        "HOTKEY_RECORD_CANCEL",
                        "session={session_id} source=escape"
                    );
                    break;
                }
                let Some(key) = keysym_to_key(keysym) else {
                    continue;
                };
                let Some(formatted) = grammar::format(&Hotkey {
                    mods: modifiers.clone(),
                    key,
                }) else {
                    continue;
                };
                qol_runtime::probe!(
                    "HOTKEY_RECORD_COMPLETE",
                    "session={session_id} key={formatted} backend=x11"
                );
                events.send(DaemonEvent::HotkeyRecorded {
                    session_id,
                    key: formatted,
                });
                break;
            }
            Event::KeyRelease(event) => {
                if let Some(modifier) = keymap.keysym(event.detail).and_then(keysym_to_modifier) {
                    modifiers.remove(&modifier);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

struct KeyboardGrabGuard<'a, C: Connection>(&'a C);

impl<C: Connection> Drop for KeyboardGrabGuard<'_, C> {
    fn drop(&mut self) {
        let _ = self.0.ungrab_keyboard(x11rb::CURRENT_TIME);
        let _ = self.0.flush();
    }
}

struct KeyboardMap {
    min_keycode: u8,
    keysyms_per_keycode: usize,
    keysyms: Vec<u32>,
}

impl KeyboardMap {
    fn load<C: Connection>(connection: &C) -> anyhow::Result<Self> {
        let setup = connection.setup();
        let min_keycode = setup.min_keycode;
        let count = setup.max_keycode - min_keycode + 1;
        let reply = connection
            .get_keyboard_mapping(min_keycode, count)?
            .reply()?;
        Ok(Self {
            min_keycode,
            keysyms_per_keycode: usize::from(reply.keysyms_per_keycode),
            keysyms: reply.keysyms,
        })
    }

    fn keysym(&self, keycode: u8) -> Option<u32> {
        let index = usize::from(keycode.checked_sub(self.min_keycode)?)
            .checked_mul(self.keysyms_per_keycode)?;
        self.keysyms
            .get(index..index + self.keysyms_per_keycode)?
            .iter()
            .copied()
            .find(|keysym| *keysym != 0)
    }
}

fn keysym_to_modifier(keysym: u32) -> Option<Modifier> {
    match keysym {
        XK_CONTROL_L | XK_CONTROL_R => Some(Modifier::Ctrl),
        XK_ALT_L | XK_ALT_R | XK_META_L | XK_META_R => Some(Modifier::Alt),
        XK_SHIFT_L | XK_SHIFT_R => Some(Modifier::Shift),
        XK_SUPER_L | XK_SUPER_R => Some(Modifier::Super),
        _ => None,
    }
}

fn keysym_to_key(keysym: u32) -> Option<Key> {
    if (u32::from(b'a')..=u32::from(b'z')).contains(&keysym) {
        return u8::try_from(keysym - u32::from(b'a')).ok().map(Key::Letter);
    }
    if (u32::from(b'A')..=u32::from(b'Z')).contains(&keysym) {
        return u8::try_from(keysym - u32::from(b'A')).ok().map(Key::Letter);
    }
    if (u32::from(b'0')..=u32::from(b'9')).contains(&keysym) {
        return u8::try_from(keysym - u32::from(b'0')).ok().map(Key::Digit);
    }
    if (XK_F1..=XK_F12).contains(&keysym) {
        return u8::try_from(keysym - XK_F1 + 1).ok().map(Key::Function);
    }
    Some(Key::Named(match keysym {
        0x20 => NamedKey::Space,
        XK_RETURN => NamedKey::Enter,
        XK_ESCAPE => NamedKey::Escape,
        XK_TAB => NamedKey::Tab,
        XK_BACKSPACE => NamedKey::Backspace,
        XK_DELETE => NamedKey::Delete,
        XK_INSERT => NamedKey::Insert,
        XK_HOME => NamedKey::Home,
        XK_END => NamedKey::End,
        XK_PAGE_UP => NamedKey::PageUp,
        XK_PAGE_DOWN => NamedKey::PageDown,
        XK_UP => NamedKey::Up,
        XK_DOWN => NamedKey::Down,
        XK_LEFT => NamedKey::Left,
        XK_RIGHT => NamedKey::Right,
        XK_PRINT => NamedKey::PrintScreen,
        XK_PAUSE => NamedKey::Pause,
        _ => return char::from_u32(keysym).and_then(grammar::symbol_key),
    }))
}

pub(super) fn global() -> Arc<RecorderHub> {
    static RECORDER: OnceLock<Arc<RecorderHub>> = OnceLock::new();
    RECORDER
        .get_or_init(|| Arc::new(RecorderHub::default()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_x11_keysyms_to_the_shared_hotkey_grammar() {
        let modifiers = BTreeSet::from([Modifier::Ctrl, Modifier::Alt, Modifier::Shift]);
        let key = keysym_to_key(XK_LEFT).unwrap();
        assert_eq!(
            grammar::format(&Hotkey {
                mods: modifiers,
                key
            })
            .as_deref(),
            Some("Ctrl+Alt+Shift+Left")
        );
        assert_eq!(keysym_to_key(u32::from(b'q')), Some(Key::Letter(16)));
        assert_eq!(keysym_to_key(u32::from(b'5')), Some(Key::Digit(5)));
        assert_eq!(keysym_to_key(XK_F12), Some(Key::Function(12)));
    }

    #[test]
    fn maps_symbol_keysyms_to_the_key_they_type() {
        assert_eq!(keysym_to_key(0x2b), Some(Key::Symbol('+')));
        assert_eq!(keysym_to_key(0x2d), Some(Key::Symbol('-')));
        assert_eq!(keysym_to_key(0xe5), Some(Key::Symbol('\u{e5}')));
        assert_eq!(
            grammar::format(&Hotkey {
                mods: BTreeSet::from([Modifier::Super]),
                key: keysym_to_key(0x2b).unwrap(),
            })
            .as_deref(),
            Some("Super+Plus")
        );
        assert_eq!(keysym_to_key(0xfe51), None);
    }

    #[test]
    fn maps_both_sides_of_every_x11_modifier() {
        let cases = [
            (XK_CONTROL_L, Modifier::Ctrl),
            (XK_CONTROL_R, Modifier::Ctrl),
            (XK_ALT_L, Modifier::Alt),
            (XK_ALT_R, Modifier::Alt),
            (XK_SHIFT_L, Modifier::Shift),
            (XK_SHIFT_R, Modifier::Shift),
            (XK_SUPER_L, Modifier::Super),
            (XK_SUPER_R, Modifier::Super),
        ];
        for (keysym, expected) in cases {
            assert_eq!(keysym_to_modifier(keysym), Some(expected));
        }
    }
}
