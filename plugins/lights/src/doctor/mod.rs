use qol_headless::{DoctorCheck, DoctorCheckResult};
use serde_json::{json, Value};

use crate::config::model::PluginConfig;
use crate::platform::{SerialAccess, SerialMetadata};

const CHECK_IDS: [&str; 4] = [
    "platform_supported",
    "config_readable",
    "coordinator_candidates",
    "permissions",
];

pub(crate) fn checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            CHECK_IDS[0],
            "Verify the current platform is declared and has a Lights adapter.",
            || Ok(platform_supported_check()),
        ),
        DoctorCheck::new(
            CHECK_IDS[1],
            "Read and validate typed plugin config without loading runtime state or changing it.",
            || Ok(config_readable_check()),
        ),
        DoctorCheck::new(
            CHECK_IDS[2],
            "Enumerate serial metadata and rank coordinator candidates without opening or probing ports.",
            || Ok(coordinator_candidates_check()),
        ),
        DoctorCheck::new(
            CHECK_IDS[3],
            "Inspect effective read/write access to candidate serial paths without opening them.",
            || Ok(permissions_check()),
        ),
    ]
}

fn permissions_check() -> DoctorCheckResult {
    let inspection = match crate::config::store::inspect() {
        Ok(inspection) => inspection,
        Err(error) => {
            return DoctorCheckResult::fail(CHECK_IDS[3], error.to_string())
                .with_fix("Repair or remove the invalid Lights config file");
        }
    };
    let metadata = match crate::platform::enumerate_serial_metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            return DoctorCheckResult::warn(
                CHECK_IDS[3],
                format!("Serial access cannot be inspected: {error}"),
            )
            .with_fix("Reconnect the coordinator and verify its serial device permissions")
            .with_details(permission_details(&[], false));
        }
    };
    let paths = permission_paths(&inspection.config, &metadata);
    let observations = paths
        .iter()
        .map(|path| crate::platform::inspect_serial_access(path))
        .collect::<Vec<_>>();
    permissions_result(&observations)
}

fn permission_paths(config: &PluginConfig, metadata: &SerialMetadata) -> Vec<String> {
    if config.backend.serial_port != "auto" {
        return vec![config.backend.serial_port.clone()];
    }
    if let Some(detected) = crate::platform::detect_coordinator_port(&metadata.ports) {
        return vec![detected];
    }
    crate::platform::candidate_coordinator_ports(&metadata.ports)
}

fn permissions_result(observations: &[SerialAccess]) -> DoctorCheckResult {
    let details = permission_details(observations, true);
    if observations.is_empty() {
        return DoctorCheckResult::warn(
            CHECK_IDS[3],
            "No coordinator serial path is available for an access check",
        )
        .with_fix("Connect a compatible Zigbee coordinator or configure its serial path")
        .with_details(details);
    }
    let inaccessible = observations
        .iter()
        .filter(|observation| !observation.readable_writable)
        .collect::<Vec<_>>();
    if !inaccessible.is_empty() {
        return DoctorCheckResult::fail(
            CHECK_IDS[3],
            format!(
                "{} coordinator serial path(s) are not readable and writable",
                inaccessible.len()
            ),
        )
        .with_fix("Grant the current user read/write access to the reported serial device paths")
        .with_details(details);
    }
    DoctorCheckResult::ok(
        CHECK_IDS[3],
        format!(
            "{} coordinator serial path(s) are readable and writable",
            observations.len()
        ),
    )
    .with_details(details)
}

fn permission_details(observations: &[SerialAccess], enumeration_succeeded: bool) -> Value {
    json!({
        "enumeration_succeeded": enumeration_succeeded,
        "paths": observations.iter().map(|observation| json!({
            "path": observation.path,
            "readable_writable": observation.readable_writable,
            "issue": observation.issue,
        })).collect::<Vec<_>>(),
        "port_open_attempted": false,
        "coordinator_probe_attempted": false,
    })
}

fn platform_supported_check() -> DoctorCheckResult {
    let metadata = crate::platform::doctor_platform_metadata();
    let details = json!({
        "platform": metadata.name,
        "declared": metadata.supported,
        "serial_enumeration": metadata.serial_enumeration,
        "port_open_attempted": false,
        "coordinator_probe_attempted": false,
    });
    if metadata.supported {
        return DoctorCheckResult::ok(
            "platform_supported",
            format!(
                "{} is declared and has a Lights platform adapter",
                metadata.name
            ),
        )
        .with_details(details);
    }

    DoctorCheckResult::fail(
        "platform_supported",
        format!("{} is not declared by Lights", metadata.name),
    )
    .with_fix("Run Lights on Linux or macOS")
    .with_details(details)
}

