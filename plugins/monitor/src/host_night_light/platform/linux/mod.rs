use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

mod controller;

use super::super::{HostNightLight, HostNightLightError, UnavailableHostNightLight};
use controller::{session_dir, Controller, Settings};

const ENABLED: &str = "night-light-enabled";
const TEMPERATURE: &str = "night-light-temperature";
const DISABLED_UNTIL_TOMORROW: &str = "@disabled-until-tomorrow";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Desktop {
    Cinnamon,
    Gnome,
    Kde,
}

impl Desktop {
    fn detect(value: &str) -> Option<Self> {
        value
            .split(':')
            .find_map(|token| match token.to_ascii_lowercase().as_str() {
                "cinnamon" | "x-cinnamon" => Some(Self::Cinnamon),
                "gnome" | "ubuntu" | "unity" => Some(Self::Gnome),
                "kde" => Some(Self::Kde),
                _ => None,
            })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Cinnamon => "cinnamon",
            Self::Gnome => "gnome",
            Self::Kde => "kwin",
        }
    }

    fn schema(self) -> &'static str {
        match self {
            Self::Cinnamon => "org.cinnamon.settings-daemon.plugins.color",
            Self::Gnome => "org.gnome.settings-daemon.plugins.color",
            Self::Kde => "",
        }
    }
}

struct Gsettings {
    desktop: Desktop,
    schedule: BTreeMap<String, String>,
}

