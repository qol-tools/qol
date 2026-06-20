use super::super::binding::{Binding, Mod};
use super::super::{OnFire, RebuildBindings};
use anyhow::{bail, Result};
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CallbackResult, EventField,
};
use crossbeam_channel::Receiver;
use qol_runtime::keyremap_marker::{self, KeyRemapMarker};
use std::collections::BTreeSet;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, RwLock};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
struct MacCombo {
    mods: BTreeSet<Mod>,
    key: u16,
}

pub(crate) fn install(
    bindings: Vec<Binding>,
    on_fire: OnFire,
    reload_rx: Receiver<()>,
    rebuild: RebuildBindings,
) -> Result<()> {
    let matcher = Arc::new(RwLock::new(MacBindingMatcher::new(bindings)));
    let (fire_tx, fire_rx) = mpsc::channel::<Binding>();
    std::thread::Builder::new()
        .name("hotkey-capture-macos-actions".into())
        .spawn(move || {
            while let Ok(binding) = fire_rx.recv() {
                on_fire(&binding);
            }
        })?;

    spawn_reload_thread(matcher.clone(), reload_rx, rebuild);

    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    let tap_matcher = matcher.clone();
    std::thread::Builder::new()
        .name("hotkey-capture-macos".into())
        .spawn(move || run_tap(tap_matcher, fire_tx, ready_tx))?;

    match ready_rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => bail!(message),
        Err(RecvTimeoutError::Timeout) => {
            bail!("macOS hotkey event tap did not report readiness")
        }
        Err(RecvTimeoutError::Disconnected) => {
            bail!("macOS hotkey event tap exited before reporting readiness")
        }
    }
}

fn spawn_reload_thread(
    matcher: Arc<RwLock<MacBindingMatcher>>,
    reload_rx: Receiver<()>,
    rebuild: RebuildBindings,
) {
    let _ = std::thread::Builder::new()
        .name("hotkey-capture-macos-reload".into())
        .spawn(move || {
            while reload_rx.recv().is_ok() {
                drain_pending(&reload_rx);
                let next = MacBindingMatcher::new(rebuild());
                match matcher.write() {
                    Ok(mut guard) => {
                        *guard = next;
                        log::info!("macOS hotkey capture: bindings reloaded");
                    }
                    Err(poisoned) => {
                        log::error!(
                            "macOS hotkey matcher lock poisoned during reload; recovering: {poisoned}"
                        );
                        let mut guard = poisoned.into_inner();
                        *guard = next;
                    }
                }
            }
        });
}

fn drain_pending(reload_rx: &Receiver<()>) {
    while reload_rx.try_recv().is_ok() {}
}

fn run_tap(
    matcher: Arc<RwLock<MacBindingMatcher>>,
    fire_tx: Sender<Binding>,
    ready_tx: Sender<Result<(), String>>,
) {
    let events = vec![CGEventType::KeyDown];
    let tap = CGEventTap::new(
        CGEventTapLocation::HID,
        CGEventTapPlacement::TailAppendEventTap,
        CGEventTapOptions::Default,
        events,
        move |_proxy, event_type, event| {
            if !matches!(event_type, CGEventType::KeyDown) {
                return CallbackResult::Keep;
            }
            if is_auto_repeat(event) {
                return CallbackResult::Keep;
            }
            let observed = observed_combo(event);
            let fired = match matcher.read() {
                Ok(guard) => guard.match_combo(&observed).cloned(),
                Err(poisoned) => poisoned.into_inner().match_combo(&observed).cloned(),
            };
            let Some(binding) = fired else {
                return CallbackResult::Keep;
            };
            let _ = fire_tx.send(binding);
            CallbackResult::Drop
        },
    );

    let tap = match tap {
        Ok(tap) => tap,
        Err(()) => {
            let _ = ready_tx.send(Err(
                "failed to create macOS hotkey event tap; grant qol-tray Accessibility permission"
                    .into(),
            ));
            return;
        }
    };

    let loop_source = match tap.mach_port().create_runloop_source(0) {
        Ok(loop_source) => loop_source,
        Err(()) => {
            let _ = ready_tx.send(Err(
                "failed to create run loop source for macOS hotkey event tap".into(),
            ));
            return;
        }
    };
    CFRunLoop::get_current().add_source(&loop_source, unsafe { kCFRunLoopCommonModes });
    tap.enable();
    let _ = ready_tx.send(Ok(()));
    CFRunLoop::run_current();
}

#[derive(Debug)]
struct MacBindingMatcher {
    bindings: Vec<(MacCombo, Binding)>,
}

impl MacBindingMatcher {
    fn new(bindings: Vec<Binding>) -> Self {
        Self {
            bindings: bindings
                .into_iter()
                .filter_map(|binding| parse_mac_combo(&binding).map(|combo| (combo, binding)))
                .collect(),
        }
    }

    fn match_combo(&self, observed: &MacCombo) -> Option<&Binding> {
        self.bindings
            .iter()
            .find_map(|(combo, binding)| (combo == observed).then_some(binding))
    }
}