fn config_readable_check() -> DoctorCheckResult {
    let inspection = match crate::config::store::inspect() {
        Ok(inspection) => inspection,
        Err(error) => {
            return DoctorCheckResult::fail("config_readable", error.to_string())
                .with_fix("Repair or remove the invalid Lights config file");
        }
    };
    let source = inspection
        .source
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "contract defaults".to_string());

    DoctorCheckResult::ok(
        "config_readable",
        format!("Typed Lights config is valid from {source}"),
    )
    .with_details(config_details(&inspection.config, &source))
}

fn config_details(config: &PluginConfig, source: &str) -> Value {
    json!({
        "source": source,
        "backend_kind": config.backend.kind,
        "serial_port": config.backend.serial_port,
        "serial_port_mode": serial_port_mode(&config.backend.serial_port),
        "channel": config.backend.channel,
        "network_key_state": network_key_state(&config.backend.network_key),
        "known_devices": config.devices.len(),
        "inspection": "read_only",
        "config_written": false,
        "network_key_generated": false,
    })
}

fn serial_port_mode(serial_port: &str) -> &'static str {
    if serial_port == "auto" {
        return "auto";
    }
    "configured"
}

fn network_key_state(network_key: &str) -> &'static str {
    if network_key == "auto" {
        return "auto";
    }
    "configured"
}

fn coordinator_candidates_check() -> DoctorCheckResult {
    let inspection = match crate::config::store::inspect() {
        Ok(inspection) => inspection,
        Err(error) => {
            return DoctorCheckResult::fail("coordinator_candidates", error.to_string())
                .with_fix("Repair or remove the invalid Lights config file");
        }
    };
    let metadata = match crate::platform::enumerate_serial_metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            return serial_enumeration_warning(&inspection.config, &error.to_string());
        }
    };

    coordinator_result(&inspection.config, &metadata)
}

fn serial_enumeration_warning(config: &PluginConfig, error: &str) -> DoctorCheckResult {
    DoctorCheckResult::warn(
        "coordinator_candidates",
        format!("Serial metadata enumeration is unavailable: {error}"),
    )
    .with_fix("Inspect the configured coordinator path on Linux or macOS")
    .with_details(json!({
        "configured_port": config.backend.serial_port,
        "serial_port_mode": serial_port_mode(&config.backend.serial_port),
        "enumeration": "unavailable",
        "port_open_attempted": false,
        "coordinator_probe_attempted": false,
        "network_key_generated": false,
        "state_written": false,
    }))
}

fn coordinator_result(config: &PluginConfig, metadata: &SerialMetadata) -> DoctorCheckResult {
    let candidates = crate::platform::candidate_coordinator_ports(&metadata.ports);
    let detected = crate::platform::detect_coordinator_port(&metadata.ports);
    let available = metadata
        .ports
        .iter()
        .map(crate::platform::describe_port)
        .collect::<Vec<_>>();
    let details = coordinator_details(
        config,
        metadata,
        &candidates,
        detected.as_deref(),
        available,
    );

    if config.backend.serial_port != "auto" {
        return configured_port_result(config, metadata, details);
    }
    if let Some(port) = detected {
        return DoctorCheckResult::ok(
            "coordinator_candidates",
            format!("Coordinator metadata identifies {port}"),
        )
        .with_details(details);
    }
    if !candidates.is_empty() {
        return DoctorCheckResult::warn(
            "coordinator_candidates",
            format!(
                "{} serial metadata candidate(s) require an explicit operational probe",
                candidates.len()
            ),
        )
        .with_fix("Run plugin-lights reload to perform coordinator detection")
        .with_details(details);
    }
    if metadata.ports.is_empty() {
        return DoctorCheckResult::warn(
            "coordinator_candidates",
            "No serial devices are present in platform metadata",
        )
        .with_fix("Connect a compatible Zigbee coordinator")
        .with_details(details);
    }

    DoctorCheckResult::warn(
        "coordinator_candidates",
        "Serial devices are present, but none match coordinator metadata",
    )
    .with_fix("Configure an explicit coordinator path or connect a supported Zigbee dongle")
    .with_details(details)
}

