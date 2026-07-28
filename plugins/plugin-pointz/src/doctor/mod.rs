use std::path::PathBuf;

use qol_headless::{DoctorCheck, DoctorCheckResult};
use serde_json::json;

use crate::config::ServerConfig;
use crate::security::{ExistingSecretInspection, ExistingSecretState};

const CHECK_IDS: [&str; 6] = [
    "platform_supported",
    "config_readable",
    "permissions",
    "network_metadata",
    "runtime_endpoints",
    "pairing_secret",
];

pub(crate) fn checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            CHECK_IDS[0],
            "Verify the current platform is declared and has a PointZ input backend.",
            || Ok(platform_supported_result()),
        ),
        DoctorCheck::new(
            CHECK_IDS[1],
            "Read and deserialize the typed plugin config without changing it.",
            || Ok(config_readable_result()),
        ),
        DoctorCheck::new(
            CHECK_IDS[2],
            "Query input-backend authorization and display readiness without sending input events.",
            || Ok(permissions_result(crate::input::inspect_readiness())),
        ),
        DoctorCheck::new(
            CHECK_IDS[3],
            "Inspect hostname and network-interface metadata without binding, connecting, or sending.",
            || Ok(network_metadata_result()),
        ),
        DoctorCheck::new(
            CHECK_IDS[4],
            "Report expected daemon and UDP endpoints without binding or connecting.",
            || Ok(runtime_endpoints_result()),
        ),
        DoctorCheck::new(
            CHECK_IDS[5],
            "Inspect existing pairing-secret path metadata without reading, creating, or revealing it.",
            || Ok(pairing_secret_result()),
        ),
    ]
}

#[cfg(test)]
pub(crate) fn check_ids() -> &'static [&'static str] {
    &CHECK_IDS
}

fn platform_supported_result() -> DoctorCheckResult {
    let support = crate::input::platform_support();
    let details = json!({
        "platform": support.name,
        "declared": support.declared,
        "input_backend": support.input_backend,
        "inspection": "metadata_only",
        "input_initialized": false,
    });
    if support.declared && support.input_backend {
        return DoctorCheckResult::ok(
            CHECK_IDS[0],
            format!(
                "{} is declared and has a PointZ input backend",
                support.name
            ),
        )
        .with_details(details);
    }

    DoctorCheckResult::fail(
        CHECK_IDS[0],
        format!("{} is not a declared PointZ platform", support.name),
    )
    .with_fix("Run PointZ on Linux or macOS")
    .with_details(details)
}

fn config_readable_result() -> DoctorCheckResult {
    let inspection = match crate::config::inspect() {
        Ok(inspection) => inspection,
        Err(error) => {
            return DoctorCheckResult::fail(CHECK_IDS[1], error.to_string())
                .with_fix("Repair or remove the invalid PointZ config file")
                .with_details(json!({
                    "inspection": "read_only",
                    "config_changed": false,
                    "parse_markers_changed": false,
                }));
        }
    };
    let message = match inspection.source.as_ref() {
        Some(path) => format!(
            "Config at {} is readable and matches the typed contract",
            path.display()
        ),
        None => "No config file found; typed contract defaults are valid".to_string(),
    };
    DoctorCheckResult::ok(CHECK_IDS[1], message).with_details(json!({
        "source": inspection.source,
        "inspection": "read_only",
        "config_changed": false,
        "parse_markers_changed": false,
    }))
}

fn permissions_result(readiness: crate::input::InputReadiness) -> DoctorCheckResult {
    let details = json!({
        "platform": readiness.platform,
        "backend": readiness.backend,
        "authorization_granted": readiness.authorization_granted,
        "display_env_set": readiness.display_env_set,
        "issue": readiness.issue,
        "input_event_sent": false,
        "input_handler_initialized": false,
    });
    if readiness.ready {
        return DoctorCheckResult::ok(
            CHECK_IDS[2],
            format!(
                "The {} input backend is authorized and ready",
                readiness.backend
            ),
        )
        .with_details(details);
    }
    DoctorCheckResult::fail(
        CHECK_IDS[2],
        readiness
            .issue
            .as_deref()
            .unwrap_or("The PointZ input backend is not ready"),
    )
    .with_fix(match readiness.platform {
        "macos" => "Enable PointZ in System Settings > Privacy & Security > Accessibility",
        "linux" => "Run PointZ in an authorized X11 session with the XTEST extension",
        _ => "Run PointZ on Linux or macOS",
    })
    .with_details(details)
}

