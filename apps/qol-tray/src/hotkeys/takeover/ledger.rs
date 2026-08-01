use super::dconf::BindingReach;
use anyhow::{Context, Result};
use qol_host_fixes::takeover;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const OWNER: &str = "qol-tray-hotkeys";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Claim {
    pub dir: String,
    pub key: String,
    pub previous: String,
    pub applied: String,
    pub qol_combo: String,
    pub reach: BindingReach,
    pub recorded_at: SystemTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestoreDecision {
    Rewrite,
    Abandon,
}

pub(crate) fn claims_dir() -> Option<PathBuf> {
    takeover::claims_dir(OWNER)
}

pub(crate) fn component_name(dir: &str, key: &str) -> String {
    super::dconf::full_key(dir, key)
        .trim_start_matches('/')
        .replace('/', ".")
}

pub(crate) fn record(root: &Path, claim: &Claim) -> Result<()> {
    let hint = serde_json::to_string(claim).context("failed to serialize hotkey takeover claim")?;
    takeover::record(
        root,
        &takeover::Claim {
            component: component_name(&claim.dir, &claim.key),
            restore_hint: hint,
        },
    )
}

pub(crate) fn clear(root: &Path, claim: &Claim) -> Result<()> {
    takeover::clear(root, &component_name(&claim.dir, &claim.key))
}

pub(crate) fn outstanding(root: &Path) -> Vec<Claim> {
    takeover::outstanding(root)
        .into_iter()
        .filter_map(|entry| match serde_json::from_str(&entry.restore_hint) {
            Ok(claim) => Some(claim),
            Err(error) => {
                log::warn!(
                    "hotkey takeover: dropping unreadable claim {}: {error}",
                    entry.component
                );
                let _ = takeover::clear(root, &entry.component);
                None
            }
        })
        .collect()
}

pub(crate) fn decide_restore(claim: &Claim, current: Option<&str>) -> RestoreDecision {
    match current {
        None => RestoreDecision::Rewrite,
        Some(value) if value.trim() == claim.applied.trim() => RestoreDecision::Rewrite,
        Some(_) => RestoreDecision::Abandon,
    }
}

pub(crate) fn restart_pending(
    claims: &[Claim],
    compositor_started_at: Option<SystemTime>,
) -> Vec<&Claim> {
    claims
        .iter()
        .filter(|claim| claim.reach == BindingReach::LegacyOrphan)
        .filter(|claim| match compositor_started_at {
            None => true,
            Some(started) => started <= claim.recorded_at,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn claim(dir: &str, reach: BindingReach, recorded_at: SystemTime) -> Claim {
        Claim {
            dir: dir.to_string(),
            key: "binding".into(),
            previous: "['<Shift><Super>s']".into(),
            applied: "@as []".into(),
            qol_combo: "Shift+Super+S".into(),
            reach,
            recorded_at,
        }
    }

    #[test]
    fn component_name_is_a_flat_collision_free_encoding_of_the_dconf_key() {
        let cases = [
            (
                "/org/cinnamon/desktop/keybindings/custom2/",
                "binding",
                "org.cinnamon.desktop.keybindings.custom2.binding",
            ),
            (
                "/org/cinnamon/desktop/keybindings/wm/",
                "close",
                "org.cinnamon.desktop.keybindings.wm.close",
            ),
            (
                "/desktop/ibus/general/hotkey/",
                "triggers",
                "desktop.ibus.general.hotkey.triggers",
            ),
        ];
        for (dir, key, want) in cases {
            assert_eq!(component_name(dir, key), want, "dir: {dir}");
        }
    }

    #[test]
    fn component_names_of_sibling_keys_never_collide() {
        let a = component_name("/org/cinnamon/desktop/keybindings/custom2/", "binding");
        let b = component_name("/org/cinnamon/desktop/keybindings/custom20/", "binding");
        let c = component_name("/org/cinnamon/desktop/keybindings/custom2/", "bindings");
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn record_then_outstanding_round_trips_every_field() {
        let dir = tempfile::tempdir().expect("tempdir");
        let entry = claim(
            "/org/cinnamon/desktop/keybindings/custom2/",
            BindingReach::LegacyOrphan,
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        );
        record(dir.path(), &entry).expect("record");
        assert_eq!(outstanding(dir.path()), vec![entry.clone()]);
        clear(dir.path(), &entry).expect("clear");
        assert!(outstanding(dir.path()).is_empty());
    }

    #[test]
    fn a_corrupt_marker_is_dropped_rather_than_blocking_every_later_restore() {
        let dir = tempfile::tempdir().expect("tempdir");
        let good = claim(
            "/org/cinnamon/desktop/keybindings/wm/",
            BindingReach::Managed,
            SystemTime::UNIX_EPOCH,
        );
        record(dir.path(), &good).expect("record");
        takeover::record(
            dir.path(),
            &takeover::Claim {
                component: "org.broken.entry".into(),
                restore_hint: "{not json".into(),
            },
        )
        .expect("record corrupt");

        assert_eq!(outstanding(dir.path()), vec![good]);
        assert!(
            !takeover::is_claimed(dir.path(), "org.broken.entry"),
            "an unreadable claim must be cleared so it stops being re-read every boot"
        );
    }

    #[test]
    fn restore_is_abandoned_when_the_user_changed_the_binding_behind_us() {
        let entry = claim(
            "/org/cinnamon/desktop/keybindings/custom2/",
            BindingReach::LegacyOrphan,
            SystemTime::UNIX_EPOCH,
        );
        let cases = [
            (None, RestoreDecision::Rewrite),
            (Some("@as []"), RestoreDecision::Rewrite),
            (Some("  @as []  "), RestoreDecision::Rewrite),
            (Some("['<Super>k']"), RestoreDecision::Abandon),
            (Some(""), RestoreDecision::Abandon),
        ];
        for (current, want) in cases {
            assert_eq!(
                decide_restore(&entry, current),
                want,
                "current: {current:?}"
            );
        }
    }

    #[test]
    fn only_orphaned_claims_older_than_the_running_compositor_need_a_restart() {
        let boot = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let before_boot = SystemTime::UNIX_EPOCH + Duration::from_secs(500);
        let after_boot = SystemTime::UNIX_EPOCH + Duration::from_secs(1_500);
        let claims = vec![
            claim(
                "/a/keybindings/custom0/",
                BindingReach::LegacyOrphan,
                after_boot,
            ),
            claim(
                "/b/keybindings/custom1/",
                BindingReach::LegacyOrphan,
                before_boot,
            ),
            claim("/c/keybindings/wm/", BindingReach::Managed, after_boot),
        ];

        let pending = restart_pending(&claims, Some(boot));
        assert_eq!(
            pending.iter().map(|c| c.dir.as_str()).collect::<Vec<_>>(),
            vec!["/a/keybindings/custom0/"],
            "a claim recorded before the compositor started is already live; managed bindings \
             are re-read by the compositor and never need a restart"
        );

        let pending_unknown = restart_pending(&claims, None);
        assert_eq!(
            pending_unknown.len(),
            2,
            "with no compositor evidence, every orphan claim stays pending"
        );
    }
}
