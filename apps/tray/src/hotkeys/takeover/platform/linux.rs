use super::super::dconf::parse_string_array;
use super::super::spices::{self, SpiceConfig};
use super::super::{Compositor, HostFailure};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

const USER_HZ: f64 = 100.0;

const KNOWN_COMPOSITORS: &[&str] = &[
    "cinnamon",
    "gnome-shell",
    "kwin_x11",
    "kwin_wayland",
    "xfwm4",
    "marco",
    "muffin",
    "mutter",
];

pub(crate) fn dump(root: &str) -> Result<String, HostFailure> {
    run(&["dump", root])
}

pub(crate) fn list_schema(schema: &str) -> Result<String, HostFailure> {
    let command = format!("gsettings list-recursively {schema}");
    let output = Command::new("gsettings")
        .args(["list-recursively", schema])
        .output()
        .map_err(|error| HostFailure {
            command: command.clone(),
            detail: error.to_string(),
            tool_missing: true,
        })?;
    if !output.status.success() {
        return Err(HostFailure {
            command,
            detail: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            tool_missing: false,
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string())
}

pub(crate) fn read(full_key: &str) -> Result<String, HostFailure> {
    run(&["read", full_key])
}

pub(crate) fn write(full_key: &str, value: &str) -> Result<(), HostFailure> {
    run(&["write", full_key, value]).map(|_| ())
}

pub(crate) fn reset(full_key: &str) -> Result<(), HostFailure> {
    run(&["reset", full_key]).map(|_| ())
}

pub(crate) fn get_schema_value(schema: &str, key: &str) -> Result<String, HostFailure> {
    let command = format!("gsettings get {schema} {key}");
    let output = Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .map_err(|error| HostFailure {
            command: command.clone(),
            detail: error.to_string(),
            tool_missing: true,
        })?;
    if !output.status.success() {
        return Err(HostFailure {
            command,
            detail: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            tool_missing: false,
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string())
}

fn run(args: &[&str]) -> Result<String, HostFailure> {
    let command = format!("dconf {}", args.join(" "));
    let output = Command::new("dconf")
        .args(args)
        .output()
        .map_err(|error| HostFailure {
            command: command.clone(),
            detail: error.to_string(),
            tool_missing: true,
        })?;
    if !output.status.success() {
        return Err(HostFailure {
            command,
            detail: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            tool_missing: false,
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string())
}

#[derive(Debug, Default)]
struct EnabledSpices {
    uuids: BTreeSet<String>,
    instances: BTreeSet<(String, String)>,
}

impl EnabledSpices {
    fn from_gsettings(applets: &str, desklets: &str, extensions: &str) -> Self {
        let mut enabled = Self::default();
        for entry in parse_string_array(applets).unwrap_or_default() {
            let fields: Vec<&str> = entry.split(':').collect();
            if let Some(uuid) = fields.get(3) {
                enabled.add(uuid, fields.get(4).copied());
            }
        }
        for entry in parse_string_array(desklets).unwrap_or_default() {
            let fields: Vec<&str> = entry.split(':').collect();
            if let Some(uuid) = fields.first() {
                enabled.add(uuid, fields.get(1).copied());
            }
        }
        for uuid in parse_string_array(extensions).unwrap_or_default() {
            enabled.add(&uuid, None);
        }
        enabled
    }

    fn add(&mut self, uuid: &str, instance: Option<&str>) {
        self.uuids.insert(uuid.to_string());
        if let Some(instance) = instance {
            self.instances
                .insert((uuid.to_string(), instance.to_string()));
        }
    }

    fn uuids(&self) -> impl Iterator<Item = &str> {
        self.uuids.iter().map(String::as_str)
    }

    fn live_config_ids(&self, uuid: &str, config_ids: &[String]) -> Vec<String> {
        let numbered = |id: &str| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit());
        let any_numbered = config_ids.iter().any(|id| numbered(id));
        config_ids
            .iter()
            .filter(|id| {
                if numbered(id) {
                    self.instances.contains(&(uuid.to_string(), id.to_string()))
                } else {
                    !any_numbered && id.as_str() == uuid
                }
            })
            .cloned()
            .collect()
    }
}

pub(crate) fn spice_configs() -> Result<Vec<SpiceConfig>, HostFailure> {
    let Some(root) = spices_dir().filter(|root| root.is_dir()) else {
        return Ok(Vec::new());
    };
    let enabled = EnabledSpices::from_gsettings(
        &get_schema_value("org.cinnamon", "enabled-applets")?,
        &get_schema_value("org.cinnamon", "enabled-desklets")?,
        &get_schema_value("org.cinnamon", "enabled-extensions")?,
    );
    let mut configs = Vec::new();
    for uuid in enabled.uuids() {
        let Ok(files) = fs::read_dir(root.join(uuid)) else {
            continue;
        };
        let config_ids: Vec<String> = files
            .flatten()
            .filter_map(|file| {
                let name = file.file_name();
                name.to_str()?.strip_suffix(".json").map(String::from)
            })
            .collect();
        for config_id in enabled.live_config_ids(uuid, &config_ids) {
            let path = root.join(uuid).join(format!("{config_id}.json"));
            match fs::read_to_string(&path) {
                Ok(json) => configs.push(SpiceConfig {
                    uuid: uuid.to_string(),
                    config_id,
                    json,
                }),
                Err(error) => log::debug!("hotkey takeover: skipped {}: {error}", path.display()),
            }
        }
    }
    Ok(configs)
}

pub(crate) fn read_spice(full_key: &str) -> Result<String, HostFailure> {
    let path = spice_path(full_key)?;
    fs::read_to_string(&path).map_err(|error| HostFailure {
        command: format!("read {}", path.display()),
        detail: error.to_string(),
        tool_missing: false,
    })
}

pub(crate) fn write_spice(full_key: &str, json: &str) -> Result<(), HostFailure> {
    let path = spice_path(full_key)?;
    qol_fs::atomic_write(&path, json.as_bytes()).map_err(|error| HostFailure {
        command: format!("write {}", path.display()),
        detail: error.to_string(),
        tool_missing: false,
    })?;
    reload_spice_setting(full_key);
    Ok(())
}

fn spices_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|config| config.join("cinnamon").join("spices"))
}

fn spice_path(full_key: &str) -> Result<PathBuf, HostFailure> {
    let failure = |detail: &str| HostFailure {
        command: format!("cinnamon spice settings {full_key}"),
        detail: detail.to_string(),
        tool_missing: false,
    };
    let (uuid, config_id, _) =
        spices::split_full_key(full_key).ok_or_else(|| failure("not a spice settings key"))?;
    let root = spices_dir().ok_or_else(|| failure("no config directory"))?;
    Ok(root.join(uuid).join(format!("{config_id}.json")))
}

fn reload_spice_setting(full_key: &str) {
    let Some((uuid, config_id, key)) = spices::split_full_key(full_key) else {
        return;
    };
    let output = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.Cinnamon",
            "--object-path",
            "/org/Cinnamon",
            "--method",
            "org.Cinnamon.updateSetting",
        ])
        .args(
            [uuid, config_id, key, ""]
                .iter()
                .map(|arg| gvariant_string(arg)),
        )
        .output();
    let failure = match output {
        Ok(output) if output.status.success() => return,
        Ok(output) => String::from_utf8_lossy(&output.stderr).trim().to_string(),
        Err(error) => error.to_string(),
    };
    log::warn!("hotkey takeover: Cinnamon keeps {uuid} {key} until it restarts: {failure}");
}

fn gvariant_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

pub(crate) fn compositor() -> Option<Compositor> {
    let uptime = read_uptime_seconds(Path::new("/proc/uptime"))?;
    let now = SystemTime::now();
    fs::read_dir("/proc")
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let comm = fs::read_to_string(path.join("comm")).ok()?;
            let name = comm.trim().to_string();
            KNOWN_COMPOSITORS.contains(&name.as_str()).then_some(())?;
            let stat = fs::read_to_string(path.join("stat")).ok()?;
            let ticks = parse_start_ticks(&stat)?;
            Some(Compositor {
                name,
                started_at: started_at(now, uptime, ticks)?,
            })
        })
        .min_by_key(|found| {
            KNOWN_COMPOSITORS
                .iter()
                .position(|known| *known == found.name)
                .unwrap_or(usize::MAX)
        })
}

