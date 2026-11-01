use super::capture::{CaptureEvent, Combo, Phase};
use super::hold::{Fire, HoldPlan, HoldTimer};
use crossbeam_channel::{after, never, select, unbounded, Receiver, Sender};
use std::collections::HashMap;
use std::time::Instant;

#[derive(Default)]
pub(crate) struct HoldMachine {
    timers: HashMap<Combo, HoldTimer>,
}

impl HoldMachine {
    pub(crate) fn press(&mut self, chord: Combo, plan: HoldPlan, now: Instant) {
        self.timers.insert(chord, HoldTimer::start(plan, now));
    }

    pub(crate) fn release(&mut self, chord: &Combo, now: Instant) -> Option<Fire> {
        self.timers.remove(chord)?.release(now)
    }

    pub(crate) fn cancel(&mut self, chord: &Combo) {
        self.timers.remove(chord);
    }

    pub(crate) fn expire(&mut self, now: Instant) -> Vec<Fire> {
        self.timers
            .values_mut()
            .filter_map(|timer| timer.expire(now))
            .collect()
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.timers.values().filter_map(HoldTimer::deadline).min()
    }

    pub(crate) fn apply(&mut self, event: &CaptureEvent, now: Instant) -> Option<Fire> {
        let chord = event.binding.combo.clone()?;
        let plan = event.binding.hold.clone()?;
        if event.phase == Phase::START {
            self.press(chord, plan, now);
            None
        } else if event.phase == Phase::STOP {
            self.release(&chord, now)
        } else {
            if event.phase == Phase::CANCEL {
                self.cancel(&chord);
            }
            None
        }
    }
}

pub(crate) struct HoldResolver {
    events: Sender<CaptureEvent>,
}

impl HoldResolver {
    pub(crate) fn spawn(fire: impl Fn(Fire) + Send + 'static) -> std::io::Result<Self> {
        let (events, inbox) = unbounded::<CaptureEvent>();
        std::thread::Builder::new()
            .name("hotkey-hold-resolver".into())
            .spawn(move || run(&inbox, &fire))?;
        Ok(Self { events })
    }

    pub(crate) fn feed(&self, event: &CaptureEvent) {
        let _ = self.events.send(event.clone());
    }
}

