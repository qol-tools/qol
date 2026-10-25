use super::catalog::Declaration;
use super::slot::{self, SlotContext, SlotHost};
use super::store;
use crate::hotkeys::HotkeyLayers;
use anyhow::{anyhow, bail, Result};
use qol_host_defaults::{HostDefaults, Ledger, Lifetime, OpenerSpec, ReleaseOutcome, Role, State};
use std::path::PathBuf;

pub(super) struct Env<'a, H: HostDefaults> {
    pub host: &'a H,
    pub ledger: &'a Ledger,
    pub layers: &'a HotkeyLayers,
    pub slots: &'a dyn SlotHost,
    pub exec: PathBuf,
    pub lifetime: Lifetime,
}

impl<H: HostDefaults> Env<'_, H> {
    fn slot_context(&self) -> SlotContext<'_> {
        SlotContext {
            layers: self.layers,
            ledger: self.ledger,
            host: self.slots,
        }
    }
}

pub(super) fn lifetime_label(lifetime: Lifetime) -> &'static str {
    match lifetime {
        Lifetime::PortableSession => "a portable session",
        Lifetime::ResidentPolicy => "a resident policy",
    }
}

pub(super) fn admit(
    role: Role,
    state: &State,
    lifetimes: &[Lifetime],
    lifetime: Lifetime,
) -> Result<()> {
    let id = role.id();
    match state {
        State::Silent => {}
        State::NeedsConfirmation { ask } => bail!("{id} needs your confirmation: {ask}"),
        State::NeedsPermission { ask } => bail!("{id} needs a permission first: {ask}"),
        State::Impossible { reason } => bail!("{id} cannot be taken on this computer: {reason}"),
        State::Unknown { reason } => bail!("{id} could not be checked: {reason}"),
    }
    if lifetimes.contains(&lifetime) {
        return Ok(());
    }
    bail!(
        "{id} cannot be kept as {} on this computer",
        lifetime_label(lifetime)
    )
}

pub(super) fn claim<H: HostDefaults>(env: &Env<'_, H>, declaration: &Declaration) -> Result<()> {
    let role = declaration.role;
    let report = env.host.probe(role);
    admit(role, &report.state, &report.lifetimes, env.lifetime)?;
    release_other_holder(env, declaration)?;
    if !role.is_slot() {
        return env.host.claim_handler(
            role,
            &OpenerSpec {
                exec: env.exec.clone(),
            },
            &declaration.plugin_id,
            env.lifetime,
            env.ledger,
        );
    }
    let route = env
        .host
        .slot_route(role)
        .ok_or_else(|| anyhow!("{} has no key route on this computer", role.id()))?;
    slot::claim_slot(
        &env.slot_context(),
        declaration,
        &route.keys,
        &route.reaches,
        env.lifetime,
    )
}

fn release_other_holder<H: HostDefaults>(
    env: &Env<'_, H>,
    declaration: &Declaration,
) -> Result<()> {
    let holder = env.ledger.holder(declaration.role)?;
    match holder {
        Some(holder) if holder != declaration.plugin_id => {
            release(env, declaration.role, &[]).map(drop)
        }
        Some(_) | None => Ok(()),
    }
}

pub(super) fn release<H: HostDefaults>(
    env: &Env<'_, H>,
    role: Role,
    disable: &[Declaration],
) -> Result<ReleaseOutcome> {
    if !role.is_slot() {
        return env.host.release(role, env.ledger);
    }
    let outcome = release_slot_role(env, role, disable)?;
    if has_records(env.ledger, role)? {
        env.host.release(role, env.ledger)?;
    }
    Ok(outcome)
}

pub(super) fn release_slot_role<H: HostDefaults>(
    env: &Env<'_, H>,
    role: Role,
    disable: &[Declaration],
) -> Result<ReleaseOutcome> {
    let keys = env
        .host
        .slot_route(role)
        .map(|route| route.reaches)
        .unwrap_or_default();
    slot::release_slot(&env.slot_context(), role, &keys, disable)
}

fn has_records(ledger: &Ledger, role: Role) -> Result<bool> {
    Ok(store::claims(ledger)?
        .iter()
        .any(|record| record.role == role.id()))
}

pub(super) fn outcome_label(outcome: &ReleaseOutcome) -> &'static str {
    match outcome {
        ReleaseOutcome::Restored => "restored",
        ReleaseOutcome::LeftUserChange => "left_user_change",
        ReleaseOutcome::AlreadyGone => "already_gone",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROLE: Role = Role::SlotScreenshot;

    #[test]
    fn a_silent_state_with_the_lifetime_is_admitted() {
        assert!(admit(
            ROLE,
            &State::Silent,
            &[Lifetime::PortableSession],
            Lifetime::PortableSession
        )
        .is_ok());
    }

    #[test]
    fn a_missing_lifetime_names_what_cannot_be_kept() {
        let error = admit(
            ROLE,
            &State::Silent,
            &[Lifetime::ResidentPolicy],
            Lifetime::PortableSession,
        )
        .expect_err("refused")
        .to_string();
        assert!(
            error.contains("slot.screenshot") && error.contains("portable"),
            "{error}"
        );
    }

    #[test]
    fn every_state_but_silent_is_refused_with_its_reason() {
        let cases = [
            (
                State::NeedsConfirmation {
                    ask: "Windows asks".to_string(),
                },
                "Windows asks",
            ),
            (
                State::NeedsPermission {
                    ask: "grant Accessibility".to_string(),
                },
                "grant Accessibility",
            ),
            (
                State::Impossible {
                    reason: "Finder owns it".to_string(),
                },
                "Finder owns it",
            ),
            (
                State::Unknown {
                    reason: "dconf missing".to_string(),
                },
                "dconf missing",
            ),
        ];
        for (state, expected) in cases {
            let error = admit(
                ROLE,
                &state,
                &[Lifetime::PortableSession],
                Lifetime::PortableSession,
            )
            .expect_err("refused")
            .to_string();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn outcomes_have_stable_labels() {
        assert_eq!(outcome_label(&ReleaseOutcome::Restored), "restored");
        assert_eq!(
            outcome_label(&ReleaseOutcome::LeftUserChange),
            "left_user_change"
        );
        assert_eq!(outcome_label(&ReleaseOutcome::AlreadyGone), "already_gone");
    }
}