fn configured_port_result(
    config: &PluginConfig,
    metadata: &SerialMetadata,
    details: Value,
) -> DoctorCheckResult {
    let configured = &config.backend.serial_port;
    if metadata
        .ports
        .iter()
        .any(|port| port.port_name == *configured)
    {
        return DoctorCheckResult::ok(
            "coordinator_candidates",
            format!("Configured coordinator {configured} is present in serial metadata"),
        )
        .with_details(details);
    }

    DoctorCheckResult::warn(
        "coordinator_candidates",
        format!("Configured coordinator {configured} is absent from serial metadata"),
    )
    .with_fix("Reconnect the coordinator or select an available serial path")
    .with_details(details)
}

fn coordinator_details(
    config: &PluginConfig,
    metadata: &SerialMetadata,
    candidates: &[String],
    detected: Option<&str>,
    available: Vec<String>,
) -> Value {
    json!({
        "configured_port": config.backend.serial_port,
        "serial_port_mode": serial_port_mode(&config.backend.serial_port),
        "metadata_source": metadata.source,
        "available_ports": available,
        "candidate_ports": candidates,
        "metadata_selected_port": detected,
        "port_open_attempted": false,
        "coordinator_probe_attempted": false,
        "network_key_generated": false,
        "state_written": false,
    })
}

#[cfg(test)]
mod tests {
    use qol_headless::DoctorStatus;
    use serialport::{SerialPortInfo, SerialPortType};

    use super::*;

    #[test]
    fn check_ids_are_stable() {
        assert_eq!(
            checks().iter().map(DoctorCheck::id).collect::<Vec<_>>(),
            CHECK_IDS
        );
    }

    #[test]
    fn configured_port_is_checked_from_inventory_without_a_probe() {
        let mut config = PluginConfig::default();
        config.backend.serial_port = "/dev/tty.test".to_string();
        let metadata = SerialMetadata {
            source: "test_metadata",
            ports: vec![unknown_port("/dev/tty.test")],
        };

        let result = coordinator_result(&config, &metadata);

        assert_eq!(result.status, DoctorStatus::Ok);
        assert_eq!(
            result.details.unwrap()["coordinator_probe_attempted"],
            false
        );
    }

    #[test]
    fn empty_auto_inventory_is_a_read_only_warning() {
        let config = PluginConfig::default();
        let metadata = SerialMetadata {
            source: "test_metadata",
            ports: Vec::new(),
        };

        let result = coordinator_result(&config, &metadata);
        let details = result.details.unwrap();

        assert_eq!(result.status, DoctorStatus::Warn);
        assert_eq!(details["port_open_attempted"], false);
        assert_eq!(details["coordinator_probe_attempted"], false);
        assert_eq!(details["network_key_generated"], false);
        assert_eq!(details["state_written"], false);
    }

    #[test]
    fn config_details_never_expose_network_key_material() {
        let mut config = PluginConfig::default();
        config.backend.network_key = "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF".to_string();

        let details = config_details(&config, "test");
        let encoded = serde_json::to_string(&details).unwrap();

        assert_eq!(details["network_key_state"], "configured");
        assert!(!encoded.contains("00:11:22"));
    }

    #[test]
    fn permission_results_are_semantic_and_never_claim_to_open_ports() {
        let cases = [
            (
                vec![SerialAccess {
                    path: "/dev/tty.ok".to_string(),
                    readable_writable: true,
                    issue: None,
                }],
                DoctorStatus::Ok,
            ),
            (
                vec![SerialAccess {
                    path: "/dev/tty.denied".to_string(),
                    readable_writable: false,
                    issue: Some("permission denied".to_string()),
                }],
                DoctorStatus::Fail,
            ),
            (Vec::new(), DoctorStatus::Warn),
        ];

        for (observations, status) in cases {
            let result = permissions_result(&observations);
            let details = result.details.unwrap();

            assert_eq!(result.status, status);
            assert_eq!(details["port_open_attempted"], false);
            assert_eq!(details["coordinator_probe_attempted"], false);
        }
    }

    fn unknown_port(name: &str) -> SerialPortInfo {
        SerialPortInfo {
            port_name: name.to_string(),
            port_type: SerialPortType::Unknown,
        }
    }
}