fn network_metadata_result() -> DoctorCheckResult {
    let metadata = crate::network::inspect_metadata();
    let details = network_details(&metadata);
    if let Some(issue) = metadata.interface_issue {
        return DoctorCheckResult::warn(
            CHECK_IDS[3],
            format!("Hostname is available, but interfaces could not be inspected: {issue}"),
        )
        .with_fix("Allow PointZ to inspect local network-interface metadata")
        .with_details(details);
    }
    if metadata.local_ipv4.is_none() {
        return DoctorCheckResult::warn(
            CHECK_IDS[3],
            "Hostname is available, but no non-loopback IPv4 interface was found",
        )
        .with_fix("Connect this host to an IPv4 network reachable by the PointZ client")
        .with_details(details);
    }

    DoctorCheckResult::ok(
        CHECK_IDS[3],
        format!(
            "Hostname and {} network interface address(es) are available",
            metadata.interfaces.len()
        ),
    )
    .with_details(details)
}

fn network_details(metadata: &crate::network::NetworkMetadata) -> serde_json::Value {
    let interfaces = metadata
        .interfaces
        .iter()
        .map(|interface| {
            json!({
                "name": interface.name,
                "address": interface.address,
                "loopback": interface.loopback,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "hostname": metadata.hostname,
        "local_ipv4": metadata.local_ipv4,
        "interfaces": interfaces,
        "interface_issue": metadata.interface_issue,
        "inspection": "metadata_only",
        "pointz_socket_opened": false,
    })
}

fn runtime_endpoints_result() -> DoctorCheckResult {
    let injected_socket = std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET).map(PathBuf::from);
    let effective_socket = injected_socket
        .clone()
        .unwrap_or_else(|| PathBuf::from(ServerConfig::DAEMON_SOCKET));
    DoctorCheckResult::ok(
        CHECK_IDS[4],
        format!(
            "Expected daemon socket and UDP ports {} and {} are defined",
            ServerConfig::DISCOVERY_PORT,
            ServerConfig::COMMAND_PORT
        ),
    )
    .with_details(runtime_endpoints_details(injected_socket, effective_socket))
}

fn runtime_endpoints_details(
    injected_socket: Option<PathBuf>,
    effective_socket: PathBuf,
) -> serde_json::Value {
    json!({
        "daemon_socket": {
            "manifest": ServerConfig::DAEMON_SOCKET,
            "injected": injected_socket,
            "effective": effective_socket,
            "environment": qol_conventions::ENV_DAEMON_SOCKET,
            "connected": false,
        },
        "udp": [
            {
                "name": "discovery",
                "address": "0.0.0.0",
                "port": ServerConfig::DISCOVERY_PORT,
                "protocol": "udp",
                "inherited_fd_present": inherited_fd_present("discovery"),
                "bound": false,
            },
            {
                "name": "command",
                "address": "0.0.0.0",
                "port": ServerConfig::COMMAND_PORT,
                "protocol": "udp",
                "inherited_fd_present": inherited_fd_present("command"),
                "bound": false,
            },
        ],
        "inspection": "process_environment",
    })
}

fn inherited_fd_present(name: &str) -> bool {
    let variable = format!(
        "{}_{}",
        qol_conventions::ENV_DAEMON_PORT_FD,
        name.to_uppercase()
    );
    std::env::var_os(variable).is_some()
}

fn pairing_secret_result() -> DoctorCheckResult {
    pairing_secret_inspection_result(crate::security::inspect_existing_secret())
}

fn pairing_secret_inspection_result(inspection: ExistingSecretInspection) -> DoctorCheckResult {
    let details = secret_details(&inspection);
    match inspection.state {
        ExistingSecretState::Missing => DoctorCheckResult::warn(
            CHECK_IDS[5],
            "No pairing secret exists; the daemon will create one when it starts",
        )
        .with_fix("Start PointZ normally to initialize its pairing secret")
        .with_details(details),
        ExistingSecretState::Present => DoctorCheckResult::ok(
            CHECK_IDS[5],
            "Existing pairing-secret regular-file metadata is present; contents were not inspected",
        )
        .with_details(details),
        ExistingSecretState::Invalid => DoctorCheckResult::fail(
            CHECK_IDS[5],
            inspection
                .issue
                .as_deref()
                .unwrap_or("Existing pairing secret is invalid"),
        )
        .with_fix("Remove the invalid pairing-secret path, then start PointZ to regenerate it")
        .with_details(details),
        ExistingSecretState::Unavailable => DoctorCheckResult::fail(
            CHECK_IDS[5],
            inspection
                .issue
                .as_deref()
                .unwrap_or("PointZ data directory is unavailable"),
        )
        .with_fix("Configure a writable local data directory before starting PointZ")
        .with_details(details),
    }
}

fn secret_details(inspection: &ExistingSecretInspection) -> serde_json::Value {
    json!({
        "path": inspection.path,
        "state": match inspection.state {
            ExistingSecretState::Missing => "missing",
            ExistingSecretState::Present => "present",
            ExistingSecretState::Invalid => "invalid",
            ExistingSecretState::Unavailable => "unavailable",
        },
        "file_type": inspection.file_type,
        "bytes": inspection.bytes,
        "readonly": inspection.readonly,
        "issue": inspection.issue,
        "inspection": "read_only",
        "content_inspected": false,
        "validity": "not_inspected",
        "created": false,
        "secret_exposed": false,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use qol_headless::DoctorStatus;

    use super::*;

    #[test]
    fn endpoint_check_declares_that_no_socket_was_opened() {
        let result = runtime_endpoints_result();
        let details = result.details.expect("endpoint details missing");

        assert_eq!(details["daemon_socket"]["connected"], false);
        assert_eq!(details["udp"][0]["bound"], false);
        assert_eq!(details["udp"][1]["bound"], false);
    }

    #[test]
    fn missing_secret_check_is_read_only_and_redacted() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("missing-secret");
        let result = pairing_secret_inspection_result(ExistingSecretInspection {
            path: Some(path.clone()),
            state: ExistingSecretState::Missing,
            file_type: "missing",
            bytes: None,
            readonly: None,
            issue: None,
        });

        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(!path.exists());
        let details = result.details.expect("secret details missing");
        assert_eq!(details["created"], false);
        assert_eq!(details["content_inspected"], false);
        assert_eq!(details["secret_exposed"], false);
    }

    #[test]
    fn secret_check_details_never_contain_file_contents() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pairing-secret");
        let private_material = "do-not-print-this-secret";
        fs::write(&path, private_material).unwrap();
        let result = pairing_secret_inspection_result(ExistingSecretInspection {
            path: Some(path.clone()),
            state: ExistingSecretState::Present,
            file_type: "regular",
            bytes: Some(private_material.len() as u64),
            readonly: Some(false),
            issue: None,
        });

        assert_eq!(fs::read_to_string(path).unwrap(), private_material);
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains(private_material));
        assert_eq!(
            result.details.expect("secret details missing")["content_inspected"],
            false
        );
    }

    #[test]
    fn input_readiness_result_never_sends_an_event() {
        let cases = [
            (
                crate::input::InputReadiness {
                    platform: "linux",
                    ready: true,
                    authorization_granted: Some(true),
                    display_env_set: Some(true),
                    backend: "x11-xtest",
                    issue: None,
                },
                DoctorStatus::Ok,
            ),
            (
                crate::input::InputReadiness {
                    platform: "macos",
                    ready: false,
                    authorization_granted: Some(false),
                    display_env_set: None,
                    backend: "coregraphics-accessibility",
                    issue: Some("Accessibility permission is not granted".to_string()),
                },
                DoctorStatus::Fail,
            ),
        ];

        for (readiness, status) in cases {
            let result = permissions_result(readiness);
            let details = result.details.unwrap();

            assert_eq!(result.status, status);
            assert_eq!(details["input_event_sent"], false);
            assert_eq!(details["input_handler_initialized"], false);
        }
    }
}
