use qol_headless::DoctorCheckResult;

use super::super::{Check, PLATFORM_SUPPORTED, PLATFORM_SUPPORTED_ABOUT};

const SERVER_REACHABLE: &str = "server_reachable";
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

    #[test]
    fn the_production_check_ids_are_pinned() {
        let ids = CHECKS.iter().map(|check| check.id).collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec!["platform_supported", "server_reachable", "saved_output"]
        );
    }
}
