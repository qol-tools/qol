use std::path::Path;
use std::process;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use qol_terminal_sessions::cli::CliLaunchProgram;

const SCOPE_SLICE: &str = "qol-agents.slice";
const SCOPE_WEIGHT_MIN: u32 = 1;
const SCOPE_WEIGHT_MAX: u32 = 10_000;
const SPAWN_CAP_DEFAULT_CPU_WEIGHT: u32 = 40;
const SPAWN_CAP_DEFAULT_IO_WEIGHT: u32 = 40;
const SCOPE_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const SCOPE_PROBE_POLL: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnCapConfig {
    pub enabled: bool,
    pub cpu_weight: u32,
    pub io_weight: u32,
    pub cpu_quota: Option<String>,
}

impl Default for SpawnCapConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cpu_weight: SPAWN_CAP_DEFAULT_CPU_WEIGHT,
            io_weight: SPAWN_CAP_DEFAULT_IO_WEIGHT,
            cpu_quota: None,
        }
    }
}

pub fn config_spawn_cap() -> Result<SpawnCapConfig> {
    let Some(path) = crate::config::primary_config_path() else {
        return Ok(SpawnCapConfig::default());
    };
    config_spawn_cap_at(&path)
}

pub fn config_spawn_cap_at(path: &Path) -> Result<SpawnCapConfig> {
    let mut cap = SpawnCapConfig::default();
    let Some(config) = crate::config::read(path, "spawn cap config")? else {
        return Ok(cap);
    };
    if let Some(enabled) = config.spawn_cap {
        cap.enabled = enabled;
    }
    if let Some(weight) = config.spawn_cpu_weight {
        validate_scope_weight(weight, "spawn_cpu_weight", path)?;
        cap.cpu_weight = weight;
    }
    if let Some(weight) = config.spawn_io_weight {
        validate_scope_weight(weight, "spawn_io_weight", path)?;
        cap.io_weight = weight;
    }
    if let Some(quota) = config.spawn_cpu_quota {
        if quota.trim().is_empty() {
            bail!(
                "spawn_cpu_quota must be a non-empty value such as `600%` in {}",
                path.display()
            );
        }
        cap.cpu_quota = Some(quota);
    }
    Ok(cap)
}

fn validate_scope_weight(weight: u32, key: &str, path: &Path) -> Result<()> {
    if (SCOPE_WEIGHT_MIN..=SCOPE_WEIGHT_MAX).contains(&weight) {
        return Ok(());
    }
    bail!(
        "{key} must be between {SCOPE_WEIGHT_MIN} and {SCOPE_WEIGHT_MAX} in {}",
        path.display()
    )
}

pub fn wrap_launch(launch: &CliLaunchProgram, cap: Option<&SpawnCapConfig>) -> CliLaunchProgram {
    let Some(cap) = cap else {
        return launch.clone();
    };
    if !cap.enabled {
        return launch.clone();
    }
    let mut args = scope_property_args(cap, true);
    args.push("--".to_owned());
    args.push(launch.program.clone());
    args.extend(launch.args.iter().cloned());
    CliLaunchProgram {
        program: "systemd-run".to_owned(),
        args,
        env: launch.env.clone(),
    }
}

fn scope_property_args(cap: &SpawnCapConfig, with_quota: bool) -> Vec<String> {
    let mut args = vec![
        "--user".to_owned(),
        "--scope".to_owned(),
        "--quiet".to_owned(),
        format!("--slice={SCOPE_SLICE}"),
        "-p".to_owned(),
        format!("CPUWeight={}", cap.cpu_weight),
        "-p".to_owned(),
        format!("IOWeight={}", cap.io_weight),
    ];
    if with_quota {
        if let Some(quota) = &cap.cpu_quota {
            args.push("-p".to_owned());
            args.push(format!("CPUQuota={quota}"));
        }
    }
    args
}

pub fn resolve_spawn_cap(config: SpawnCapConfig) -> Option<SpawnCapConfig> {
    if !config.enabled {
        qol_runtime::probe!("CLI_SESSION_SPAWN", "event=cap_disabled reason=config");
        return None;
    }
    if probe_scope(&scope_property_args(&config, true)) {
        return Some(config);
    }
    if config.cpu_quota.is_some() && probe_scope(&scope_property_args(&config, false)) {
        let mut weight_only = config.clone();
        weight_only.cpu_quota = None;
        qol_runtime::probe!(
            "CLI_SESSION_SPAWN",
            "event=cap_quota_dropped reason=systemd_rejected_cpu_quota"
        );
        return Some(weight_only);
    }
    qol_runtime::probe!(
        "CLI_SESSION_SPAWN",
        "event=cap_disabled reason=systemd_scope_unavailable"
    );
    None
}

fn probe_scope(args: &[String]) -> bool {
    let mut command = process::Command::new("systemd-run");
    command
        .args(args)
        .arg("--")
        .arg("true")
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => return false,
    };
    let deadline = Instant::now() + SCOPE_PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => std::thread::sleep(SCOPE_PROBE_POLL),
            Err(_) => return false,
        }
    }
}

