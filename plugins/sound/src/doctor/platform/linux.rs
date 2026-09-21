use qol_headless::DoctorCheckResult;

use super::super::{Check, PLATFORM_SUPPORTED, PLATFORM_SUPPORTED_ABOUT};

const SERVER_REACHABLE: &str = "server_reachable";
const SWITCHING_AVAILABLE: &str = "switching_available";
const OWNERSHIP_RECORD: &str = "ownership_record";
const DEFAULT_LOCK: &str = "default_lock";
const SAVED_OUTPUT: &str = "saved_output";

pub(crate) const CHECKS: &[Check] = &[
    Check {
        id: PLATFORM_SUPPORTED,
        about: PLATFORM_SUPPORTED_ABOUT,
        run: platform_supported_check,
    },
    Check {
        id: SERVER_REACHABLE,
        about: "Ask the audio server for its outputs without changing anything.",
        run: server_reachable_check,
    },
    Check {
        id: SWITCHING_AVAILABLE,
        about: "Ask whether a default-output change could be undone before one is made.",
        run: switching_available_check,
    },
    Check {
        id: OWNERSHIP_RECORD,
        about: "Read the durable ownership record without claiming or releasing anything.",
        run: ownership_record_check,
    },
    Check {
        id: DEFAULT_LOCK,
        about: "Read the default sound lock holder without holding the lock.",
        run: default_lock_check,
    },
    Check {
        id: SAVED_OUTPUT,
        about: "Resolve the saved output against the live system without switching.",
        run: saved_output_check,
    },
];

fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        PLATFORM_SUPPORTED,
        "Sound has a Linux backend on this system.",
    )
}

fn server_reachable_check() -> DoctorCheckResult {
    match qol_audio::devices::list_outputs() {
        Ok(outputs) => DoctorCheckResult::ok(
            SERVER_REACHABLE,
            format!(
                "The audio server answered a read-only output query with {} output(s).",
                outputs.len()
            ),
        ),
        Err(error) => DoctorCheckResult::fail(
            SERVER_REACHABLE,
            format!(
                "The audio server could not be read: {error}. Start the sound server, then run `plugin-sound doctor` again."
            ),
        ),
    }
}

fn switching_available_check() -> DoctorCheckResult {
    use qol_audio::default_output::ReleaseCapability;
    use qol_audio::devices::Direction;

    match qol_audio::default_output::release_capability(Direction::Output) {
        Ok(ReleaseCapability::Nonpersistent) => DoctorCheckResult::ok(
            SWITCHING_AVAILABLE,
            "Switching is available and is nonpersistent (Nonpersistent), so a killed owner leaves nothing behind.",
        ),
        Ok(ReleaseCapability::ExactRestore) => DoctorCheckResult::ok(
            SWITCHING_AVAILABLE,
            "Switching is available and can be undone exactly (ExactRestore).",
        ),
        Ok(ReleaseCapability::Unavailable(reason)) => {
            DoctorCheckResult::fail(SWITCHING_AVAILABLE, reason)
        }
        Err(error) => DoctorCheckResult::fail(
            SWITCHING_AVAILABLE,
            format!(
                "The switching capability could not be read: {error}. Start the sound server, then run `plugin-sound doctor` again."
            ),
        ),
    }
}

fn ownership_record_check() -> DoctorCheckResult {
    ownership_record_result(qol_audio::attempts::record::read_ownership())
}

fn ownership_record_result(
    state: Result<qol_audio::attempts::record::OwnershipState, qol_audio::AudioError>,
) -> DoctorCheckResult {
    use qol_audio::attempts::record::OwnershipState;

    match state {
        Ok(OwnershipState::None) => {
            DoctorCheckResult::ok(OWNERSHIP_RECORD, "Sound holds no stored output choice.")
        }
        Ok(OwnershipState::Held(ownership)) => DoctorCheckResult::ok(
            OWNERSHIP_RECORD,
            format!(
                "{} holds output {} under epoch {} for a {:?} lifetime. If that claim is stuck, run `plugin-sound give-back`.",
                ownership.owner, ownership.output, ownership.epoch, ownership.lifetime
            ),
        ),
        Ok(OwnershipState::Unreadable(reason)) => DoctorCheckResult::fail(
            OWNERSHIP_RECORD,
            format!(
                "The stored sound choice cannot be read: {reason}. Run `plugin-sound give-back --abandon` to drop it, or repair the Sound state directory, then run `plugin-sound doctor` again."
            ),
        )
        .with_fix("plugin-sound give-back --abandon"),
        Err(error) => DoctorCheckResult::fail(
            OWNERSHIP_RECORD,
            format!(
                "The stored sound choice could not be read: {error}. Check that the Sound state directory is available, then run `plugin-sound doctor` again."
            ),
        ),
    }
}

fn default_lock_check() -> DoctorCheckResult {
    use qol_audio::attempts::lease::{holder, Scope};

    default_lock_result(holder(&Scope::GlobalDefault))
}

