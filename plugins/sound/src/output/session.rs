use anyhow::Context;

use qol_audio::attempts::lease::{self, Scope};
use qol_audio::attempts::record::{self, Lifetime, Ownership, OwnershipState};
use qol_audio::default_output::{self, DefaultSnapshot, ReleaseCapability};
use qol_audio::devices::{self, AudioDevice, Direction, Identity, Resolution};
use qol_audio::AudioError;
use qol_host_session::{SessionSnapshot, SessionStore};

/// The owner of the current choice, and the epoch that makes a late result
/// from a released owner harmless.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub name: String,
    pub epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Released {
    Output,
    Nothing,
}

const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
const SNAPSHOT_ID: &str = "default-output";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct SavedDefault {
    schema_version: u32,
    id: String,
    effective: Option<String>,
    configured: Option<String>,
    fallback_order: Vec<String>,
    server_incarnation: Option<String>,
}

impl SessionSnapshot for SavedDefault {
    const SCHEMA_VERSION: u32 = SNAPSHOT_SCHEMA_VERSION;
    const SUBDIR: &'static str = "default-output";

    fn id(&self) -> &str {
        &self.id
    }

    fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

fn snapshot_store() -> anyhow::Result<SessionStore> {
    let root = qol_config::data_subdir("sound").ok_or_else(|| {
        anyhow::anyhow!("no local data directory is available for the sound session snapshot")
    })?;
    Ok(SessionStore::new(root.join("session")))
}

fn save_snapshot(snapshot: &DefaultSnapshot) -> anyhow::Result<()> {
    let store = snapshot_store()?;
    if store.load::<SavedDefault>(SNAPSHOT_ID)?.is_some() {
        return Ok(());
    }
    store
        .write(&SavedDefault {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            id: SNAPSHOT_ID.to_string(),
            effective: snapshot.effective.clone(),
            configured: snapshot.configured.clone(),
            fallback_order: snapshot.fallback_order.clone(),
            server_incarnation: snapshot.server_incarnation.clone(),
        })
        .context("cannot save the sound session snapshot")
}

fn load_snapshot() -> anyhow::Result<Option<DefaultSnapshot>> {
    let store = snapshot_store()?;
    let saved = store
        .load::<SavedDefault>(SNAPSHOT_ID)
        .context("cannot read the sound session snapshot")?;
    Ok(saved.map(|saved| DefaultSnapshot {
        direction: Direction::Output,
        effective: saved.effective,
        configured: saved.configured,
        fallback_order: saved.fallback_order,
        server_incarnation: saved.server_incarnation,
    }))
}

fn forget_snapshot() -> anyhow::Result<()> {
    snapshot_store()?
        .delete(SNAPSHOT_ID)
        .context("cannot clear the sound session snapshot")
}

fn held_ownership(state: OwnershipState) -> anyhow::Result<Option<Ownership>> {
    match state {
        OwnershipState::None => Ok(None),
        OwnershipState::Held(ownership) => Ok(Some(ownership)),
        OwnershipState::Unreadable(reason) => Err(anyhow::anyhow!(
            "the saved sound output record is unreadable, so Sound refuses to treat it as no choice: {reason}"
        )),
    }
}

fn resolve_output(requested: &str) -> anyhow::Result<AudioDevice> {
    match devices::resolve(Direction::Output, requested) {
        Ok(Resolution::Resolved(device)) => Ok(device),
        Ok(Resolution::Ambiguous(candidates)) => {
            let labels = candidates
                .iter()
                .map(|device| device.label.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!("the sound output `{requested}` is ambiguous between: {labels}")
        }
        Ok(Resolution::NotFound) => {
            anyhow::bail!("no connected sound output matches `{requested}`")
        }
        Err(error) => Err(error).context("cannot resolve the sound output"),
    }
}

fn keep_claim_undo(effective: &Result<Option<Identity>, AudioError>, requested: &Identity) -> bool {
    !matches!(effective, Ok(Some(identity)) if identity != requested)
}

fn roll_back_claim(undo: impl FnOnce(), error: AudioError) -> anyhow::Error {
    undo();
    anyhow::Error::new(error)
}

/// Claims the single Sound owner and saves the complete affected default
/// policy before anything is written.
pub fn claim(output: &str, keep: bool) -> anyhow::Result<Owner> {
    let device = resolve_output(output)?;
    let _lock =
        lease::acquire(Scope::GlobalDefault).context("cannot take the sound output lock")?;
    let previous =
        held_ownership(record::read_ownership().context("cannot read the saved sound output")?)?;
    let snapshot = default_output::capture(Direction::Output)
        .context("cannot read the current default sound output")?;
    let epoch = match &previous {
        None => 1,
        Some(ownership) => ownership
            .epoch
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("the saved sound output epoch is exhausted"))?,
    };
    save_snapshot(&snapshot)?;
    let owner = Owner {
        name: crate::PLUGIN_ID.to_string(),
        epoch,
    };
    let ownership = Ownership {
        owner: owner.name.clone(),
        epoch,
        lifetime: if keep {
            Lifetime::ResidentPolicy
        } else {
            Lifetime::PortableSession
        },
        output: device.identity,
    };
    if let Err(error) = record::write_ownership(&ownership) {
        if previous.is_none() {
            let _ = forget_snapshot();
        }
        return Err(error).context("cannot save the sound output choice");
    }
    match default_output::set(Direction::Output, &ownership.output) {
        Ok(()) => Ok(owner),
        Err(error) => Err(roll_back_claim(
            || match previous.as_ref() {
                Some(previous_ownership) => {
                    let _ = record::write_ownership(previous_ownership);
                }
                None => {
                    let effective = default_output::effective_identity(Direction::Output);
                    if !keep_claim_undo(&effective, &ownership.output) {
                        let _ = record::clear_ownership(epoch);
                        let _ = forget_snapshot();
                    }
                }
            },
            error,
        )
        .context(format!(
            "cannot switch the default sound output to `{output}`"
        ))),
    }
}

/// The one release transition. Choosing System Default, `give-back`, stopping
/// and removal all go through here; none of them sets a default directly.
pub fn release(abandon: bool) -> anyhow::Result<Released> {
    let _lock =
        lease::acquire(Scope::GlobalDefault).context("cannot take the sound output lock")?;
    let state = record::read_ownership().context("cannot read the saved sound output")?;
    let ownership = match state {
        OwnershipState::Unreadable(_) if abandon => None,
        state => match held_ownership(state)? {
            Some(ownership) => Some(ownership),
            None => return Ok(Released::Nothing),
        },
    };
    match default_output::release_capability(Direction::Output) {
        Ok(ReleaseCapability::ExactRestore) => {
            if !abandon {
                let snapshot = load_snapshot()?.ok_or_else(|| {
                    anyhow::anyhow!(
                        "the saved sound session snapshot is missing, so the default sound output cannot be restored; `give-back --abandon` drops the claim without restoring"
                    )
                })?;
                default_output::restore(&snapshot)
                    .context("cannot restore the default sound output")?;
            } else if let Ok(Some(snapshot)) = load_snapshot() {
                let _ = default_output::restore(&snapshot);
            }
        }
        Ok(ReleaseCapability::Nonpersistent) => {}
        Ok(ReleaseCapability::Unavailable(reason)) if !abandon => {
            anyhow::bail!(
                "the default sound output cannot be restored on this system: {reason}; `give-back --abandon` drops the claim without restoring"
            );
        }
        Ok(ReleaseCapability::Unavailable(_)) => {}
        Err(error) if !abandon => {
            return Err(error)
                .context("cannot check whether the default sound output can be restored");
        }
        Err(_) => {}
    }
    forget_snapshot().context("cannot clear the sound session snapshot")?;
    match &ownership {
        Some(ownership) => record::clear_ownership(ownership.epoch)
            .context("cannot clear the saved sound output ownership")?,
        None => {
            record::abandon_ownership().context("cannot drop the unreadable sound output record")?
        }
    }
    Ok(Released::Output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_audio::devices::Identity;

    #[test]
    fn releasing_with_no_owner_succeeds() {
        let held = held_ownership(OwnershipState::None).expect("no owner is not a failure");
        assert!(held.is_none(), "there is nothing to release");
    }

    #[test]
    fn unreadable_ownership_refuses_rather_than_reading_as_no_owner() {
        let error = held_ownership(OwnershipState::Unreadable(
            "failed its checksum".to_string(),
        ))
        .expect_err("a corrupt record must fail closed");
        assert!(error.to_string().contains("unreadable"));
    }

    #[test]
    fn a_failed_set_runs_the_undo_before_returning_the_backend_error() {
        let mut rolled_back = false;
        let error = roll_back_claim(|| rolled_back = true, AudioError::Unsupported);
        assert!(
            rolled_back,
            "the claim is undone before the error is returned"
        );
        assert_eq!(
            error.to_string(),
            AudioError::Unsupported.to_string(),
            "the backend's own reason survives verbatim"
        );
    }

    #[test]
    fn the_claim_undo_is_cleared_only_by_a_different_effective_identity() {
        let requested = Identity::from_raw("speaker-a:analog-stereo:Speaker");
        assert!(keep_claim_undo(&Ok(Some(requested.clone())), &requested));
        assert!(!keep_claim_undo(
            &Ok(Some(Identity::from_raw("speaker-b:analog-stereo:Speaker"))),
            &requested
        ));
        assert!(
            keep_claim_undo(&Ok(None), &requested),
            "no effective default keeps the ownership record and the snapshot"
        );
        assert!(
            keep_claim_undo(&Err(AudioError::Unsupported), &requested),
            "a failed re-read keeps the ownership record and the snapshot"
        );
    }
}