fn run(inbox: &Receiver<CaptureEvent>, fire: &dyn Fn(Fire)) {
    let mut machine = HoldMachine::default();
    loop {
        let deadline = match machine.next_deadline() {
            Some(deadline) => after(deadline.saturating_duration_since(Instant::now())),
            None => never(),
        };
        select! {
            recv(inbox) -> event => match event {
                Ok(event) => {
                    if let Some(decision) = machine.apply(&event, Instant::now()) {
                        fire(decision);
                    }
                }
                Err(_) => return,
            },
            recv(deadline) -> _ => {}
        }
        for decision in machine.expire(Instant::now()) {
            fire(decision);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::capture::{parse_combo, Binding};
    use crate::hotkeys::hold::Target;
    use crate::plugins::PluginUid;
    use std::sync::mpsc;
    use std::time::Duration;

    fn target(action: &str) -> Target {
        Target {
            plugin_uid: PluginUid::new("uid-shot"),
            action: action.into(),
        }
    }

    fn event(key: &str, hold_ms: u32, tap: Option<&str>, phase: Phase) -> CaptureEvent {
        let plan = HoldPlan {
            hold_ms,
            hold: target(&format!("hold-{key}")),
            tap: tap.map(|name| target(&format!("{name}-{key}"))),
        };
        CaptureEvent {
            binding: Binding {
                combo: parse_combo(key),
                plugin_uid: PluginUid::new("uid-shot"),
                action: plan.primary().action.clone(),
                raw_key: key.into(),
                continuous: false,
                hold: Some(plan),
            },
            phase,
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn tap(key: &str) -> Fire {
        Fire::Tap(target(&format!("tap-{key}")))
    }

    fn hold(key: &str) -> Fire {
        Fire::Hold(target(&format!("hold-{key}")))
    }

    const S: &str = "Shift+Super+S";
    const R: &str = "Shift+Super+R";

    #[test]
    fn a_release_before_the_deadline_fires_the_tap() {
        let mut machine = HoldMachine::default();
        let t0 = Instant::now();
        assert_eq!(
            machine.apply(&event(S, 1000, Some("tap"), Phase::START), t0),
            None
        );
        assert_eq!(machine.expire(t0 + ms(999)), Vec::new());
        assert_eq!(
            machine.apply(&event(S, 1000, Some("tap"), Phase::STOP), t0 + ms(140)),
            Some(tap(S))
        );
        assert_eq!(machine.next_deadline(), None);
    }

    #[test]
    fn a_hold_only_chord_released_early_fires_nothing() {
        let mut machine = HoldMachine::default();
        let t0 = Instant::now();
        machine.apply(&event(S, 1000, None, Phase::START), t0);
        assert_eq!(
            machine.apply(&event(S, 1000, None, Phase::STOP), t0 + ms(200)),
            None
        );
        assert_eq!(machine.next_deadline(), None);
    }

    #[test]
    fn a_hold_only_chord_fires_at_the_deadline_with_the_keys_still_down() {
        let mut machine = HoldMachine::default();
        let t0 = Instant::now();
        machine.apply(&event(S, 800, None, Phase::START), t0);
        assert_eq!(machine.next_deadline(), Some(t0 + ms(800)));
        assert_eq!(machine.expire(t0 + ms(799)), Vec::new());
        assert_eq!(machine.expire(t0 + ms(800)), vec![hold(S)]);
        assert_eq!(machine.next_deadline(), None);
    }

    #[test]
    fn a_tap_and_hold_chord_held_past_the_deadline_fires_only_the_hold() {
        let mut machine = HoldMachine::default();
        let t0 = Instant::now();
        machine.apply(&event(S, 1000, Some("tap"), Phase::START), t0);
        assert_eq!(machine.expire(t0 + ms(1000)), vec![hold(S)]);
        assert_eq!(
            machine.apply(&event(S, 1000, Some("tap"), Phase::STOP), t0 + ms(2400)),
            None
        );
        assert_eq!(machine.expire(t0 + ms(5000)), Vec::new());
    }

    #[test]
    fn the_hold_never_fires_twice_while_the_keys_stay_down() {
        let mut machine = HoldMachine::default();
        let t0 = Instant::now();
        machine.apply(&event(S, 300, None, Phase::START), t0);
        assert_eq!(machine.expire(t0 + ms(300)), vec![hold(S)]);
        assert_eq!(machine.expire(t0 + ms(600)), Vec::new());
        assert_eq!(machine.expire(t0 + ms(9000)), Vec::new());
    }

    #[test]
    fn a_release_past_the_deadline_before_the_timer_ran_still_holds() {
        let mut machine = HoldMachine::default();
        let t0 = Instant::now();
        machine.apply(&event(S, 1000, Some("tap"), Phase::START), t0);
        assert_eq!(
            machine.apply(&event(S, 1000, Some("tap"), Phase::STOP), t0 + ms(1001)),
            Some(hold(S))
        );
    }

    #[test]
    fn heartbeats_change_nothing() {
        let mut machine = HoldMachine::default();
        let t0 = Instant::now();
        machine.apply(&event(S, 1000, Some("tap"), Phase::START), t0);
        for step in 1..=5 {
            assert_eq!(
                machine.apply(
                    &event(S, 1000, Some("tap"), Phase::HEARTBEAT),
                    t0 + ms(100 * step)
                ),
                None
            );
        }
        assert_eq!(machine.next_deadline(), Some(t0 + ms(1000)));
        assert_eq!(
            machine.apply(&event(S, 1000, Some("tap"), Phase::STOP), t0 + ms(600)),
            Some(tap(S))
        );
    }

    #[test]
    fn a_reload_in_the_middle_of_a_press_cancels_it() {
        let mut machine = HoldMachine::default();
        let t0 = Instant::now();
        machine.apply(&event(S, 1000, Some("tap"), Phase::START), t0);
        assert_eq!(
            machine.apply(&event(S, 1000, Some("tap"), Phase::CANCEL), t0 + ms(300)),
            None
        );
        assert_eq!(machine.next_deadline(), None);
        assert_eq!(machine.expire(t0 + ms(5000)), Vec::new());
        assert_eq!(
            machine.apply(&event(S, 1000, Some("tap"), Phase::STOP), t0 + ms(5100)),
            None
        );
    }

    #[test]
    fn a_reload_after_the_hold_fired_leaves_nothing_behind() {
        let mut machine = HoldMachine::default();
        let t0 = Instant::now();
        machine.apply(&event(S, 500, Some("tap"), Phase::START), t0);
        assert_eq!(machine.expire(t0 + ms(500)), vec![hold(S)]);
        machine.apply(&event(S, 500, Some("tap"), Phase::CANCEL), t0 + ms(700));
        assert_eq!(
            machine.apply(&event(S, 500, Some("tap"), Phase::STOP), t0 + ms(900)),
            None
        );
    }

    #[test]
    fn two_chords_interleaved_keep_their_own_deadlines() {
        let mut machine = HoldMachine::default();
        let t0 = Instant::now();
        machine.apply(&event(S, 1000, Some("tap"), Phase::START), t0);
        machine.apply(&event(R, 400, Some("tap"), Phase::START), t0 + ms(100));
        assert_eq!(machine.next_deadline(), Some(t0 + ms(500)));
        assert_eq!(machine.expire(t0 + ms(500)), vec![hold(R)]);
        assert_eq!(machine.next_deadline(), Some(t0 + ms(1000)));
        assert_eq!(
            machine.apply(&event(S, 1000, Some("tap"), Phase::STOP), t0 + ms(600)),
            Some(tap(S))
        );
        assert_eq!(
            machine.apply(&event(R, 400, Some("tap"), Phase::STOP), t0 + ms(650)),
            None
        );
        assert_eq!(machine.next_deadline(), None);
    }

    #[test]
    fn a_cancel_for_one_chord_leaves_the_other_running() {
        let mut machine = HoldMachine::default();
        let t0 = Instant::now();
        machine.apply(&event(S, 1000, Some("tap"), Phase::START), t0);
        machine.apply(&event(R, 1000, Some("tap"), Phase::START), t0);
        machine.apply(&event(S, 1000, Some("tap"), Phase::CANCEL), t0 + ms(10));
        assert_eq!(machine.expire(t0 + ms(1000)), vec![hold(R)]);
    }

    #[test]
    fn a_second_press_without_a_release_starts_a_fresh_deadline() {
        let mut machine = HoldMachine::default();
        let t0 = Instant::now();
        machine.apply(&event(S, 1000, Some("tap"), Phase::START), t0);
        assert_eq!(machine.expire(t0 + ms(1000)), vec![hold(S)]);
        machine.apply(&event(S, 1000, Some("tap"), Phase::START), t0 + ms(1500));
        assert_eq!(machine.next_deadline(), Some(t0 + ms(2500)));
        assert_eq!(
            machine.apply(&event(S, 1000, Some("tap"), Phase::STOP), t0 + ms(1600)),
            Some(tap(S))
        );
    }

    #[test]
    fn a_release_with_no_press_is_ignored() {
        let mut machine = HoldMachine::default();
        assert_eq!(
            machine.apply(&event(S, 1000, Some("tap"), Phase::STOP), Instant::now()),
            None
        );
    }

    #[test]
    fn the_thread_fires_the_tap_on_an_early_release() {
        let (sent, fired) = mpsc::channel();
        let resolver = HoldResolver::spawn(move |decision| {
            let _ = sent.send(decision);
        })
        .unwrap();
        resolver.feed(&event(S, 60_000, Some("tap"), Phase::START));
        resolver.feed(&event(S, 60_000, Some("tap"), Phase::STOP));
        assert_eq!(fired.recv_timeout(Duration::from_secs(5)), Ok(tap(S)));
    }

    #[test]
    fn the_thread_fires_the_hold_by_itself_once_the_time_is_up() {
        let (sent, fired) = mpsc::channel();
        let resolver = HoldResolver::spawn(move |decision| {
            let _ = sent.send(decision);
        })
        .unwrap();
        resolver.feed(&event(S, 100, Some("tap"), Phase::START));
        assert_eq!(fired.recv_timeout(Duration::from_secs(5)), Ok(hold(S)));
        resolver.feed(&event(S, 100, Some("tap"), Phase::STOP));
        resolver.feed(&event(R, 60_000, Some("tap"), Phase::START));
        resolver.feed(&event(R, 60_000, Some("tap"), Phase::STOP));
        assert_eq!(fired.recv_timeout(Duration::from_secs(5)), Ok(tap(R)));
    }

    #[test]
    fn the_thread_drops_a_cancelled_press_without_firing() {
        let (sent, fired) = mpsc::channel();
        let resolver = HoldResolver::spawn(move |decision| {
            let _ = sent.send(decision);
        })
        .unwrap();
        resolver.feed(&event(S, 100, Some("tap"), Phase::START));
        resolver.feed(&event(S, 100, Some("tap"), Phase::CANCEL));
        resolver.feed(&event(R, 60_000, Some("tap"), Phase::START));
        resolver.feed(&event(R, 60_000, Some("tap"), Phase::STOP));
        assert_eq!(fired.recv_timeout(Duration::from_secs(5)), Ok(tap(R)));
        assert!(fired.recv_timeout(Duration::from_millis(300)).is_err());
    }
}
