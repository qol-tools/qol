use serde::Deserialize;

use super::{run, READ};
use crate::updates::{Package, Unit, Update};

#[derive(Debug, Default, Deserialize)]
struct Report {
    #[serde(rename = "Devices", default)]
    devices: Vec<Device>,
}

#[derive(Debug, Deserialize)]
struct Device {
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "DeviceId")]
    id: String,
    #[serde(rename = "Plugin", default)]
    plugin: String,
    #[serde(rename = "Flags", default)]
    flags: Vec<String>,
    #[serde(rename = "Releases", default)]
    releases: Vec<Release>,
}

#[derive(Debug, Deserialize)]
struct Release {
    #[serde(rename = "Summary", default)]
    summary: String,
    #[serde(rename = "Version", default)]
    version: String,
    #[serde(rename = "Size")]
    size: Option<u64>,
}

pub(super) fn read() -> Result<Vec<Update>, String> {
    if !super::found("fwupdmgr") {
        return Ok(Vec::new());
    }
    let done = run("fwupdmgr", &["get-updates", "--json"], READ)?;
    if done.stdout.trim().is_empty() {
        return match done.code {
            Some(0 | 2) => Ok(Vec::new()),
            _ => Err(done.reason()),
        };
    }
    if !matches!(done.code, Some(0 | 2)) && !done.stdout.contains("\"Devices\"") {
        return Err(done.reason());
    }
    parse(&done.stdout)
}

pub(super) fn parse(text: &str) -> Result<Vec<Update>, String> {
    let report: Report = serde_json::from_str(text).map_err(|error| {
        log::warn!("fwupd answered in a way qol cannot read: {error}");
        "Could not read the firmware update list.".to_string()
    })?;
    Ok(report.devices.into_iter().filter_map(update).collect())
}

fn update(device: Device) -> Option<Update> {
    let release = device.releases.into_iter().next()?;
    let restart = device.flags.iter().any(|flag| flag == "needs-reboot");
    let after = if restart {
        "Needs a restart."
    } else {
        "Restarts nothing."
    };
    let (label, name, why) = match device.plugin.as_str() {
        "uefi_dbx" => (
            "UEFI Secure Boot list".to_string(),
            "the Secure Boot list".to_string(),
            format!("Blocks unsafe boot software. {after}"),
        ),
        "uefi_kek" => (
            "UEFI Secure Boot keys".to_string(),
            "the Secure Boot keys".to_string(),
            format!("Lets newer boot software start. {after}"),
        ),
        _ => {
            let label = format!("{} firmware {}", device.name, release.version);
            (label.clone(), label, format!("New firmware. {after}"))
        }
    };
    Some(Update {
        id: format!("fwupd:{}", device.id),
        tag: "Firmware".into(),
        label,
        name,
        why,
        restart,
        held: None,
        unit: Unit::Firmware,
        packages: vec![Package {
            name: device.name,
            detail: release.summary,
            bytes: release.size,
            target: device.id,
        }],
        running: None,
        finish: if restart {
            "Installed. The board applies it at the restart.".into()
        } else {
            "Installed. Nothing restarts.".into()
        },
        stops_sound: false,
        caution: None,
        unattended: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = r#"{"Devices":[
        {"Name":"KEK CA","DeviceId":"b7a1","Plugin":"uefi_kek","Version":"2011","Flags":["internal","updatable","needs-reboot"],
         "Releases":[{"Name":"Secure Boot KEK","Summary":"UEFI Secure Boot Key Exchange Key","Version":"2023","Size":2797}]},
        {"Name":"UEFI dbx","DeviceId":"3623","Plugin":"uefi_dbx","Version":"20230501","Flags":["internal","needs-reboot"],
         "Releases":[{"Name":"Secure Boot dbx","Summary":"UEFI Secure Boot Forbidden Signature Database","Version":"20260402","Size":24629}]},
        {"Name":"Mouse","DeviceId":"9","Plugin":"logitech_hidpp","Flags":[],"Releases":[]}
    ]}"#;

    #[test]
    fn secure_boot_updates_read_like_the_design() {
        let updates = parse(REPORT).expect("report");
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].label, "UEFI Secure Boot keys");
        assert_eq!(updates[1].label, "UEFI Secure Boot list");
        assert_eq!(updates[1].name, "the Secure Boot list");
        assert_eq!(
            updates[1].why,
            "Blocks unsafe boot software. Needs a restart."
        );
        assert_eq!(updates[1].wait(), "1 firmware update, needs a restart");
        assert_eq!(updates[1].packages[0].name, "UEFI dbx");
        assert_eq!(updates[1].packages[0].bytes, Some(24_629));
        assert_eq!(updates[1].packages[0].target, "3623");
        assert!(parse("not json").is_err());
    }
}