fn read_uptime_seconds(path: &Path) -> Option<f64> {
    parse_uptime_seconds(&fs::read_to_string(path).ok()?)
}

fn parse_uptime_seconds(raw: &str) -> Option<f64> {
    raw.split_ascii_whitespace().next()?.parse().ok()
}

fn parse_start_ticks(stat: &str) -> Option<u64> {
    let after_comm = stat.rsplit_once(')')?.1;
    after_comm.split_ascii_whitespace().nth(19)?.parse().ok()
}

fn started_at(now: SystemTime, uptime_seconds: f64, start_ticks: u64) -> Option<SystemTime> {
    let age = uptime_seconds - (start_ticks as f64 / USER_HZ);
    if !age.is_finite() || age < 0.0 {
        return None;
    }
    now.checked_sub(Duration::from_secs_f64(age))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_configs_cinnamon_loads_for_an_enabled_spice_are_live() {
        let enabled = EnabledSpices::from_gsettings(
            "['panel1:right:4:notifications@cinnamon.org:5', 'panel1:left:0:menu@cinnamon.org:1']",
            "['clock@cinnamon.org:7:100:200']",
            "['transparent-panels@germanfr']",
        );
        let ids = |list: &[&str]| list.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        let cases: [(&str, Vec<String>, Vec<String>); 5] = [
            (
                "notifications@cinnamon.org",
                ids(&["notifications@cinnamon.org"]),
                ids(&["notifications@cinnamon.org"]),
            ),
            (
                "menu@cinnamon.org",
                ids(&["1", "2", "menu@cinnamon.org"]),
                ids(&["1"]),
            ),
            ("clock@cinnamon.org", ids(&["7"]), ids(&["7"])),
            (
                "transparent-panels@germanfr",
                ids(&["transparent-panels@germanfr"]),
                ids(&["transparent-panels@germanfr"]),
            ),
            ("menu@cinnamon.org", ids(&["other"]), ids(&[])),
        ];
        for (uuid, files, want) in cases {
            assert_eq!(
                enabled.live_config_ids(uuid, &files),
                want,
                "{uuid} {files:?}"
            );
        }
        let uuids: Vec<&str> = enabled.uuids().collect();
        assert_eq!(
            uuids,
            vec![
                "clock@cinnamon.org",
                "menu@cinnamon.org",
                "notifications@cinnamon.org",
                "transparent-panels@germanfr"
            ]
        );
    }

    #[test]
    fn gvariant_strings_are_quoted_so_numeric_instance_ids_stay_strings() {
        let cases = [
            ("5", "'5'"),
            ("notifications@cinnamon.org", "'notifications@cinnamon.org'"),
            ("", "''"),
            ("it's", "'it\\'s'"),
            ("a\\b", "'a\\\\b'"),
        ];
        for (raw, want) in cases {
            assert_eq!(gvariant_string(raw), want, "raw: {raw}");
        }
    }

    #[test]
    fn start_ticks_are_read_past_a_comm_containing_spaces_and_parens() {
        let cases = [
            (
                "1 (cinnamon) S 1 1 1 0 -1 4194560 1 2 3 4 5 6 7 8 20 0 9 0 4242 x y",
                Some(4242),
            ),
            (
                "2 (weird ) name) S 1 1 1 0 -1 0 1 2 3 4 5 6 7 8 20 0 9 0 77 z",
                Some(77),
            ),
            ("3 (short) S 1 2 3", None),
            ("no parens here", None),
            ("", None),
        ];
        for (stat, want) in cases {
            assert_eq!(parse_start_ticks(stat), want, "stat: {stat}");
        }
    }

    #[test]
    fn uptime_reads_the_first_field_only() {
        let cases = [
            ("12345.67 98765.43\n", Some(12345.67)),
            ("0.00 0.00", Some(0.0)),
            ("garbage", None),
            ("", None),
        ];
        for (raw, want) in cases {
            assert_eq!(parse_uptime_seconds(raw), want, "raw: {raw}");
        }
    }

    #[test]
    fn started_at_converts_ticks_into_wall_clock_and_rejects_impossible_ages() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        assert_eq!(
            started_at(now, 500.0, 20_000),
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(9_700)),
            "500s uptime with a process started at tick 20000 (200s) is 300s old"
        );
        assert_eq!(
            started_at(now, 100.0, 50_000),
            None,
            "a process that claims to predate boot must not produce a timestamp"
        );
        assert_eq!(started_at(now, f64::NAN, 0), None);
    }
}
