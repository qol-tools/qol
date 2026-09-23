mod platform;

use std::{io::ErrorKind, path::PathBuf};

use anyhow::Result;
use qol_headless::{DoctorCheck, DoctorCheckResult};
use serde_json::json;

use crate::listen::{audio_input_devices, verify_audio_input};

pub(super) fn checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            "config_readable",
            "Read and validate the typed Voice config without changing it.",
            config_readable_check,
        ),
        DoctorCheck::new(
            "runtime_dirs",
            "Inspect runtime and model-cache directory metadata without creating or reading content.",
            runtime_dirs_check,
        ),
        DoctorCheck::new(
            "external_services",
            "Probe only the explicitly configured remote transcription service without sending audio.",
            external_services_check,
        ),
        DoctorCheck::new(
            "audio_capture",
            "Verify the audio backend exposes at least one microphone.",
            audio_capture_check,
        ),
        DoctorCheck::new(
            "transcription_providers",
            "Verify at least one engine-neutral STT provider is registered.",
            transcription_provider_check,
        ),
        DoctorCheck::new(
            "speech_output",
            "Report the current speech-output capability.",
            speech_output_check,
        ),
    ]
}

fn config_readable_check() -> Result<DoctorCheckResult> {
    let inspection = match crate::config::inspect() {
        Ok(inspection) => inspection,
        Err(error) => {
            return Ok(
                DoctorCheckResult::fail("config_readable", error.to_string())
                    .with_fix("Repair or remove the invalid QoL Voice config file")
                    .with_details(json!({
                        "inspection": "read_only",
                        "config_changed": false,
                        "parse_markers_changed": false,
                    })),
            );
        }
    };
    let source = inspection
        .source
        .as_ref()
        .map(|path| path.display().to_string());
    let message = source.as_ref().map_or_else(
        || "No config file found; typed contract defaults are valid.".to_string(),
        |path| format!("Config at {path} is readable and matches the typed contract."),
    );
    Ok(
        DoctorCheckResult::ok("config_readable", message).with_details(json!({
            "source": source,
            "recognition_enabled": inspection.config.recognition.enabled,
            "recognition_provider": inspection.config.recognition.provider,
            "inspection": "read_only",
            "config_changed": false,
            "parse_markers_changed": false,
        })),
    )
}

fn runtime_dirs_check() -> Result<DoctorCheckResult> {
    let mut paths = Vec::new();
    let Some(runtime_dir) = qol_config::runtime_dir() else {
        return Ok(DoctorCheckResult::fail(
            "runtime_dirs",
            "The QoL runtime directory cannot be resolved.",
        )
        .with_fix("Run with a valid platform user-data directory."));
    };
    paths.push(PathSpec {
        label: "qol_runtime",
        path: runtime_dir,
    });
    if let Some(model_cache) = platform::model_cache_dir() {
        paths.push(PathSpec {
            label: "huggingface_model_cache",
            path: model_cache,
        });
    }
    Ok(runtime_dirs_result(paths))
}

fn runtime_dirs_result(paths: Vec<PathSpec>) -> DoctorCheckResult {
    let observations = paths.into_iter().map(PathSpec::observe).collect::<Vec<_>>();
    let failures = observations
        .iter()
        .filter(|observation| {
            matches!(
                observation.state,
                PathState::WrongType | PathState::Unreadable(_)
            ) || observation.readonly == Some(true)
        })
        .count();
    let missing = observations
        .iter()
        .filter(|observation| observation.state == PathState::Missing)
        .count();
    let symlinks = observations
        .iter()
        .filter(|observation| observation.state == PathState::Symlink)
        .count();
    let details = json!({
        "paths": observations.iter().map(PathObservation::details).collect::<Vec<_>>(),
        "content_read": false,
        "created": false,
        "model_download_attempted": false,
    });
    if failures > 0 {
        return DoctorCheckResult::fail(
            "runtime_dirs",
            format!("{failures} Voice runtime path(s) are unusable."),
        )
        .with_fix("Repair the reported runtime or model-cache directory paths.")
        .with_details(details);
    }
    if symlinks > 0 {
        return DoctorCheckResult::warn(
            "runtime_dirs",
            format!("{symlinks} Voice runtime path(s) are symbolic links."),
        )
        .with_fix("Replace symbolic links with directly owned runtime or model-cache directories.")
        .with_details(details);
    }
    if missing > 0 {
        return DoctorCheckResult::warn(
            "runtime_dirs",
            format!(
                "{missing} Voice runtime path(s) are absent and will only be created by operational use."
            ),
        )
        .with_fix("Ensure the reported parent directories are writable.")
        .with_details(details);
    }
    DoctorCheckResult::ok(
        "runtime_dirs",
        "Voice runtime and model-cache directory metadata is usable.",
    )
    .with_details(details)
}

struct PathSpec {
    label: &'static str,
    path: PathBuf,
}