fn default_lock_result(
    verdict: Result<Option<qol_audio::attempts::lease::Holder>, qol_audio::AudioError>,
) -> DoctorCheckResult {
    match verdict {
        Ok(None) => DoctorCheckResult::ok(DEFAULT_LOCK, "The default sound lock is free."),
        Ok(Some(info)) => DoctorCheckResult::warn(
            DEFAULT_LOCK,
            format!(
                "The default sound lock is held by pid {} ({}), since unix time {}. If that holder is stuck, run `plugin-sound give-back`.",
                info.pid, info.owner, info.since_unix_seconds
            ),
        )
        .with_fix("plugin-sound give-back"),
        Err(error) => DoctorCheckResult::fail(
            DEFAULT_LOCK,
            format!(
                "The default sound lock could not be read: {error}. Check that the Sound state directory and its locks are readable, then run `plugin-sound doctor` again."
            ),
        ),
    }
}

fn saved_output_check() -> DoctorCheckResult {
    use qol_audio::devices::{resolve, Direction, Resolution};

    let inspection = match crate::config::inspect() {
        Ok(inspection) => inspection,
        Err(error) => {
            return DoctorCheckResult::fail(
                SAVED_OUTPUT,
                format!(
                    "The saved sound output could not be read: {error}. Repair or remove the Sound config file, then run `plugin-sound doctor` again."
                ),
            )
        }
    };
    let requested = inspection.config.output.device;
    if requested == "default" {
        return DoctorCheckResult::ok(
            SAVED_OUTPUT,
            "Sound is set to System Default, so there is no saved output to resolve.",
        );
    }
    match resolve(Direction::Output, &requested) {
        Ok(Resolution::Resolved(device)) => DoctorCheckResult::ok(
            SAVED_OUTPUT,
            format!(
                "The saved output {requested} resolves to {}.",
                device.label
            ),
        ),
        Ok(Resolution::Ambiguous(devices)) => DoctorCheckResult::fail(
            SAVED_OUTPUT,
            format!(
                "The saved output {requested} matches {} available outputs. Run `plugin-sound outputs` and choose the exact value in the Sound settings.",
                devices.len()
            ),
        ),
        Ok(Resolution::NotFound) => DoctorCheckResult::fail(
            SAVED_OUTPUT,
            format!(
                "The saved output {requested} is not available on this system. Run `plugin-sound outputs` and choose a connected output in the Sound settings."
            ),
        ),
        Err(error) => DoctorCheckResult::fail(
            SAVED_OUTPUT,
            format!(
                "The saved output {requested} could not be checked: {error}. Start the sound server, then run `plugin-sound doctor` again."
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_audio::attempts::lease::Holder;
    use qol_audio::attempts::record::OwnershipState;
    use qol_audio::AudioError;
    use qol_headless::DoctorStatus;

    #[test]
    fn a_free_default_lock_reports_ok() {
        let result = default_lock_result(Ok(None));
        assert_eq!(result.id, DEFAULT_LOCK);
        assert_eq!(result.status, DoctorStatus::Ok);
        assert_eq!(result.message, "The default sound lock is free.");
        assert!(result.fix.is_none());
    }

    #[test]
    fn a_held_default_lock_names_the_holder_and_the_give_back() {
        let result = default_lock_result(Ok(Some(Holder {
            owner: "plugin-bluetooth".to_string(),
            pid: 4242,
            since_unix_seconds: 1_700_000_000,
        })));
        assert_eq!(result.id, DEFAULT_LOCK);
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.message.contains("plugin-bluetooth"));
        assert!(result.message.contains("4242"));
        assert_eq!(result.fix.as_deref(), Some("plugin-sound give-back"));
    }

    #[test]
    fn a_default_lock_read_failure_reports_fail() {
        let result = default_lock_result(Err(AudioError::Operation(
            "the lock directory is unavailable".to_string(),
        )));
        assert_eq!(result.id, DEFAULT_LOCK);
        assert_eq!(result.status, DoctorStatus::Fail);
        assert!(result
            .message
            .contains("The default sound lock could not be read"));
        assert!(result.message.contains("the lock directory is unavailable"));
        assert!(result.fix.is_none());
    }

    #[test]
    fn the_production_check_ids_and_fixes_are_pinned() {
        let ids = CHECKS.iter().map(|check| check.id).collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "platform_supported",
                "server_reachable",
                "switching_available",
                "ownership_record",
                "default_lock",
                "saved_output",
            ]
        );
        let unreadable = ownership_record_result(Ok(OwnershipState::Unreadable(
            "failed its checksum".to_string(),
        )));
        assert_eq!(unreadable.id, OWNERSHIP_RECORD);
        assert_eq!(unreadable.status, DoctorStatus::Fail);
        assert_eq!(
            unreadable.fix.as_deref(),
            Some("plugin-sound give-back --abandon")
        );
        let held = default_lock_result(Ok(Some(Holder {
            owner: "plugin-bluetooth".to_string(),
            pid: 4242,
            since_unix_seconds: 1_700_000_000,
        })));
        assert_eq!(held.status, DoctorStatus::Warn);
        assert_eq!(held.fix.as_deref(), Some("plugin-sound give-back"));
    }
}
