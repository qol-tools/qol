use super::super::binding::{Binding, CaptureEvent, Phase, HEARTBEAT_INTERVAL};
use super::super::{OnFire, RebuildBindings};
use crossbeam_channel::Receiver;
use qol_hotkeys::grammar::Modifier as Mod;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, RwLock};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyCombo {
    pub(crate) mods: BTreeSet<Mod>,
    pub(crate) key: u16,
}

pub(crate) type ResolveCombo = fn(&Binding) -> Option<KeyCombo>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyTransition {
    Press,
    Repeat,
    Release,
}

#[derive(Debug)]
pub(crate) struct KeyMatcher {
    resolve: ResolveCombo,
    bindings: Vec<(KeyCombo, Binding)>,
    active_continuous: HashMap<u16, (Binding, Instant)>,
    swallowed_keys: HashSet<u16>,
}

#[derive(Debug, Default)]
pub(crate) struct MatchOutcome {
    pub(crate) swallow: bool,
    pub(crate) fired: Option<(Binding, Phase)>,
}

impl KeyMatcher {
    pub(crate) fn new(bindings: Vec<Binding>, resolve: ResolveCombo) -> Self {
        Self {
            resolve,
            bindings: resolve_all(bindings, resolve),
            active_continuous: HashMap::new(),
            swallowed_keys: HashSet::new(),
        }
    }

    pub(crate) fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) fn is_swallowing(&self, key: u16) -> bool {
        self.swallowed_keys.contains(&key)
    }

    pub(crate) fn match_combo(&self, observed: &KeyCombo) -> Option<&Binding> {
        self.bindings
            .iter()
            .find_map(|(combo, binding)| (combo == observed).then_some(binding))
    }

    pub(crate) fn reload(&mut self, bindings: Vec<Binding>) -> Vec<CaptureEvent> {
        self.bindings = resolve_all(bindings, self.resolve);
        self.active_continuous
            .drain()
            .map(|(_, (binding, _))| CaptureEvent {
                binding,
                phase: Phase::STOP,
            })
            .collect()
    }

    pub(crate) fn match_event(
        &mut self,
        transition: KeyTransition,
        observed: &KeyCombo,
    ) -> MatchOutcome {
        match transition {
            KeyTransition::Repeat => MatchOutcome {
                swallow: self.swallowed_keys.contains(&observed.key),
                fired: self.heartbeat(observed.key),
            },
            KeyTransition::Release => {
                let fired = self
                    .active_continuous
                    .remove(&observed.key)
                    .map(|(binding, _)| (binding, Phase::STOP));
                MatchOutcome {
                    swallow: self.swallowed_keys.remove(&observed.key) || fired.is_some(),
                    fired,
                }
            }
            KeyTransition::Press => self.press(observed),
        }
    }

    fn press(&mut self, observed: &KeyCombo) -> MatchOutcome {
        let Some(binding) = self.match_combo(observed).cloned() else {
            self.swallowed_keys.remove(&observed.key);
            return MatchOutcome::default();
        };
        self.swallowed_keys.insert(observed.key);
        if binding.continuous {
            self.active_continuous
                .insert(observed.key, (binding.clone(), Instant::now()));
        }
        MatchOutcome {
            swallow: true,
            fired: Some((binding, Phase::START)),
        }
    }

    fn heartbeat(&mut self, key: u16) -> Option<(Binding, Phase)> {
        let (binding, last_heartbeat) = self.active_continuous.get_mut(&key)?;
        if last_heartbeat.elapsed() < HEARTBEAT_INTERVAL {
            return None;
        }
        *last_heartbeat = Instant::now();
        Some((binding.clone(), Phase::HEARTBEAT))
    }
}

fn resolve_all(bindings: Vec<Binding>, resolve: ResolveCombo) -> Vec<(KeyCombo, Binding)> {
    bindings
        .into_iter()
        .filter_map(|binding| resolve(&binding).map(|combo| (combo, binding)))
        .collect()
}

pub(crate) fn spawn_fire_thread(on_fire: OnFire) -> std::io::Result<Sender<CaptureEvent>> {
    let (fire_tx, fire_rx) = mpsc::channel::<CaptureEvent>();
    std::thread::Builder::new()
        .name("hotkey-capture-actions".into())
        .spawn(move || {
            while let Ok(event) = fire_rx.recv() {
                on_fire(&event);
            }
        })?;
    Ok(fire_tx)
}