fn observed_combo(event: &CGEvent) -> MacCombo {
    let marker = remap_marker(event);
    MacCombo {
        mods: marker
            .map(|marker| marker_mods(marker.mods))
            .unwrap_or_else(|| event_mods(event.get_flags())),
        key: marker
            .map(|marker| marker.key)
            .unwrap_or_else(|| event_key(event)),
    }
}

fn remap_marker(event: &CGEvent) -> Option<KeyRemapMarker> {
    keyremap_marker::decode(event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA))
}

fn event_key(event: &CGEvent) -> u16 {
    event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16
}

fn is_auto_repeat(event: &CGEvent) -> bool {
    event.get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT) != 0
}

fn event_mods(flags: CGEventFlags) -> BTreeSet<Mod> {
    let mut mods = BTreeSet::new();
    if flags.contains(CGEventFlags::CGEventFlagShift) {
        mods.insert(Mod::Shift);
    }
    if flags.contains(CGEventFlags::CGEventFlagControl) {
        mods.insert(Mod::Ctrl);
    }
    if flags.contains(CGEventFlags::CGEventFlagAlternate) {
        mods.insert(Mod::Alt);
    }
    if flags.contains(CGEventFlags::CGEventFlagCommand) {
        mods.insert(Mod::Super);
    }
    mods
}

fn marker_mods(bits: u8) -> BTreeSet<Mod> {
    let mut mods = BTreeSet::new();
    if bits & keyremap_marker::MOD_SHIFT != 0 {
        mods.insert(Mod::Shift);
    }
    if bits & keyremap_marker::MOD_CTRL != 0 {
        mods.insert(Mod::Ctrl);
    }
    if bits & keyremap_marker::MOD_ALT != 0 {
        mods.insert(Mod::Alt);
    }
    if bits & keyremap_marker::MOD_SUPER != 0 {
        mods.insert(Mod::Super);
    }
    mods
}

fn parse_mac_combo(binding: &Binding) -> Option<MacCombo> {
    let combo = binding.combo.as_ref()?;
    let key = parse_combo_key(&binding.raw_key)?;
    Some(MacCombo {
        mods: combo.mods.clone(),
        key,
    })
}

fn parse_combo_key(raw_key: &str) -> Option<u16> {
    let mut key = None;
    for raw in raw_key.split('+') {
        let token = raw.trim().to_ascii_lowercase();
        if is_modifier(&token) {
            continue;
        }
        if key.is_some() {
            return None;
        }
        key = mac_key_name_to_code(&token);
    }
    key
}

fn is_modifier(token: &str) -> bool {
    matches!(
        token,
        "shift"
            | "ctrl"
            | "control"
            | "alt"
            | "option"
            | "opt"
            | "super"
            | "meta"
            | "win"
            | "cmd"
            | "command"
    )
}

fn mac_key_name_to_code(name: &str) -> Option<u16> {
    if let Some(code) = mac_letter_code(name) {
        return Some(code);
    }
    if let Some(code) = mac_digit_code(name) {
        return Some(code);
    }
    if let Some(code) = mac_function_key_code(name) {
        return Some(code);
    }
    Some(match name {
        "space" => 49,
        "enter" | "return" => 36,
        "escape" | "esc" => 53,
        "tab" => 48,
        "backspace" => 51,
        "delete" | "del" => 117,
        "home" => 115,
        "end" => 119,
        "pageup" | "pgup" => 116,
        "pagedown" | "pgdn" => 121,
        "up" => 126,
        "down" => 125,
        "left" => 123,
        "right" => 124,
        _ => return None,
    })
}

fn mac_letter_code(name: &str) -> Option<u16> {
    if name.len() != 1 {
        return None;
    }
    let c = name.chars().next()?;
    if !c.is_ascii_lowercase() {
        return None;
    }
    let index = (c as u8 - b'a') as usize;
    const LETTERS: [u16; 26] = [
        0, 11, 8, 2, 14, 3, 5, 4, 34, 38, 40, 37, 46, 45, 31, 35, 12, 15, 1, 17, 32, 9, 13, 7, 16,
        6,
    ];
    Some(LETTERS[index])
}

fn mac_digit_code(name: &str) -> Option<u16> {
    if name.len() != 1 {
        return None;
    }
    let c = name.chars().next()?;
    if !c.is_ascii_digit() {
        return None;
    }
    const DIGITS: [u16; 10] = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25];
    Some(DIGITS[(c as u8 - b'0') as usize])
}

