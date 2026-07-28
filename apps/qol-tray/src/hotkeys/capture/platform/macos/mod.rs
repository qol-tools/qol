use super::super::binding::{Binding, CaptureEvent, Phase};
use super::super::{OnFire, RebuildBindings};
use anyhow::{bail, Result};
use core_foundation::base::TCFType;
use core_foundation::mach_port::CFMachPortRef;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CallbackResult, EventField,
};
use crossbeam_channel::Receiver;
use qol_hotkeys::grammar::Modifier as Mod;
use qol_hotkeys::macos_keycode;
use qol_runtime::keyremap_marker::{self, KeyRemapMarker};
use std::collections::{BTreeSet, HashMap};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
struct MacCombo {
    mods: BTreeSet<Mod>,
    key: u16,
}

pub(crate) fn start_recording(_session_id: u64, _events: Arc<crate::daemon::EventBus>) -> bool {
    false
}

pub(crate) fn cancel_recording(_session_id: u64) {}

pub(crate) fn install(
    bindings: Vec<Binding>,
    on_fire: OnFire,
    reload_rx: Receiver<()>,
    rebuild: RebuildBindings,
) -> Result<()> {
    let matcher = Arc::new(RwLock::new(MacBindingMatcher::new(bindings)));
    let (fire_tx, fire_rx) = mpsc::channel::<CaptureEvent>();
    std::thread::Builder::new()
        .name("hotkey-capture-macos-actions".into())
        .spawn(move || {
            while let Ok(event) = fire_rx.recv() {
                on_fire(&event);
            }
        })?;

    spawn_reload_thread(matcher.clone(), reload_rx, rebuild, fire_tx.clone());

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
    fire_tx: Sender<CaptureEvent>,
) {
    let _ = std::thread::Builder::new()
        .name("hotkey-capture-macos-reload".into())
        .spawn(move || {
            while reload_rx.recv().is_ok() {
                drain_pending(&reload_rx);
                let bindings = match rebuild() {
                    Ok(bindings) => bindings,
                    Err(error) => {
                        log::error!(
                            "macOS hotkey reload skipped; keeping current bindings: {error:#}"
                        );
                        continue;
                    }
                };
                let stopped = match matcher.write() {
                    Ok(mut guard) => {
                        let stopped = guard.reload(bindings);
                        log::info!("macOS hotkey capture: bindings reloaded");
                        stopped
                    }
                    Err(poisoned) => {
                        log::error!(
                            "macOS hotkey matcher lock poisoned during reload; recovering: {poisoned}"
                        );
                        let mut guard = poisoned.into_inner();
                        guard.reload(bindings)
                    }
                };
                for event in stopped {
                    let _ = fire_tx.send(event);
                }
            }
        });
}

fn drain_pending(reload_rx: &Receiver<()>) {
    while reload_rx.try_recv().is_ok() {}
}

fn requires_reenable(event_type: CGEventType) -> bool {
    matches!(
        event_type,
        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
    )
}

struct ReenablePort(CFMachPortRef);

unsafe impl Send for ReenablePort {}
unsafe impl Sync for ReenablePort {}