impl PathSpec {
    fn observe(self) -> PathObservation {
        let (state, readonly) = match std::fs::symlink_metadata(&self.path) {
            Ok(metadata) if metadata.file_type().is_symlink() => (PathState::Symlink, None),
            Ok(metadata) if metadata.is_dir() => (
                PathState::Directory,
                Some(metadata.permissions().readonly()),
            ),
            Ok(_) => (PathState::WrongType, None),
            Err(error) if error.kind() == ErrorKind::NotFound => (PathState::Missing, None),
            Err(error) => (PathState::Unreadable(error.to_string()), None),
        };
        PathObservation {
            label: self.label,
            path: self.path,
            state,
            readonly,
        }
    }
}

#[derive(PartialEq, Eq)]
enum PathState {
    Directory,
    Missing,
    Symlink,
    WrongType,
    Unreadable(String),
}

struct PathObservation {
    label: &'static str,
    path: PathBuf,
    state: PathState,
    readonly: Option<bool>,
}

impl PathObservation {
    fn details(&self) -> serde_json::Value {
        let (state, issue) = match &self.state {
            PathState::Directory => ("directory", None),
            PathState::Missing => ("missing", None),
            PathState::Symlink => ("symlink", None),
            PathState::WrongType => ("wrong_type", None),
            PathState::Unreadable(error) => ("unreadable", Some(error.as_str())),
        };
        json!({
            "label": self.label,
            "path": self.path,
            "state": state,
            "readonly": self.readonly,
            "issue": issue,
        })
    }
}

fn external_services_check() -> Result<DoctorCheckResult> {
    let inspection = match crate::config::inspect() {
        Ok(inspection) => inspection,
        Err(error) => {
            return Ok(DoctorCheckResult::fail(
                "external_services",
                format!("Remote service config cannot be inspected: {error}"),
            )
            .with_fix("Repair or remove the invalid QoL Voice config file"));
        }
    };
    let recognition = inspection.config.recognition;
    if !recognition.enabled {
        return Ok(external_services_result(
            None,
            ExternalServiceObservation::NotConfigured("transcription is disabled"),
        ));
    }
    if recognition.provider != "websocket" {
        return Ok(external_services_result(
            None,
            ExternalServiceObservation::NotConfigured(
                "the selected provider does not use a configured remote service",
            ),
        ));
    }
    let endpoint = recognition.websocket_endpoint;
    if endpoint.trim().is_empty() {
        return Ok(external_services_result(
            Some(endpoint),
            ExternalServiceObservation::Failed(
                "the WebSocket provider requires a non-empty endpoint".to_string(),
            ),
        ));
    }
    let observation = match crate::transcribe::probe_endpoint(&endpoint) {
        Ok(()) => ExternalServiceObservation::Ready,
        Err(error) => ExternalServiceObservation::Failed(error.to_string()),
    };
    Ok(external_services_result(Some(endpoint), observation))
}

enum ExternalServiceObservation<'a> {
    NotConfigured(&'a str),
    Ready,
    Failed(String),
}

fn external_services_result(
    endpoint: Option<String>,
    observation: ExternalServiceObservation<'_>,
) -> DoctorCheckResult {
    let (state, issue, connection_attempted) = match &observation {
        ExternalServiceObservation::NotConfigured(reason) => {
            ("not_configured", Some(*reason), false)
        }
        ExternalServiceObservation::Ready => ("ready", None, true),
        ExternalServiceObservation::Failed(error) => ("failed", Some(error.as_str()), true),
    };
    let details = json!({
        "provider": endpoint.as_ref().map(|_| "websocket"),
        "endpoint": endpoint,
        "state": state,
        "issue": issue,
        "connection_attempted": connection_attempted,
        "audio_sent": false,
        "configuration_sent": false,
        "mutated": false,
    });
    match observation {
        ExternalServiceObservation::NotConfigured(reason) => DoctorCheckResult::ok(
            "external_services",
            format!("No remote transcription service probe is required: {reason}."),
        )
        .with_details(details),
        ExternalServiceObservation::Ready => DoctorCheckResult::ok(
            "external_services",
            "The configured WebSocket transcription service completed a handshake.",
        )
        .with_details(details),
        ExternalServiceObservation::Failed(error) => DoctorCheckResult::fail(
            "external_services",
            format!("The configured WebSocket transcription service is not ready: {error}"),
        )
        .with_fix("Start the configured WebSocket transcription service or correct its endpoint.")
        .with_details(details),
    }
}