fn mac_function_key_code(name: &str) -> Option<u16> {
    let rest = name.strip_prefix('f')?;
    let num: usize = rest.parse().ok()?;
    const FUNCTION_KEYS: [u16; 12] = [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111];
    FUNCTION_KEYS.get(num.checked_sub(1)?).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::capture::parse_combo;

    fn binding(key: &str) -> Binding {
        Binding {
            combo: parse_combo(key),
            plugin_id: "plugin".into(),
            action: "open".into(),
            raw_key: key.into(),
        }
    }

    fn binding_for(key: &str, plugin: &str, action: &str) -> Binding {
        Binding {
            combo: parse_combo(key),
            plugin_id: plugin.into(),
            action: action.into(),
            raw_key: key.into(),
        }
    }

    fn combo_for(key: &str) -> MacCombo {
        parse_mac_combo(&binding(key)).expect("combo")
    }

    #[test]
    fn parses_shift_super_r_as_macos_keycode() {
        let combo = parse_mac_combo(&binding("Shift+Super+R")).expect("combo");
        assert_eq!(combo.key, 15);
        assert_eq!(combo.mods, BTreeSet::from([Mod::Shift, Mod::Super]));
    }

    #[test]
    fn marker_mods_round_trip_to_combo_mods() {
        let bits = keyremap_marker::MOD_CTRL | keyremap_marker::MOD_SUPER;
        assert_eq!(marker_mods(bits), BTreeSet::from([Mod::Ctrl, Mod::Super]));
    }

    #[test]
    fn original_ctrl_does_not_match_synthetic_super_binding() {
        let observed = MacCombo {
            mods: BTreeSet::from([Mod::Ctrl]),
            key: 15,
        };

        let super_matcher = MacBindingMatcher::new(vec![binding("Super+R")]);
        assert!(super_matcher.match_combo(&observed).is_none());

        let ctrl_matcher = MacBindingMatcher::new(vec![binding("Ctrl+R")]);
        assert!(ctrl_matcher.match_combo(&observed).is_some());
    }

    #[test]
    fn rejects_unknown_key() {
        assert!(parse_mac_combo(&binding("Super+Nope")).is_none());
    }

    #[test]
    fn rebuilt_matcher_reflects_newly_added_binding() {
        let matcher = Arc::new(RwLock::new(MacBindingMatcher::new(vec![binding_for(
            "Super+R", "first", "open",
        )])));

        let added = combo_for("Shift+Super+J");
        assert!(
            matcher.read().unwrap().match_combo(&added).is_none(),
            "binding must not match before reload"
        );

        let next = MacBindingMatcher::new(vec![
            binding_for("Super+R", "first", "open"),
            binding_for("Shift+Super+J", "launcher", "show"),
        ]);
        *matcher.write().unwrap() = next;

        let hit = matcher
            .read()
            .unwrap()
            .match_combo(&added)
            .cloned()
            .expect("newly added binding must match after swap");
        assert_eq!(hit.plugin_id, "launcher");
        assert_eq!(hit.action, "show");
    }

    #[test]
    fn rebuilt_matcher_drops_removed_binding() {
        let matcher = Arc::new(RwLock::new(MacBindingMatcher::new(vec![
            binding_for("Super+R", "first", "open"),
            binding_for("Shift+Super+J", "launcher", "show"),
        ])));

        let removed = combo_for("Shift+Super+J");
        assert!(
            matcher.read().unwrap().match_combo(&removed).is_some(),
            "binding must match before reload"
        );

        let next = MacBindingMatcher::new(vec![binding_for("Super+R", "first", "open")]);
        *matcher.write().unwrap() = next;

        assert!(
            matcher.read().unwrap().match_combo(&removed).is_none(),
            "removed binding must no longer match after swap"
        );
    }

    #[test]
    fn rebuilt_matcher_honors_disabled_filter() {
        let matcher = Arc::new(RwLock::new(MacBindingMatcher::new(vec![binding_for(
            "Super+R", "first", "open",
        )])));

        let disabled = combo_for("Super+R");
        assert!(matcher.read().unwrap().match_combo(&disabled).is_some());

        let next = MacBindingMatcher::new(vec![]);
        *matcher.write().unwrap() = next;

        assert!(
            matcher.read().unwrap().match_combo(&disabled).is_none(),
            "disabled (filtered-out) binding must no longer match after swap"
        );
    }

    #[test]
    fn cases_for_combo_rebuild_contract() {
        type BindingTuple = (&'static str, &'static str, &'static str);
        type ExpectedHit = Option<(&'static str, &'static str)>;
        type Case = (&'static [BindingTuple], &'static str, ExpectedHit);

        let cases: &[Case] = &[
            (&[("Super+R", "p", "a")], "Super+R", Some(("p", "a"))),
            (
                &[("Super+R", "p", "a"), ("Shift+Super+J", "q", "b")],
                "Shift+Super+J",
                Some(("q", "b")),
            ),
            (&[("Super+R", "p", "a")], "Shift+Super+J", None),
            (&[], "Super+R", None),
        ];

        for (initial, lookup, expected) in cases {
            let bindings: Vec<Binding> = initial
                .iter()
                .map(|(k, p, a)| binding_for(k, p, a))
                .collect();
            let m = MacBindingMatcher::new(bindings);
            let observed = combo_for(lookup);
            let actual = m
                .match_combo(&observed)
                .map(|b| (b.plugin_id.as_str(), b.action.as_str()));
            assert_eq!(actual, *expected, "initial={:?} lookup={}", initial, lookup);
        }
    }
}