extern "C" {
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

fn run_tap(
    matcher: Arc<RwLock<MacBindingMatcher>>,
    fire_tx: Sender<CaptureEvent>,
    ready_tx: Sender<Result<(), String>>,
) {
    let reenable: Arc<OnceLock<ReenablePort>> = Arc::new(OnceLock::new());
    let cb_reenable = Arc::clone(&reenable);
    let events = vec![CGEventType::KeyDown, CGEventType::KeyUp];
    let tap = CGEventTap::new(
        CGEventTapLocation::HID,
        CGEventTapPlacement::TailAppendEventTap,
        CGEventTapOptions::Default,
        events,
        move |_proxy, event_type, event| {
            if requires_reenable(event_type) {
                if let Some(port) = cb_reenable.get() {
                    log::warn!("macOS hotkey tap disabled by OS; re-enabling");
                    unsafe { CGEventTapEnable(port.0, true) };
                }
                return CallbackResult::Keep;
            }
            if !matches!(event_type, CGEventType::KeyDown | CGEventType::KeyUp) {
                return CallbackResult::Keep;
            }
            if matches!(event_type, CGEventType::KeyDown) && is_auto_repeat(event) {
                return CallbackResult::Keep;
            }
            let observed = observed_combo(event);
            let fired = match matcher.write() {
                Ok(mut guard) => guard.match_event(event_type, &observed),
                Err(poisoned) => poisoned.into_inner().match_event(event_type, &observed),
            };
            let Some((binding, phase)) = fired else {
                return CallbackResult::Keep;
            };
            let _ = fire_tx.send(CaptureEvent { binding, phase });
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
    let _ = reenable.set(ReenablePort(tap.mach_port().as_concrete_TypeRef()));

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
    active_continuous: HashMap<u16, Binding>,
}

impl MacBindingMatcher {
    fn new(bindings: Vec<Binding>) -> Self {
        Self {
            bindings: bindings
                .into_iter()
                .filter_map(|binding| parse_mac_combo(&binding).map(|combo| (combo, binding)))
                .collect(),
            active_continuous: HashMap::new(),
        }
    }

    fn match_combo(&self, observed: &MacCombo) -> Option<&Binding> {
        self.bindings
            .iter()
            .find_map(|(combo, binding)| (combo == observed).then_some(binding))
    }

    fn reload(&mut self, bindings: Vec<Binding>) -> Vec<CaptureEvent> {
        self.bindings = bindings
            .into_iter()
            .filter_map(|binding| parse_mac_combo(&binding).map(|combo| (combo, binding)))
            .collect();
        self.active_continuous
            .drain()
            .map(|(_, binding)| CaptureEvent {
                binding,
                phase: Phase::STOP,
            })
            .collect()
    }

    fn match_event(
        &mut self,
        event_type: CGEventType,
        observed: &MacCombo,
    ) -> Option<(Binding, Phase)> {
        if matches!(event_type, CGEventType::KeyUp) {
            return self
                .active_continuous
                .remove(&observed.key)
                .map(|binding| (binding, Phase::STOP));
        }
        let binding = self.match_combo(observed)?.clone();
        if binding.continuous {
            self.active_continuous.insert(observed.key, binding.clone());
        }
        Some((binding, Phase::START))
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
    let key = macos_keycode::key_to_keycode(combo.key)?;
    Some(MacCombo {
        mods: combo.mods.clone(),
        key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::capture::parse_combo;

    fn binding(key: &str) -> Binding {
        Binding {
            combo: parse_combo(key),
            plugin_uid: crate::plugins::PluginUid::new("plugin"),
            action: "open".into(),
            raw_key: key.into(),
            continuous: false,
        }
    }

    fn binding_for(key: &str, plugin: &str, action: &str) -> Binding {
        Binding {
            combo: parse_combo(key),
            plugin_uid: crate::plugins::PluginUid::new(plugin),
            action: action.into(),
            raw_key: key.into(),
            continuous: false,
        }
    }

    fn continuous_binding_for(key: &str, plugin: &str, action: &str) -> Binding {
        Binding {
            continuous: true,
            ..binding_for(key, plugin, action)
        }
    }

    fn combo_for(key: &str) -> MacCombo {
        parse_mac_combo(&binding(key)).expect("combo")
    }

    #[test]
    fn os_disabled_tap_events_require_reenable_others_do_not() {
        let cases = [
            (CGEventType::TapDisabledByTimeout, true),
            (CGEventType::TapDisabledByUserInput, true),
            (CGEventType::KeyDown, false),
            (CGEventType::FlagsChanged, false),
            (CGEventType::Null, false),
        ];
        for (event_type, expected) in cases {
            assert_eq!(
                requires_reenable(event_type),
                expected,
                "event_type: {event_type:?}"
            );
        }
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
        assert_eq!(hit.plugin_uid.as_str(), "launcher");
        assert_eq!(hit.action, "show");
    }

    #[test]
    fn reload_keeps_current_bindings_when_rebuild_fails() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let matcher = Arc::new(RwLock::new(MacBindingMatcher::new(vec![binding_for(
            "Super+R", "first", "open",
        )])));
        let (tx, rx) = crossbeam_channel::unbounded::<()>();
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_in_rebuild = attempts.clone();
        let rebuild: RebuildBindings = Box::new(move || -> anyhow::Result<Vec<Binding>> {
            attempts_in_rebuild.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("simulated corrupt config")
        });

        let (fire_tx, _fire_rx) = mpsc::channel();
        spawn_reload_thread(matcher.clone(), rx, rebuild, fire_tx);
        tx.send(()).unwrap();

        for _ in 0..200 {
            if attempts.load(Ordering::SeqCst) > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            attempts.load(Ordering::SeqCst) > 0,
            "the reload thread must have attempted a rebuild"
        );
        std::thread::sleep(Duration::from_millis(30));

        assert!(
            matcher
                .read()
                .unwrap()
                .match_combo(&combo_for("Super+R"))
                .is_some(),
            "a failed rebuild must keep the previous bindings, not wipe them"
        );
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
    fn reload_stops_active_continuous_binding() {
        let mut matcher =
            MacBindingMatcher::new(vec![continuous_binding_for("Super+R", "first", "open")]);
        let observed = combo_for("Super+R");
        let started = matcher.match_event(CGEventType::KeyDown, &observed);

        let stopped = matcher.reload(Vec::new());

        assert_eq!(started.map(|(_, phase)| phase), Some(Phase::START));
        assert_eq!(stopped.len(), 1);
        assert_eq!(stopped[0].phase, Phase::STOP);
        assert_eq!(stopped[0].binding.plugin_uid.as_str(), "first");
        assert_eq!(stopped[0].binding.action, "open");
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
                .map(|b| (b.plugin_uid.as_str(), b.action.as_str()));
            assert_eq!(actual, *expected, "initial={:?} lookup={}", initial, lookup);
        }
    }
}