pub(crate) fn spawn_reload_thread(
    matcher: Arc<RwLock<KeyMatcher>>,
    reload_rx: Receiver<()>,
    rebuild: RebuildBindings,
    fire_tx: Sender<CaptureEvent>,
) {
    let _ = std::thread::Builder::new()
        .name("hotkey-capture-reload".into())
        .spawn(move || {
            while reload_rx.recv().is_ok() {
                drain_pending(&reload_rx);
                let bindings = match rebuild() {
                    Ok(bindings) => bindings,
                    Err(error) => {
                        log::error!("hotkey reload skipped; keeping current bindings: {error:#}");
                        continue;
                    }
                };
                let (stopped, poisoned) = reload_under_lock(&matcher, bindings);
                if poisoned {
                    log::error!("hotkey matcher lock poisoned during reload; recovered");
                } else {
                    log::info!("hotkey capture: bindings reloaded");
                }
                for event in stopped {
                    let _ = fire_tx.send(event);
                }
            }
        });
}

fn drain_pending(reload_rx: &Receiver<()>) {
    while reload_rx.try_recv().is_ok() {}
}

fn reload_under_lock(
    matcher: &RwLock<KeyMatcher>,
    bindings: Vec<Binding>,
) -> (Vec<CaptureEvent>, bool) {
    match matcher.write() {
        Ok(mut guard) => (guard.reload(bindings), false),
        Err(poisoned) => (poisoned.into_inner().reload(bindings), true),
    }
}

