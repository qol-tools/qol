use proptest::prelude::*;

mod common;
use common::config;

use gpui_test::{action_for_modifiers, action_hint, LaunchAction};

#[test]
fn prop_ctrl_has_priority() {
    proptest!(config(), |(shift in any::<bool>(), alt in any::<bool>())| {
        let action = action_for_modifiers(true, shift, alt);
        prop_assert_eq!(
            action,
            LaunchAction::Terminal,
            "ctrl with shift={} alt={} returned {:?}",
            shift,
            alt,
            action
        );
    });
}

#[test]
fn prop_shift_has_priority_when_no_ctrl() {
    proptest!(config(), |(alt in any::<bool>())| {
        let action = action_for_modifiers(false, true, alt);
        prop_assert_eq!(
            action,
            LaunchAction::OpenFolder,
            "shift with alt={} returned {:?}",
            alt,
            action
        );
    });
}

#[test]
fn prop_alt_has_priority_when_no_ctrl_or_shift() {
    proptest!(config(), |(_ in Just(()))| {
        let action = action_for_modifiers(false, false, true);
        prop_assert_eq!(
            action,
            LaunchAction::CopyPath,
            "alt only returned {:?}",
            action
        );
    });
}

#[test]
fn prop_no_modifiers_defaults_to_open() {
    proptest!(config(), |(_ in Just(()))| {
        let action = action_for_modifiers(false, false, false);
        prop_assert_eq!(
            action,
            LaunchAction::Open,
            "no modifiers returned {:?}",
            action
        );
    });
}

#[test]
fn prop_hint_matches_action_when_modified() {
    proptest!(config(), |(ctrl in any::<bool>(), shift in any::<bool>(), alt in any::<bool>())| {
        prop_assume!(ctrl || shift || alt);
        let action = action_for_modifiers(ctrl, shift, alt);
        let hint = action_hint(ctrl, shift, alt);
        prop_assert_eq!(
            hint,
            Some(action),
            "ctrl={} shift={} alt={} action={:?} hint={:?}",
            ctrl,
            shift,
            alt,
            action,
            hint
        );
    });
}
