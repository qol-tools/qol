use std::sync::{LazyLock, OnceLock};

use core_graphics::event::{CGEvent, CGEventFlags, CallbackResult, EventField};
use foreign_types_shared::ForeignType;
use qol_hotkeys::macos_keycode as keycode;
use qol_runtime::event_tap_trace::{TraceSink, QUEUE_DEPTH};

use crate::platform::macos::app::remap::{self, KeyAction, Modifiers, ResolvedConfig};
use crate::platform::macos::input::{marker_for, MarkerBook};

static TRACE_KEYS: LazyLock<bool> =
    LazyLock::new(|| std::env::var_os("QOL_KEYREMAP_TRACE").is_some());

fn event_character(event: &CGEvent) -> Option<String> {
    extern "C" {
        fn CGEventKeyboardGetUnicodeString(
            event: core_graphics::sys::CGEventRef,
            max_len: core::ffi::c_ulong,
            actual_len: *mut core::ffi::c_ulong,
            buf: *mut u16,
        );
    }
    let mut buf = [0u16; 4];
    let mut len: core::ffi::c_ulong = 0;
    unsafe {
        CGEventKeyboardGetUnicodeString(
            event.as_ptr(),
            buf.len() as core::ffi::c_ulong,
            &mut len,
            buf.as_mut_ptr(),
        );
    }
    if len == 0 {
        return None;
    }
    String::from_utf16(&buf[..len as usize]).ok()
}

fn tap_trace() -> &'static TraceSink {
    static SINK: OnceLock<TraceSink> = OnceLock::new();
    SINK.get_or_init(|| {
        TraceSink::spawn("keyremap-tap-trace", QUEUE_DEPTH, |batch| {
            for line in batch {
                log::debug!("{line}");
            }
        })
    })
}

pub(crate) fn handle_key_event(
    config: &ResolvedConfig,
    event: &CGEvent,
    target_pid: i32,
    bundle_id: &str,
) -> CallbackResult {
    let flags = event.get_flags();
    let mods = extract_modifiers(flags);
    let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;
    let event_char = if config.char_swap_rules.is_empty() {
        None
    } else {
        event_character(event)
    };

    let action = remap::process_key_event(config, mods, keycode, event_char.as_deref(), bundle_id);

    if cfg!(debug_assertions)
        && *TRACE_KEYS
        && (!matches!(action, KeyAction::Passthrough) || config.excluded_apps.contains(bundle_id))
    {
        tap_trace().offer(format!(
            "[keyremap:dbg] target_pid={} app={} key=0x{:02X}({}) mods={:?} -> {:?}",
            target_pid,
            bundle_id,
            keycode,
            keycode::key_name(keycode),
            mods,
            action,
        ));
    }

    match action {
        KeyAction::Passthrough => CallbackResult::Keep,
        KeyAction::Remap {
            mods: new_mods,
            key,
        } => {
            tag_remapped_key_event(event, mods, keycode);
            let new_flags = build_flags(flags, mods, new_mods);
            event.set_flags(new_flags);
            event.set_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE, key as i64);
            CallbackResult::Keep
        }
        KeyAction::Char { ref text } => {
            tag_remapped_key_event(event, mods, keycode);
            let clean_flags = strip_all_modifiers(flags);
            event.set_flags(clean_flags);
            // Set keycode to SPACE so dead-key positions (like ´ on Nordic)
            // don't trigger the input method's dead-key state machine.
            event
                .set_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE, keycode::SPACE as i64);
            event.set_string(text);
            CallbackResult::Keep
        }
    }
}

pub(crate) fn stamp_marker(
    markers: &MarkerBook,
    event: &CGEvent,
    key_down: bool,
) -> CallbackResult {
    let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;
    if let Some(marker) = markers.lookup(keycode, key_down, std::time::Instant::now()) {
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, marker);
    }
    CallbackResult::Keep
}

fn tag_remapped_key_event(event: &CGEvent, original_mods: Modifiers, original_key: u16) {
    event.set_integer_value_field(
        EventField::EVENT_SOURCE_USER_DATA,
        marker_for(original_mods, original_key),
    );
}

fn strip_all_modifiers(flags: CGEventFlags) -> CGEventFlags {
    let mut f = flags;
    f.remove(CGEventFlags::CGEventFlagControl);
    f.remove(CGEventFlags::CGEventFlagShift);
    f.remove(CGEventFlags::CGEventFlagAlternate);
    f.remove(CGEventFlags::CGEventFlagCommand);
    f
}

/// NX_DEVICERALTKEYMASK — device-dependent bit for Right Alt/Option.
const NX_DEVICERALTKEYMASK: u64 = 0x40;

pub(crate) fn extract_modifiers(flags: CGEventFlags) -> Modifiers {
    Modifiers {
        ctrl: flags.contains(CGEventFlags::CGEventFlagControl),
        shift: flags.contains(CGEventFlags::CGEventFlagShift),
        alt: flags.contains(CGEventFlags::CGEventFlagAlternate),
        cmd: flags.contains(CGEventFlags::CGEventFlagCommand),
        ralt: (flags.bits() & NX_DEVICERALTKEYMASK) != 0,
    }
}

pub(crate) fn build_flags(original: CGEventFlags, from: Modifiers, to: Modifiers) -> CGEventFlags {
    let mut flags = original;

    if from.ctrl && !to.ctrl {
        flags.remove(CGEventFlags::CGEventFlagControl);
    }
    if from.shift && !to.shift {
        flags.remove(CGEventFlags::CGEventFlagShift);
    }
    if from.alt && !to.alt {
        flags.remove(CGEventFlags::CGEventFlagAlternate);
    }
    if from.cmd && !to.cmd {
        flags.remove(CGEventFlags::CGEventFlagCommand);
    }

    if !from.ctrl && to.ctrl {
        flags.insert(CGEventFlags::CGEventFlagControl);
    }
    if !from.shift && to.shift {
        flags.insert(CGEventFlags::CGEventFlagShift);
    }
    if !from.alt && to.alt {
        flags.insert(CGEventFlags::CGEventFlagAlternate);
    }
    if !from.cmd && to.cmd {
        flags.insert(CGEventFlags::CGEventFlagCommand);
    }

    flags
}
