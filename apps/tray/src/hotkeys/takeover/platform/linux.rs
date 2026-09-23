use super::super::{Compositor, HostFailure};
use std::fs;
use std::path::Path;
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
