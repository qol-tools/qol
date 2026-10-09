use qol_headless::DoctorCheckResult;

use crate::PLUGIN_ID;

use super::super::{Check, PLATFORM_SUPPORTED, PLATFORM_SUPPORTED_ABOUT};

const SERVER_REACHABLE: &str = "server_reachable";
const SERVER_REACHABLE_ABOUT: &str =
    "Ask the audio server for its outputs without changing anything.";
const SAVED_OUTPUT: &str = "saved_output";
const SAVED_OUTPUT_ABOUT: &str =
    "Resolve the saved output against the live system without switching.";

pub(super) trait Host {
    const NAME: &'static str;
    const START: &'static str;
}

pub(super) const fn checks<H: Host>() -> [Check; 3] {
    [
        Check {
            id: PLATFORM_SUPPORTED,
            about: PLATFORM_SUPPORTED_ABOUT,
            run: platform_supported::<H>,
        },
        Check {
            id: SERVER_REACHABLE,
            about: SERVER_REACHABLE_ABOUT,
            run: server_reachable::<H>,
        },
        Check {
            id: SAVED_OUTPUT,
            about: SAVED_OUTPUT_ABOUT,
            run: saved_output::<H>,
        },
    ]
}

fn platform_supported<H: Host>() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        PLATFORM_SUPPORTED,
        format!("Sound has a {} backend on this system.", H::NAME),
    )
}

fn server_reachable<H: Host>() -> DoctorCheckResult {
    let start = H::START;
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
                "The audio server could not be read: {error}. {start}, then run `{PLUGIN_ID} doctor` again."
            ),
        ),
    }
}

fn saved_output<H: Host>() -> DoctorCheckResult {
    use qol_audio::devices::{resolve, Direction, Resolution};

    let start = H::START;
    let inspection = match crate::config::inspect() {
        Ok(inspection) => inspection,
        Err(error) => {
            return DoctorCheckResult::fail(
                SAVED_OUTPUT,
                format!(
                    "The saved sound output could not be read: {error}. Repair or remove the Sound config file, then run `{PLUGIN_ID} doctor` again."
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
                "The saved output {requested} matches {} available outputs. Run `{PLUGIN_ID} outputs` and choose the exact value in the Sound settings.",
                devices.len()
            ),
        ),
        Ok(Resolution::NotFound) => DoctorCheckResult::fail(
            SAVED_OUTPUT,
            format!(
                "The saved output {requested} is not available on this system. Run `{PLUGIN_ID} outputs` and choose a connected output in the Sound settings."
            ),
        ),
        Err(error) => DoctorCheckResult::fail(
            SAVED_OUTPUT,
            format!(
                "The saved output {requested} could not be checked: {error}. {start}, then run `{PLUGIN_ID} doctor` again."
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Probe;

    impl Host for Probe {
        const NAME: &'static str = "Probe";
        const START: &'static str = "Start the probe";
    }

    #[test]
    fn the_production_check_ids_are_pinned() {
        let ids = checks::<Probe>().map(|check| check.id);
        assert_eq!(
            ids,
            ["platform_supported", "server_reachable", "saved_output"]
        );
    }

    #[test]
    fn the_platform_check_names_the_host() {
        let result = platform_supported::<Probe>();
        assert!(result.message.contains("Probe"), "{}", result.message);
    }
}
