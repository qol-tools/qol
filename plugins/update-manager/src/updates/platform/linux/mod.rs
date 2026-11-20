use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use chrono::{Local, NaiveDateTime};

use crate::platform::linux::command::{found, lenient, run};
use crate::platform::linux::privileged::refused;
use crate::platform::Refusal;
use crate::updates::{AutoKey, Failure, Host, Hours, Reason, Update, Updates};

mod apt;
mod boot;
mod flatpak;
mod fwupd;
mod history;
mod mint;
mod names;
mod privileged;
mod skips;

pub(crate) use crate::platform::linux::is_mint;
use boot::Running;
pub(crate) use privileged::Helper;

const INSTALL: Duration = Duration::from_secs(3600);
const HISTORY: &str = "/var/log/apt/history.log";
const HISTORY_ROTATED: &str = "/var/log/apt/history.log.1.gz";
const ADMIN_GROUPS: [&str; 3] = ["sudo", "admin", "wheel"];
const RUNS_SHOWN: usize = 2;

pub(crate) fn read_updates() -> Updates {
    let running = boot::running();
    let tag = if is_mint() {
        "Update Manager"
    } else {
        "System"
    };
    let mut updates = Updates::default();
    for (source, read) in [
        ("APT", apt::read(&running, tag)),
        ("Flatpak", flatpak::read()),
        ("Firmware", fwupd::read()),
    ] {
        match read {
            Ok(list) => updates.list.extend(list),
            Err(problem) => updates
                .problems
                .push(format!("{source} could not list updates: {problem}")),
        }
    }
    updates
}

pub(crate) fn read_host() -> Host {
    let now = Local::now().naive_local();
    let running = boot::running();
    let mut text = fs::read_to_string(HISTORY).unwrap_or_default();
    if !text.contains("Start-Date") {
        text = lenient("zcat", &["-f", HISTORY_ROTATED]);
    }
    let bluetooth = lenient(
        "systemctl",
        &[
            "show",
            "bluetooth.service",
            "-p",
            "ActiveEnterTimestamp",
            "--value",
        ],
    );
    let groups = lenient("id", &["-Gn"]);
    Host {
        finish: boot::finish(&running, now),
        runs: history::parse(
            &text,
            boot::boot_time(now),
            mint::parse_stamp(bluetooth.trim()),
            RUNS_SHOWN,
        ),
        mint: mint::read(),
        switches: crate::cleanup::read_switches(),
        admin: groups
            .split_whitespace()
            .any(|group| ADMIN_GROUPS.contains(&group)),
        restart_at: mint::read_restart(),
        log: Path::new(HISTORY).exists().then(|| HISTORY.to_string()),
        skips: skips::read(),
        problems: Vec::new(),
    }
}

pub(crate) fn install(
    updates: &[Update],
    log: &Path,
    progress: &mut dyn FnMut(usize),
) -> Result<(), Failure> {
    let stopped = |done: usize, reason: String| Failure {
        done,
        reason: Reason::Stopped(reason),
    };
    if let Some(dir) = log.parent() {
        fs::create_dir_all(dir).map_err(|error| {
            stopped(0, format!("the install log could not be written: {error}"))
        })?;
    }
    let mut file = File::create(log)
        .map_err(|error| stopped(0, format!("the install log could not be written: {error}")))?;
    let by = |prefix: &'static str| {
        updates
            .iter()
            .filter(move |update| update.id.starts_with(prefix))
            .flat_map(|update| update.packages.iter())
    };
    let mut done = 0;
    let apt: Vec<String> = by("apt:")
        .filter(|package| !package.target.is_empty())
        .map(|package| package.target.clone())
        .collect();
    if apt.is_empty() {
        progress(0);
    }
    if !apt.is_empty() {
        let mut report = |count: usize| progress(count);
        privileged::install_apt(&apt, &mut file, &mut report)?;
        done = by("apt:").count();
        progress(done);
    }
    for (program, args) in by("flatpak:")
        .map(|package| {
            (
                "flatpak",
                vec!["update", "-y", "--noninteractive", package.target.as_str()],
            )
        })
        .chain(by("fwupd:").map(|package| {
            (
                "fwupdmgr",
                vec![
                    "update",
                    "--assume-yes",
                    "--no-reboot-check",
                    package.target.as_str(),
                ],
            )
        }))
    {
        let _ = writeln!(file, "$ {program} {}", args.join(" "));
        let finished = run(program, &args, INSTALL).map_err(|reason| stopped(done, reason))?;
        let _ = write!(file, "{}{}", finished.stdout, finished.stderr);
        if finished.code != Some(0) {
            let reason = if refused(&finished.stderr) {
                Reason::Refused(Refusal::Denied)
            } else {
                Reason::Stopped(finished.reason())
            };
            return Err(Failure { done, reason });
        }
        done += 1;
        progress(done);
    }
    Ok(())
}

pub(crate) fn set_auto(key: AutoKey, on: bool) -> Result<(), Refusal> {
    if let AutoKey::Switch(switch) = key {
        return crate::cleanup::set_switch(switch, on);
    }
    mint::set_auto(key, on)
}

pub(crate) fn set_hours(
    hours: Option<Hours>,
    startup_delay: u32,
    restart: Option<NaiveDateTime>,
) -> Result<(), Refusal> {
    mint::set_hours(hours, startup_delay, restart)
}

pub(crate) fn set_skips(add: &[String], remove: &[String], system: bool) -> Result<(), Refusal> {
    skips::set(add, remove, system && is_mint())
}

pub(crate) fn schedule_restart(at: Option<NaiveDateTime>) -> Result<(), Refusal> {
    mint::schedule_restart(at)
}

pub(crate) fn restart_now() -> Result<(), Refusal> {
    mint::restart_now()
}

pub(crate) fn boot_id() -> Option<String> {
    boot::boot_id()
}

pub(crate) fn supported() -> Result<(), &'static str> {
    Ok(())
}

pub(crate) fn missing(programs: &[&'static str]) -> Vec<&'static str> {
    programs
        .iter()
        .copied()
        .filter(|program| !found(program))
        .collect()
}