impl Gsettings {
    fn command(&self, verb: &str, args: &[&str]) -> Result<String, HostNightLightError> {
        let output = Command::new("gsettings")
            .arg(verb)
            .arg(self.desktop.schema())
            .args(args)
            .output()
            .map_err(|error| HostNightLightError::Failed(format!("gsettings {verb}: {error}")))?;
        if !output.status.success() {
            return Err(HostNightLightError::Failed(format!(
                "gsettings {verb}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn disabled_until_tomorrow(&self, value: Option<&str>) -> Result<String, HostNightLightError> {
        let (service, path) = match self.desktop {
            Desktop::Cinnamon => (
                "org.cinnamon.SettingsDaemon.Color",
                "/org/cinnamon/SettingsDaemon/Color",
            ),
            Desktop::Gnome => (
                "org.gnome.SettingsDaemon.Color",
                "/org/gnome/SettingsDaemon/Color",
            ),
            Desktop::Kde => {
                return Err(HostNightLightError::Unsupported(
                    "KWin does not use GSettings night light".into(),
                ))
            }
        };
        let method = if value.is_some() {
            "org.freedesktop.DBus.Properties.Set"
        } else {
            "org.freedesktop.DBus.Properties.Get"
        };
        let mut command = Command::new("gdbus");
        command.args([
            "call",
            "--session",
            "--dest",
            service,
            "--object-path",
            path,
            "--method",
            method,
            service,
            "DisabledUntilTomorrow",
        ]);
        if let Some(value) = value {
            command.arg(format!("<{value}>"));
        }
        let output = command.output().map_err(|error| {
            HostNightLightError::Failed(format!("native night light service: {error}"))
        })?;
        if !output.status.success() {
            return Err(HostNightLightError::Failed(format!(
                "native night light service: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        if let Some(value) = value {
            return Ok(value.into());
        }
        match String::from_utf8_lossy(&output.stdout).trim() {
            "(<true>,)" => Ok("true".into()),
            "(<false>,)" => Ok("false".into()),
            _ => Err(HostNightLightError::Failed(
                "invalid native night light suspension state".into(),
            )),
        }
    }

    fn write_values(&self, values: &BTreeMap<String, String>) -> Result<(), HostNightLightError> {
        self.command("set", &[ENABLED, "false"])?;
        for (key, value) in values
            .iter()
            .filter(|(key, _)| key.as_str() != ENABLED && key.as_str() != DISABLED_UNTIL_TOMORROW)
        {
            self.command("set", &[key, value])?;
        }
        if let Some(disabled) = values.get(DISABLED_UNTIL_TOMORROW) {
            self.disabled_until_tomorrow(Some(disabled))?;
        }
        if let Some(enabled) = values.get(ENABLED) {
            self.command("set", &[ENABLED, enabled])?;
        }
        for (key, expected) in values {
            let actual = if key == DISABLED_UNTIL_TOMORROW {
                self.disabled_until_tomorrow(None)?
            } else {
                self.command("get", &[key])?
            };
            if actual != *expected {
                return Err(HostNightLightError::Failed(format!(
                    "night light setting {key} did not verify"
                )));
            }
        }
        Ok(())
    }
}

fn schedule_values(keys: &str) -> Option<BTreeMap<String, String>> {
    let keys: Vec<&str> = keys.lines().collect();
    if !keys.contains(&ENABLED) || !keys.contains(&TEMPERATURE) {
        return None;
    }
    let values = if keys.contains(&"night-light-schedule-mode") {
        vec![("night-light-schedule-mode", "'always'")]
    } else if [
        "night-light-schedule-automatic",
        "night-light-schedule-from",
        "night-light-schedule-to",
    ]
    .iter()
    .all(|key| keys.contains(key))
    {
        vec![
            ("night-light-schedule-automatic", "false"),
            ("night-light-schedule-from", "0.0"),
            ("night-light-schedule-to", "24.0"),
        ]
    } else {
        return None;
    };
    Some(
        values
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect(),
    )
}

impl Settings for Gsettings {
    fn native_supported(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        self.desktop.name()
    }

    fn get(&self) -> Result<bool, HostNightLightError> {
        match self.command("get", &[ENABLED])?.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(HostNightLightError::Failed(
                "night light enabled state is not a boolean".into(),
            )),
        }
    }

    fn set(&self, enabled: bool) -> Result<(), HostNightLightError> {
        self.command("set", &[ENABLED, if enabled { "true" } else { "false" }])?;
        if self.get()? != enabled {
            return Err(HostNightLightError::Failed(
                "night light enabled state did not verify".into(),
            ));
        }
        Ok(())
    }

    fn native_values(&self) -> Result<Option<BTreeMap<String, String>>, HostNightLightError> {
        let mut values = BTreeMap::new();
        for key in self
            .schedule
            .keys()
            .map(String::as_str)
            .chain([ENABLED, TEMPERATURE])
        {
            values.insert(key.into(), self.command("get", &[key])?);
        }
        values.insert(
            DISABLED_UNTIL_TOMORROW.into(),
            self.disabled_until_tomorrow(None)?,
        );
        Ok(Some(values))
    }

    fn apply_native(&self, active: bool, kelvin: u16) -> Result<(), HostNightLightError> {
        let mut values = self.schedule.clone();
        values.insert(
            TEMPERATURE.into(),
            format!("uint32 {}", kelvin.clamp(1000, 6500)),
        );
        values.insert(ENABLED.into(), active.to_string());
        values.insert(DISABLED_UNTIL_TOMORROW.into(), "false".into());
        self.write_values(&values)
    }

    fn restore_native(&self, values: &BTreeMap<String, String>) -> Result<(), HostNightLightError> {
        self.write_values(values)
    }
}

pub(crate) fn control(config_root: Option<&Path>) -> Arc<dyn HostNightLight> {
    let Some(desktop) = Desktop::detect(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default())
    else {
        return Arc::new(UnavailableHostNightLight(
            "No available native night light service for this desktop; using display gamma",
        ));
    };
    if desktop == Desktop::Kde {
        for (read, write, dbus) in [
            ("kreadconfig6", "kwriteconfig6", "qdbus6"),
            ("kreadconfig5", "kwriteconfig5", "qdbus"),
        ] {
            let settings = Kconfig { read, write, dbus };
            if settings
                .command(
                    dbus,
                    &[
                        "org.kde.KWin.NightLight",
                        "/org/kde/KWin/NightLight",
                        "org.kde.KWin.NightLight.enabled",
                    ],
                )
                .is_ok()
                && settings.get().is_ok()
            {
                return Arc::new(Controller::new(
                    settings,
                    session_dir(config_root, Some("kwin")),
                ));
            }
        }
        return Arc::new(UnavailableHostNightLight(
            "No available native night light service for this desktop; using display gamma",
        ));
    }
    let mut settings = Gsettings {
        desktop,
        schedule: BTreeMap::new(),
    };
    let Ok(keys) = settings.command("list-keys", &[]) else {
        return Arc::new(UnavailableHostNightLight(
            "No available native night light service for this desktop; using display gamma",
        ));
    };
    let Some(schedule) = schedule_values(&keys) else {
        return Arc::new(UnavailableHostNightLight(
            "No available native night light service for this desktop; using display gamma",
        ));
    };
    if settings.disabled_until_tomorrow(None).is_err() {
        return Arc::new(UnavailableHostNightLight(
            "No available native night light service for this desktop; using display gamma",
        ));
    }
    settings.schedule = schedule;
    Arc::new(Controller::new(
        settings,
        session_dir(
            config_root,
            match desktop {
                Desktop::Cinnamon => None,
                Desktop::Gnome | Desktop::Kde => Some(desktop.name()),
            },
        ),
    ))
}

struct Kconfig {
    read: &'static str,
    write: &'static str,
    dbus: &'static str,
}

impl Kconfig {
    fn command(&self, program: &str, args: &[&str]) -> Result<String, HostNightLightError> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|error| HostNightLightError::Failed(format!("{program}: {error}")))?;
        if !output.status.success() {
            return Err(HostNightLightError::Failed(format!(
                "{program}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
    }

    fn read(&self, key: &str, default: &str) -> Result<String, HostNightLightError> {
        self.command(
            self.read,
            &[
                "--file",
                "kwinrc",
                "--group",
                "NightColor",
                "--key",
                key,
                "--default",
                default,
            ],
        )
    }

    fn write_values(&self, values: &BTreeMap<String, String>) -> Result<(), HostNightLightError> {
        for (key, value) in values {
            self.command(
                self.write,
                &[
                    "--file",
                    "kwinrc",
                    "--group",
                    "NightColor",
                    "--key",
                    key,
                    value,
                ],
            )?;
            if self.read(key, "")? != *value {
                return Err(HostNightLightError::Failed(format!(
                    "KWin night light setting {key} did not verify"
                )));
            }
        }
        self.command(
            self.dbus,
            &["org.kde.KWin", "/KWin", "org.kde.KWin.reconfigure"],
        )?;
        Ok(())
    }
}

impl Settings for Kconfig {
    fn native_supported(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "kwin"
    }
    fn get(&self) -> Result<bool, HostNightLightError> {
        match self.read("Active", "false")?.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(HostNightLightError::Failed(
                "KWin night light enabled state is not a boolean".into(),
            )),
        }
    }
    fn set(&self, enabled: bool) -> Result<(), HostNightLightError> {
        self.write_values(&BTreeMap::from([("Active".into(), enabled.to_string())]))
    }
    fn native_values(&self) -> Result<Option<BTreeMap<String, String>>, HostNightLightError> {
        [
            ("Active", "false"),
            ("DayTemperature", "6500"),
            ("NightTemperature", "4500"),
        ]
        .into_iter()
        .map(|(key, default)| self.read(key, default).map(|value| (key.into(), value)))
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map(Some)
    }
    fn apply_native(&self, active: bool, kelvin: u16) -> Result<(), HostNightLightError> {
        self.write_values(&BTreeMap::from([
            ("Active".into(), active.to_string()),
            (
                "DayTemperature".into(),
                kelvin.clamp(1000, 6500).to_string(),
            ),
            (
                "NightTemperature".into(),
                kelvin.clamp(1000, 6500).to_string(),
            ),
        ]))
    }
    fn restore_native(&self, values: &BTreeMap<String, String>) -> Result<(), HostNightLightError> {
        self.write_values(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_selection_does_not_assume_cinnamon_on_every_linux_session() {
        for (value, expected) in [
            ("X-Cinnamon", Some(Desktop::Cinnamon)),
            ("ubuntu:GNOME", Some(Desktop::Gnome)),
            ("GNOME", Some(Desktop::Gnome)),
            ("KDE", Some(Desktop::Kde)),
            ("", None),
            ("sway", None),
        ] {
            assert_eq!(Desktop::detect(value), expected, "desktop: {value}");
        }
    }

    #[test]
    fn native_schedule_adapts_to_the_installed_schema() {
        let common = "night-light-enabled\nnight-light-temperature\n";
        for (keys, expected) in [
            ("night-light-schedule-mode", Some(vec![("night-light-schedule-mode", "'always'")])),
            ("night-light-schedule-automatic\nnight-light-schedule-from\nnight-light-schedule-to", Some(vec![("night-light-schedule-automatic", "false"), ("night-light-schedule-from", "0.0"), ("night-light-schedule-to", "24.0")])),
            ("night-light-schedule-automatic", None),
            ("", None),
        ] {
            let expected = expected.map(|entries| entries.into_iter().map(|(key, value)| (key.into(), value.into())).collect());
            assert_eq!(schedule_values(&format!("{common}{keys}")), expected, "keys: {keys}");
        }
    }
}