fn audio_capture_check() -> Result<DoctorCheckResult> {
    if let Err(error) = verify_audio_input() {
        return Ok(DoctorCheckResult::fail("audio_capture", error.to_string())
            .with_fix("verify PipeWire or PulseAudio is running and reconnect the microphone"));
    }
    let devices = audio_input_devices()?;
    if devices.is_empty() {
        return Ok(DoctorCheckResult::fail(
            "audio_capture",
            "the audio service exposes no microphone sources",
        )
        .with_fix("connect or enable a microphone input"));
    }
    let default = devices
        .iter()
        .find(|device| device.is_default)
        .map(|device| device.label.as_str())
        .unwrap_or("not identified");
    Ok(DoctorCheckResult::ok(
        "audio_capture",
        format!(
            "{} microphone source(s) available; default: {default}",
            devices.len()
        ),
    ))
}

fn transcription_provider_check() -> Result<DoctorCheckResult> {
    let configured = crate::config::inspect()
        .map(|inspection| inspection.config.recognition.provider)
        .unwrap_or_else(|_| "auto".to_owned());
    Ok(transcription_provider_result(
        &configured,
        crate::transcribe::transcriber_descriptors()
            .map(|provider| provider.id)
            .collect(),
        crate::transcribe::resolve_descriptor(&configured),
    ))
}

fn transcription_provider_result(
    configured: &str,
    registered: Vec<&'static str>,
    resolved: Result<
        crate::transcribe::TranscriberDescriptor,
        crate::transcribe::TranscriptionError,
    >,
) -> DoctorCheckResult {
    let summary = format!("{} registered: {}", registered.len(), registered.join(", "));
    let details = json!({
        "configured_provider": configured,
        "registered": registered,
        "resolved_provider": resolved.as_ref().ok().map(|provider| provider.id),
    });
    match resolved {
        Err(error) => DoctorCheckResult::fail(
            "transcription_providers",
            format!("the configured provider '{configured}' cannot be selected: {error}"),
        )
        .with_fix(
            "build QoL Voice with the features its plugin.toml declares, or select a provider this build registers",
        )
        .with_details(details),
        Ok(provider) => DoctorCheckResult::ok(
            "transcription_providers",
            format!("provider '{configured}' resolves to {}; {summary}", provider.id),
        )
        .with_details(details),
    }
}

fn speech_output_check() -> Result<DoctorCheckResult> {
    Ok(DoctorCheckResult::warn(
        "speech_output",
        "TTS playback is outside this MVP; STT and turn coordination are active",
    ))
}

#[cfg(test)]
mod tests {
    use qol_headless::DoctorStatus;

    use super::*;

    #[test]
    fn check_ids_cover_voice_diagnostic_contract() {
        assert_eq!(
            checks().iter().map(DoctorCheck::id).collect::<Vec<_>>(),
            [
                "config_readable",
                "runtime_dirs",
                "external_services",
                "audio_capture",
                "transcription_providers",
                "speech_output",
            ]
        );
    }

    #[test]
    fn runtime_directory_inspection_never_creates_missing_paths() {
        let root = tempfile::tempdir().unwrap();
        let existing = root.path().join("existing");
        let missing = root.path().join("missing");
        std::fs::create_dir(&existing).unwrap();

        let result = runtime_dirs_result(vec![
            PathSpec {
                label: "existing",
                path: existing,
            },
            PathSpec {
                label: "missing",
                path: missing.clone(),
            },
        ]);

        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(!missing.exists());
        assert_eq!(result.details.unwrap()["created"], false);
    }

    #[test]
    fn a_configured_provider_this_build_cannot_select_fails_the_check() {
        let registered = crate::transcribe::transcriber_descriptors()
            .map(|provider| provider.id)
            .collect::<Vec<_>>();
        let cases = [
            ("nonexistent", DoctorStatus::Fail),
            ("websocket", DoctorStatus::Ok),
            (
                "auto",
                if crate::transcribe::resolve_descriptor("auto").is_ok() {
                    DoctorStatus::Ok
                } else {
                    DoctorStatus::Fail
                },
            ),
        ];
        for (configured, status) in cases {
            let result = transcription_provider_result(
                configured,
                registered.clone(),
                crate::transcribe::resolve_descriptor(configured),
            );
            assert_eq!(result.status, status, "configured: {configured}");
        }
    }

    #[test]
    fn external_service_results_never_send_transcription_content() {
        let cases = [
            (ExternalServiceObservation::Ready, DoctorStatus::Ok, true),
            (
                ExternalServiceObservation::Failed("connection refused".to_string()),
                DoctorStatus::Fail,
                true,
            ),
            (
                ExternalServiceObservation::NotConfigured("local provider"),
                DoctorStatus::Ok,
                false,
            ),
        ];

        for (observation, status, connection_attempted) in cases {
            let result =
                external_services_result(Some("ws://127.0.0.1:5001".to_string()), observation);
            let details = result.details.unwrap();

            assert_eq!(result.status, status);
            assert_eq!(details["connection_attempted"], connection_attempted);
            assert_eq!(details["audio_sent"], false);
            assert_eq!(details["configuration_sent"], false);
            assert_eq!(details["mutated"], false);
        }
    }
}
