use std::path::PathBuf;

use qol_headless::{DoctorCheck, DoctorCheckResult};
use serde_json::json;

use qol_peers::admin::PointzStatus;
use qol_peers::pointz::PointzTransport;

use crate::config::ServerConfig;

const CHECK_IDS: [&str; 6] = [
    "platform_supported",
    "config_readable",
    "permissions",
    "network_metadata",
    "runtime_endpoints",
    "phone_pairing",
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
            "Report the daemon socket and the UDP ports qol-tray owns, without binding or connecting.",
            || Ok(runtime_endpoints_result()),
        ),
        DoctorCheck::new(
            CHECK_IDS[5],
            "Ask qol-tray for phone pairing status without changing it or revealing a pairing code.",
            || Ok(phone_pairing_result(crate::app::pairing::status())),
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
            "Daemon socket is defined; qol-tray owns UDP ports {} and {}",
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
                "port": ServerConfig::DISCOVERY_PORT,
                "owner": "qol-tray",
                "bound": false,
            },
            {
                "name": "command",
                "port": ServerConfig::COMMAND_PORT,
                "owner": "qol-tray",
                "bound": false,
            },
        ],
        "inspection": "process_environment",
    })
}

fn phone_pairing_result(status: Option<PointzStatus>) -> DoctorCheckResult {
    let Some(status) = status else {
        return DoctorCheckResult::warn(
            CHECK_IDS[5],
            "qol-tray did not answer, so phone pairing status is unavailable",
        )
        .with_fix("Start qol-tray, then run the PointZ doctor again")
        .with_details(json!({ "inspection": "core_status", "reachable": false }));
    };
    let details = json!({
        "inspection": "core_status",
        "reachable": true,
        "plugin": status.plugin,
        "migration": status.migration,
        "device_count": status.device_count,
        "transport": status.transport,
        "pairing_open": status.pairing.open,
    });
    let must_pair_again = status
        .migration
        .is_some_and(|migration| migration.phones_must_pair_again());
    let result = match status.transport {
        PointzTransport::PortBusy { .. } => {
            DoctorCheckResult::fail(CHECK_IDS[5], "Another program holds a PointZ network port")
                .with_fix(format!(
                    "Quit the program using UDP port {} or {}",
                    ServerConfig::DISCOVERY_PORT,
                    ServerConfig::COMMAND_PORT
                ))
        }
        PointzTransport::Failed { .. } => DoctorCheckResult::fail(
            CHECK_IDS[5],
            "qol-tray could not open the PointZ network ports",
        )
        .with_fix("Restart qol-tray and check its log for the PointZ adapter"),
        _ if must_pair_again => DoctorCheckResult::warn(
            CHECK_IDS[5],
            "Some phones must pair again after pairing moved into qol-tray",
        )
        .with_fix("Open PointZ settings and pair each phone again"),
        _ if status.migration.is_none() => {
            DoctorCheckResult::ok(CHECK_IDS[5], "No phone has been paired yet")
        }
        PointzTransport::Stopped {} => {
            DoctorCheckResult::warn(CHECK_IDS[5], "qol-tray is not listening for phones")
                .with_fix("Open PointZ settings and pair a phone")
        }
        PointzTransport::Running { .. } => DoctorCheckResult::ok(
            CHECK_IDS[5],
            format!(
                "qol-tray is listening for {} paired phone(s)",
                status.device_count
            ),
        ),
    };
    result.with_details(details)
}

#[cfg(test)]
mod tests {
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

    fn status(transport: PointzTransport) -> PointzStatus {
        PointzStatus {
            plugin: qol_peers::pointz::PointzPlugin::Compatible,
            authority: None,
            migration: Some(qol_peers::pointz::PointzImport::FRESH),
            server_id: Some("server".into()),
            device_count: 1,
            pairing: qol_peers::pointz::PointzPairing {
                open: true,
                code: Some("482913".into()),
                seconds_remaining: 40,
            },
            transport,
        }
    }

    #[test]
    fn pairing_check_reports_the_core_state_without_the_code() {
        let running = phone_pairing_result(Some(status(PointzTransport::Running { dropped: 0 })));
        let busy = phone_pairing_result(Some(status(PointzTransport::PortBusy {
            socket: qol_peers::pointz::PointzSocket::Command,
        })));
        let unreachable = phone_pairing_result(None);

        assert_eq!(running.status, DoctorStatus::Ok);
        assert_eq!(busy.status, DoctorStatus::Fail);
        assert_eq!(unreachable.status, DoctorStatus::Warn);
        assert!(!serde_json::to_string(&running).unwrap().contains("482913"));
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
