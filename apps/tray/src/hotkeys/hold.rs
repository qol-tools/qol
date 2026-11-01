use super::types::{HotkeyAction, HotkeyBinding, HotkeyConfig};
use crate::plugins::PluginUid;
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub(crate) const HOLD_MS_DEFAULT: u32 = 1000;
pub(crate) const HOLD_MS_MIN: u32 = 50;
pub(crate) const HOLD_MS_MAX: u32 = 3000;
pub(crate) const HOLD_MS_STEP: u32 = 50;
const FIRED_STALE_AFTER: Duration = Duration::from_secs(60);

pub(crate) fn clamp_hold_ms(hold_ms: u32) -> u32 {
    hold_ms.clamp(HOLD_MS_MIN, HOLD_MS_MAX)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub(crate) plugin_uid: PluginUid,
    pub(crate) action: String,
}

impl Target {
    pub(crate) fn of(binding: &HotkeyBinding) -> Self {
        Self {
            plugin_uid: binding.plugin_uid.clone(),
            action: binding.action.clone(),
        }
    }

    pub(crate) fn hotkey_action(&self) -> HotkeyAction {
        HotkeyAction {
            plugin_uid: self.plugin_uid.clone(),
            action: self.action.clone(),
            continuous: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HoldPlan {
    pub(crate) hold_ms: u32,
    pub(crate) hold: Target,
    pub(crate) tap: Option<Target>,
}

impl HoldPlan {
    pub(crate) fn duration(&self) -> Duration {
        Duration::from_millis(u64::from(self.hold_ms))
    }

    pub(crate) fn primary(&self) -> &Target {
        self.tap.as_ref().unwrap_or(&self.hold)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fire {
    Tap(Target),
    Hold(Target),
}

#[derive(Debug, Clone)]
pub(crate) struct HoldTimer {
    plan: HoldPlan,
    deadline: Instant,
    fired: bool,
}

impl HoldTimer {
    pub(crate) fn start(plan: HoldPlan, now: Instant) -> Self {
        let deadline = now + plan.duration();
        Self {
            plan,
            deadline,
            fired: false,
        }
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        (!self.fired).then_some(self.deadline)
    }

    pub(crate) fn is_stale(&self, now: Instant) -> bool {
        self.fired && now >= self.deadline + FIRED_STALE_AFTER
    }

    pub(crate) fn expire(&mut self, now: Instant) -> Option<Fire> {
        if self.fired || now < self.deadline {
            return None;
        }
        self.fired = true;
        Some(Fire::Hold(self.plan.hold.clone()))
    }

    pub(crate) fn release(self, now: Instant) -> Option<Fire> {
        if self.fired {
            return None;
        }
        if now >= self.deadline {
            return Some(Fire::Hold(self.plan.hold));
        }
        self.plan.tap.map(Fire::Tap)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChordMember {
    pub(crate) target: Target,
    pub(crate) hold_ms: Option<u32>,
    pub(crate) continuous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TimedChord {
    pub(crate) leader: usize,
    pub(crate) plan: HoldPlan,
}

pub(crate) fn timed_chord(members: &[ChordMember]) -> Option<TimedChord> {
    if members.iter().any(|member| member.continuous) {
        return None;
    }
    let hold = members.iter().position(|member| member.hold_ms.is_some())?;
    let tap = members.iter().position(|member| member.hold_ms.is_none());
    let dropped = members.len() - 1 - usize::from(tap.is_some());
    if dropped > 0 {
        log::warn!(
            "hotkey chord carries more than one tap or hold binding; ignoring {dropped} extra"
        );
    }
    Some(TimedChord {
        leader: tap.map_or(hold, |tap| tap.min(hold)),
        plan: HoldPlan {
            hold_ms: clamp_hold_ms(members[hold].hold_ms.unwrap_or(HOLD_MS_DEFAULT)),
            hold: members[hold].target.clone(),
            tap: tap.map(|tap| members[tap].target.clone()),
        },
    })
}

pub(crate) fn duplicate_slot(
    config: &HotkeyConfig,
    chord_id: impl Fn(&str) -> Option<u32>,
) -> Option<String> {
    let mut taken: HashSet<(u32, bool)> = HashSet::new();
    config
        .hotkeys
        .iter()
        .filter(|binding| binding.enabled)
        .filter_map(|binding| chord_id(&binding.key).map(|id| (binding, id)))
        .find_map(|(binding, id)| {
            let slot = (id, binding.hold_ms.is_some());
            (!taken.insert(slot)).then(|| binding.key.clone())
        })
}

pub(crate) fn invalid_hold(
    config: &HotkeyConfig,
    chord_id: impl Fn(&str) -> Option<u32>,
    is_continuous: impl Fn(&HotkeyBinding) -> bool,
) -> Option<String> {
    if let Some(binding) = config
        .hotkeys
        .iter()
        .find(|binding| binding.hold_ms.is_some_and(|ms| clamp_hold_ms(ms) != ms))
    {
        return Some(format!(
            "Hold time for {} must be between {HOLD_MS_MIN} and {HOLD_MS_MAX} ms",
            binding.key
        ));
    }
    let continuous_chords: HashSet<u32> = config
        .hotkeys
        .iter()
        .filter(|binding| binding.enabled && is_continuous(binding))
        .filter_map(|binding| chord_id(&binding.key))
        .collect();
    config
        .hotkeys
        .iter()
        .filter(|binding| binding.hold_ms.is_some())
        .find(|binding| {
            binding.enabled
                && (is_continuous(binding)
                    || chord_id(&binding.key).is_some_and(|id| continuous_chords.contains(&id)))
        })
        .map(|binding| {
            format!(
                "{} cannot run on a hold: a continuous action already uses it or is the one held",
                binding.key
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(action: &str) -> Target {
        Target {
            plugin_uid: PluginUid::new("uid-shot"),
            action: action.into(),
        }
    }

    fn member(action: &str, hold_ms: Option<u32>, continuous: bool) -> ChordMember {
        ChordMember {
            target: target(action),
            hold_ms,
            continuous,
        }
    }

    fn plan(hold_ms: u32, tap: Option<&str>) -> HoldPlan {
        HoldPlan {
            hold_ms,
            hold: target("region"),
            tap: tap.map(target),
        }
    }

    fn binding(id: &str, key: &str, action: &str, hold_ms: Option<u32>) -> HotkeyBinding {
        HotkeyBinding {
            id: id.into(),
            key: key.into(),
            plugin_uid: PluginUid::new("uid-shot"),
            action: action.into(),
            enabled: true,
            os_only: false,
            hold_ms,
        }
    }

    fn config(bindings: Vec<HotkeyBinding>) -> HotkeyConfig {
        HotkeyConfig { hotkeys: bindings }
    }

    fn chord_id(key: &str) -> Option<u32> {
        match key.to_ascii_lowercase().as_str() {
            "shift+super+s" => Some(1),
            "shift+super+r" => Some(2),
            _ => None,
        }
    }

    #[test]
    fn a_timer_before_its_deadline_releases_to_the_tap() {
        let t0 = Instant::now();
        let timer = HoldTimer::start(plan(1000, Some("monitor")), t0);
        assert_eq!(
            timer.release(t0 + Duration::from_millis(999)),
            Some(Fire::Tap(target("monitor")))
        );
    }

    #[test]
    fn a_timer_before_its_deadline_without_a_tap_releases_to_nothing() {
        let t0 = Instant::now();
        let timer = HoldTimer::start(plan(1000, None), t0);
        assert_eq!(timer.release(t0 + Duration::from_millis(10)), None);
    }

    #[test]
    fn a_release_after_the_deadline_still_runs_the_hold_when_the_timer_lagged() {
        let t0 = Instant::now();
        let timer = HoldTimer::start(plan(1000, Some("monitor")), t0);
        assert_eq!(
            timer.release(t0 + Duration::from_millis(1000)),
            Some(Fire::Hold(target("region")))
        );
    }

    #[test]
    fn an_expired_timer_fires_once_and_its_release_does_nothing() {
        let t0 = Instant::now();
        let mut timer = HoldTimer::start(plan(500, Some("monitor")), t0);
        assert_eq!(timer.expire(t0 + Duration::from_millis(499)), None);
        assert_eq!(timer.deadline(), Some(t0 + Duration::from_millis(500)));
        assert_eq!(
            timer.expire(t0 + Duration::from_millis(500)),
            Some(Fire::Hold(target("region")))
        );
        assert_eq!(timer.deadline(), None);
        assert_eq!(timer.expire(t0 + Duration::from_millis(900)), None);
        assert_eq!(timer.release(t0 + Duration::from_millis(900)), None);
    }

    #[test]
    fn a_fired_timer_turns_stale_after_a_minute_without_a_release() {
        let t0 = Instant::now();
        let mut timer = HoldTimer::start(plan(500, None), t0);
        assert!(!timer.is_stale(t0 + Duration::from_secs(600)));
        timer.expire(t0 + ms_of(500));
        assert!(!timer.is_stale(t0 + Duration::from_secs(59)));
        assert!(timer.is_stale(t0 + Duration::from_secs(61)));
    }

    fn ms_of(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_disabled_hold_binding_never_blocks_a_save() {
        let mut off = binding("a", "Shift+Super+S", "glide-left", Some(1000));
        off.enabled = false;
        let config = config(vec![off]);
        assert_eq!(
            invalid_hold(&config, chord_id, |b| b.action.starts_with("glide")),
            None
        );
    }

    #[test]
    fn a_chord_without_a_hold_member_stays_plain() {
        assert_eq!(timed_chord(&[member("a", None, false)]), None);
        assert_eq!(
            timed_chord(&[member("a", None, false), member("b", None, false)]),
            None
        );
    }

    #[test]
    fn a_continuous_member_keeps_its_chord_plain() {
        assert_eq!(
            timed_chord(&[
                member("glide", None, true),
                member("region", Some(800), false)
            ]),
            None
        );
    }

    #[test]
    fn a_hold_alone_or_beside_a_tap_makes_a_timed_chord() {
        let alone = timed_chord(&[member("region", Some(800), false)]).unwrap();
        assert_eq!(alone.leader, 0);
        assert_eq!(alone.plan.tap, None);
        assert_eq!(alone.plan.hold_ms, 800);

        let beside = timed_chord(&[
            member("region", Some(1500), false),
            member("monitor", None, false),
        ])
        .unwrap();
        assert_eq!(beside.leader, 0);
        assert_eq!(beside.plan.hold, target("region"));
        assert_eq!(beside.plan.tap, Some(target("monitor")));
        assert_eq!(beside.plan.primary(), &target("monitor"));
        assert_eq!(beside.plan.hold_ms, 1500);
    }

    #[test]
    fn the_hold_time_is_kept_inside_its_limits() {
        for (asked, expected) in [
            (0, HOLD_MS_MIN),
            (49, HOLD_MS_MIN),
            (50, 50),
            (1000, 1000),
            (3000, 3000),
            (60_000, HOLD_MS_MAX),
        ] {
            let timed = timed_chord(&[member("region", Some(asked), false)]).unwrap();
            assert_eq!(timed.plan.hold_ms, expected, "asked {asked}");
        }
    }

    #[test]
    fn one_tap_and_one_hold_may_share_a_chord() {
        let config = config(vec![
            binding("a", "Shift+Super+S", "monitor", None),
            binding("b", "shift+super+s", "region", Some(1000)),
        ]);
        assert_eq!(duplicate_slot(&config, chord_id), None);
    }

    #[test]
    fn two_taps_or_two_holds_on_a_chord_are_duplicates() {
        let two_taps = config(vec![
            binding("a", "Shift+Super+S", "monitor", None),
            binding("b", "shift+super+s", "region", None),
        ]);
        assert_eq!(
            duplicate_slot(&two_taps, chord_id),
            Some("shift+super+s".to_string())
        );
        let two_holds = config(vec![
            binding("a", "Shift+Super+S", "monitor", Some(500)),
            binding("b", "shift+super+s", "region", Some(1000)),
        ]);
        assert_eq!(
            duplicate_slot(&two_holds, chord_id),
            Some("shift+super+s".to_string())
        );
    }

    #[test]
    fn a_disabled_binding_and_other_chords_never_duplicate() {
        let mut off = binding("b", "Shift+Super+S", "region", None);
        off.enabled = false;
        let config = config(vec![
            binding("a", "Shift+Super+S", "monitor", None),
            off,
            binding("c", "Shift+Super+R", "record", None),
            binding("d", "Shift+Super+R", "stop", Some(1000)),
            binding("e", "Unparsable", "x", None),
            binding("f", "Unparsable", "y", None),
        ]);
        assert_eq!(duplicate_slot(&config, chord_id), None);
    }

    #[test]
    fn a_hold_time_outside_its_limits_is_refused() {
        for (ms, refused) in [
            (Some(0), true),
            (Some(49), true),
            (Some(50), false),
            (Some(3000), false),
            (Some(3001), true),
        ] {
            let config = config(vec![binding("a", "Shift+Super+S", "region", ms)]);
            assert_eq!(
                invalid_hold(&config, chord_id, |_| false).is_some(),
                refused,
                "{ms:?}"
            );
        }
        let none = config(vec![binding("a", "Shift+Super+S", "region", None)]);
        assert_eq!(invalid_hold(&none, chord_id, |_| false), None);
    }

    #[test]
    fn a_hold_on_a_continuous_action_is_refused() {
        let config = config(vec![binding(
            "a",
            "Shift+Super+S",
            "glide-left",
            Some(1000),
        )]);
        assert!(invalid_hold(&config, chord_id, |b| b.action.starts_with("glide")).is_some());
        assert_eq!(invalid_hold(&config, chord_id, |_| false), None);
    }

    #[test]
    fn a_hold_beside_a_continuous_action_on_one_chord_is_refused() {
        let config = config(vec![
            binding("a", "Shift+Super+S", "glide-left", None),
            binding("b", "shift+super+s", "region", Some(1000)),
            binding("c", "Shift+Super+R", "record", Some(1000)),
        ]);
        let found = invalid_hold(&config, chord_id, |b| b.action.starts_with("glide"));
        assert!(found.unwrap().contains("shift+super+s"));
        let elsewhere = config_without_first(&config);
        assert_eq!(
            invalid_hold(&elsewhere, chord_id, |b| b.action.starts_with("glide")),
            None
        );
    }

    fn config_without_first(config: &HotkeyConfig) -> HotkeyConfig {
        HotkeyConfig {
            hotkeys: config.hotkeys[1..].to_vec(),
        }
    }

    #[test]
    fn the_hold_time_round_trips_through_json_and_is_left_out_when_unset() {
        let with: HotkeyBinding = serde_json::from_str(
            r#"{"id":"a","key":"Shift+Super+S","plugin_uid":"p","action":"x","enabled":true,"hold_ms":1000}"#,
        )
        .unwrap();
        assert_eq!(with.hold_ms, Some(1000));
        let value = serde_json::to_value(&with).unwrap();
        assert_eq!(value["hold_ms"], serde_json::json!(1000));

        let without: HotkeyBinding = serde_json::from_str(
            r#"{"id":"a","key":"Shift+Super+S","plugin_uid":"p","action":"x","enabled":true}"#,
        )
        .unwrap();
        assert_eq!(without.hold_ms, None);
        assert!(serde_json::to_value(&without)
            .unwrap()
            .get("hold_ms")
            .is_none());
    }
}