pub fn apply_slice_properties(cap: Option<&SpawnCapConfig>) {
    let Some(cap) = cap else {
        return;
    };
    for slice in ["qol.slice", "qol-agents.slice"] {
        let mut command = process::Command::new("systemctl");
        let quota = cap.cpu_quota.as_deref().unwrap_or("");
        command
            .arg("--user")
            .arg("set-property")
            .arg(slice)
            .arg(format!("CPUWeight={}", cap.cpu_weight))
            .arg(format!("IOWeight={}", cap.io_weight))
            .arg(format!("CPUQuota={quota}"));
        let _ = command
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::null())
            .status();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn wrap_launch_runs_inside_a_systemd_scope_when_capping_is_resolved() {
        let launch = CliLaunchProgram {
            program: "pi".to_owned(),
            args: vec!["--model".to_owned(), "flash-x".to_owned()],
            env: Vec::new(),
        };

        let unwrapped = wrap_launch(&launch, None);
        assert_eq!(unwrapped.program, "pi");
        assert_eq!(
            unwrapped.args,
            vec!["--model".to_owned(), "flash-x".to_owned()]
        );

        let wrapped = wrap_launch(&launch, Some(&SpawnCapConfig::default()));
        assert_eq!(wrapped.program, "systemd-run");
        assert_eq!(
            wrapped.args,
            vec![
                "--user".to_owned(),
                "--scope".to_owned(),
                "--quiet".to_owned(),
                "--slice=qol-agents.slice".to_owned(),
                "-p".to_owned(),
                "CPUWeight=40".to_owned(),
                "-p".to_owned(),
                "IOWeight=40".to_owned(),
                "--".to_owned(),
                "pi".to_owned(),
                "--model".to_owned(),
                "flash-x".to_owned(),
            ]
        );
    }

    #[test]
    fn wrap_launch_adds_the_quota_property_only_when_configured() {
        let launch = CliLaunchProgram {
            program: "codex".to_owned(),
            args: Vec::new(),
            env: Vec::new(),
        };
        let cap = SpawnCapConfig {
            enabled: true,
            cpu_weight: 25,
            io_weight: 20,
            cpu_quota: Some("600%".to_owned()),
        };
        let wrapped = wrap_launch(&launch, Some(&cap));
        assert_eq!(wrapped.program, "systemd-run");
        assert_eq!(
            wrapped.args,
            vec![
                "--user".to_owned(),
                "--scope".to_owned(),
                "--quiet".to_owned(),
                "--slice=qol-agents.slice".to_owned(),
                "-p".to_owned(),
                "CPUWeight=25".to_owned(),
                "-p".to_owned(),
                "IOWeight=20".to_owned(),
                "-p".to_owned(),
                "CPUQuota=600%".to_owned(),
                "--".to_owned(),
                "codex".to_owned(),
            ]
        );

        let disabled = SpawnCapConfig {
            cpu_quota: Some("600%".to_owned()),
            ..cap
        };
        let wrapped = wrap_launch(
            &launch,
            Some(&SpawnCapConfig {
                enabled: false,
                ..disabled
            }),
        );
        assert_eq!(wrapped.program, "codex");
        assert!(wrapped.args.is_empty());
    }

    #[test]
    fn spawn_cap_config_parses_keys_and_defaults_to_weight_based_capping() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("sessions.toml");
        assert_eq!(
            config_spawn_cap_at(&path).unwrap(),
            SpawnCapConfig::default()
        );

        fs::write(
            &path,
            "spawn_cpu_weight = 25\nspawn_io_weight = 20\nspawn_cpu_quota = \"600%\"\n",
        )
        .unwrap();
        assert_eq!(
            config_spawn_cap_at(&path).unwrap(),
            SpawnCapConfig {
                enabled: true,
                cpu_weight: 25,
                io_weight: 20,
                cpu_quota: Some("600%".to_owned()),
            }
        );

        fs::write(&path, "spawn_cap = false\nspawn_cpu_quota = \"300%\"\n").unwrap();
        assert_eq!(
            config_spawn_cap_at(&path).unwrap(),
            SpawnCapConfig {
                enabled: false,
                cpu_quota: Some("300%".to_owned()),
                ..SpawnCapConfig::default()
            }
        );

        fs::write(&path, "spawn_cpu_weight = 0\n").unwrap();
        let error = config_spawn_cap_at(&path).unwrap_err().to_string();
        assert!(error.contains("spawn_cpu_weight"), "{error}");
        assert!(error.contains("10000"), "{error}");

        fs::write(&path, "spawn_io_weight = 10001\n").unwrap();
        let error = config_spawn_cap_at(&path).unwrap_err().to_string();
        assert!(error.contains("spawn_io_weight"), "{error}");

        fs::write(&path, "spawn_cpu_quota = \"  \"\n").unwrap();
        let error = config_spawn_cap_at(&path).unwrap_err().to_string();
        assert!(error.contains("spawn_cpu_quota"), "{error}");
    }
}