pub(crate) fn match_under_lock(
    matcher: &RwLock<KeyMatcher>,
    transition: KeyTransition,
    observed: &KeyCombo,
) -> MatchOutcome {
    match matcher.write() {
        Ok(mut guard) => guard.match_event(transition, observed),
        Err(poisoned) => poisoned.into_inner().match_event(transition, observed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::capture::parse_combo;

    fn resolve(binding: &Binding) -> Option<KeyCombo> {
        let combo = binding.combo.as_ref()?;
        Some(KeyCombo {
            mods: combo.mods.clone(),
            key: qol_hotkeys::windows_keycode::key_to_vk(combo.key)?,
        })
    }

    fn binding(key: &str) -> Binding {
        binding_for(key, "plugin", "open")
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

    fn combo_for(key: &str) -> KeyCombo {
        resolve(&binding(key)).expect("combo")
    }

    fn matcher(bindings: Vec<Binding>) -> KeyMatcher {
        KeyMatcher::new(bindings, resolve)
    }

    #[test]
    fn a_reload_releases_the_matcher_before_the_caller_logs() {
        let matcher = RwLock::new(matcher(vec![binding("ctrl+a")]));
        let (_stopped, poisoned) = reload_under_lock(&matcher, vec![binding("ctrl+b")]);
        assert!(!poisoned);
        assert!(
            matcher.try_write().is_ok(),
            "the reload must drop the matcher guard before the caller logs, or the hook callback waits behind a blocked stdout write and the OS drops the hook"
        );
    }

    #[test]
    fn rebuilt_matcher_reflects_newly_added_binding() {
        let shared = Arc::new(RwLock::new(matcher(vec![binding_for(
            "Super+R", "first", "open",
        )])));

        let added = combo_for("Shift+Super+J");
        assert!(
            shared.read().unwrap().match_combo(&added).is_none(),
            "binding must not match before reload"
        );

        *shared.write().unwrap() = matcher(vec![
            binding_for("Super+R", "first", "open"),
            binding_for("Shift+Super+J", "qol-launcher", "show"),
        ]);

        let hit = shared
            .read()
            .unwrap()
            .match_combo(&added)
            .cloned()
            .expect("newly added binding must match after swap");
        assert_eq!(hit.plugin_uid.as_str(), "qol-launcher");
        assert_eq!(hit.action, "show");
    }

    #[test]
    fn reload_keeps_current_bindings_when_rebuild_fails() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let shared = Arc::new(RwLock::new(matcher(vec![binding_for(
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
        spawn_reload_thread(shared.clone(), rx, rebuild, fire_tx);
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
            shared
                .read()
                .unwrap()
                .match_combo(&combo_for("Super+R"))
                .is_some(),
            "a failed rebuild must keep the previous bindings, not wipe them"
        );
    }

    #[test]
    fn rebuilt_matcher_drops_removed_binding() {
        let shared = Arc::new(RwLock::new(matcher(vec![
            binding_for("Super+R", "first", "open"),
            binding_for("Shift+Super+J", "qol-launcher", "show"),
        ])));

        let removed = combo_for("Shift+Super+J");
        assert!(
            shared.read().unwrap().match_combo(&removed).is_some(),
            "binding must match before reload"
        );

        *shared.write().unwrap() = matcher(vec![binding_for("Super+R", "first", "open")]);

        assert!(
            shared.read().unwrap().match_combo(&removed).is_none(),
            "removed binding must no longer match after swap"
        );
    }

    #[test]
    fn rebuilt_matcher_honors_disabled_filter() {
        let shared = Arc::new(RwLock::new(matcher(vec![binding_for(
            "Super+R", "first", "open",
        )])));

        let disabled = combo_for("Super+R");
        assert!(shared.read().unwrap().match_combo(&disabled).is_some());

        *shared.write().unwrap() = matcher(vec![]);

        assert!(
            shared.read().unwrap().match_combo(&disabled).is_none(),
            "disabled (filtered-out) binding must no longer match after swap"
        );
    }

    #[test]
    fn reload_stops_active_continuous_binding() {
        let mut matcher = matcher(vec![continuous_binding_for("Super+R", "first", "open")]);
        let observed = combo_for("Super+R");
        let started = matcher.match_event(KeyTransition::Press, &observed);

        let stopped = matcher.reload(Vec::new());

        assert_eq!(started.fired.map(|(_, phase)| phase), Some(Phase::START));
        assert_eq!(stopped.len(), 1);
        assert_eq!(stopped[0].phase, Phase::STOP);
        assert_eq!(stopped[0].binding.plugin_uid.as_str(), "first");
        assert_eq!(stopped[0].binding.action, "open");
    }

    #[test]
    fn a_held_hotkey_fires_once_and_keeps_its_repeats_and_release_from_the_app() {
        let mut matcher = matcher(vec![binding("Super+R")]);
        let observed = combo_for("Super+R");

        let press = matcher.match_event(KeyTransition::Press, &observed);
        assert!(matcher.is_swallowing(observed.key));
        let repeat = matcher.match_event(KeyTransition::Repeat, &observed);
        let release = matcher.match_event(KeyTransition::Release, &observed);

        assert!(press.swallow && press.fired.is_some());
        assert!(repeat.swallow && repeat.fired.is_none());
        assert!(release.swallow && release.fired.is_none());
        assert!(!matcher.is_swallowing(observed.key));
    }

    #[test]
    fn a_held_continuous_hotkey_keeps_the_action_going_until_release() {
        let mut matcher = matcher(vec![continuous_binding_for("Super+R", "first", "up")]);
        let observed = combo_for("Super+R");

        let press = matcher.match_event(KeyTransition::Press, &observed);
        let early = matcher.match_event(KeyTransition::Repeat, &observed);
        if let Some((_, last_heartbeat)) = matcher.active_continuous.get_mut(&observed.key) {
            *last_heartbeat = Instant::now() - HEARTBEAT_INTERVAL;
        }
        let repeat = matcher.match_event(KeyTransition::Repeat, &observed);
        let release = matcher.match_event(KeyTransition::Release, &observed);

        let phase = |outcome: MatchOutcome| outcome.fired.map(|(_, phase)| phase);
        assert_eq!(phase(press), Some(Phase::START));
        assert_eq!(phase(early), None);
        assert_eq!(phase(repeat), Some(Phase::HEARTBEAT));
        assert_eq!(phase(release), Some(Phase::STOP));
    }

    #[test]
    fn repeats_of_an_unbound_key_reach_the_app() {
        let mut matcher = matcher(vec![binding("Super+R")]);
        let observed = combo_for("Super+T");

        matcher.match_event(KeyTransition::Press, &observed);
        let repeat = matcher.match_event(KeyTransition::Repeat, &observed);
        let release = matcher.match_event(KeyTransition::Release, &observed);

        assert!(!repeat.swallow && !release.swallow);
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
            let m = matcher(bindings);
            let observed = combo_for(lookup);
            let actual = m
                .match_combo(&observed)
                .map(|b| (b.plugin_uid.as_str(), b.action.as_str()));
            assert_eq!(actual, *expected, "initial={:?} lookup={}", initial, lookup);
        }
    }
}
