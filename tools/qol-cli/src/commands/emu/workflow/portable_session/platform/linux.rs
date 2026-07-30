//! Portable-session product contract: launch from removable media, prove
//! usability fast, then prove the guest is left exactly as found.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use qol_conventions::local_base_url;
use qol_dev_guest::{GuestControlClient, ProcessState, RequestAction, ResponseResult};
use serde::Serialize;

use crate::commands::emu::{qmp, BootedVm};
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_desktop_guest, dispatch_plugin_action, exec, require_exec,
    stop_preinstalled_runtime, wait_for_command, wait_for_window_title, xdotool_key,
    HTTP_TOKEN_PATH,
};
use super::Verdict;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(20);
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const MOUNT_TIMEOUT: Duration = Duration::from_secs(30);
const AUTOMOUNT_WAIT: Duration = Duration::from_secs(10);
const EXIT_SETTLE: Duration = Duration::from_secs(10);
const SINGLE_DIGIT_SECONDS: Duration = Duration::from_secs(10);
const SNAPSHOT_DIR: &str = "/var/tmp/qol-portable-session";
const MAX_DELTA_LINES: usize = 400;
const QOL_PROCESS_PATTERN: &str = "qol-tray|/home/qol/\\.config/qol-tray/plugins/|^/media/qol/";
const HOME_EXCLUDES: &[&str] = &[
    "/home/qol/.cache",
    "/home/qol/.dbus",
    "/home/qol/.Xauthority",
    "/home/qol/.xsession-errors",
    "/home/qol/.config/pulse",
    "/home/qol/.config/dconf",
    "/home/qol/.local/state",
    "/home/qol/.local/share/gvfs-metadata",
    "/home/qol/.local/share/recently-used.xbel",
];

#[derive(Serialize)]
struct AssertionOutcome {
    id: &'static str,
    pass: bool,
    detail: String,
}

#[derive(Default, Serialize)]
struct Timings {
    insert_to_mount_ms: Option<u64>,
    insert_to_api_ms: Option<u64>,
    insert_to_usable_ms: Option<u64>,
    relaunch_to_ready_ms: Vec<u64>,
}

struct TrayHandle {
    process_id: u64,
    guest_pid: u32,
}

struct Session<'a> {
    vm: &'a BootedVm,
    guest: Option<GuestControlClient>,
    qmp: qmp::QmpClient,
    artifacts_dir: PathBuf,
    assertions: Vec<AssertionOutcome>,
    notes: Vec<String>,
    timings: Timings,
    post_exit_home: Vec<String>,
    post_exit_dconf: Vec<String>,
    post_pull_home: Vec<String>,
    post_pull_dconf: Vec<String>,
    post_reboot_home: Vec<String>,
    post_reboot_dconf: Vec<String>,
    artifacts: Vec<PathBuf>,
}

pub(super) fn run(vm: &BootedVm) -> Result<Verdict> {
    let artifacts_dir = vm.run_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;
    let guest = connect_desktop_guest(vm)?;
    let qmp = qmp::connect_verified(vm.qmp_port, COMMAND_TIMEOUT, &vm.run_id)?;
    let mut session = Session {
        vm,
        guest: Some(guest),
        qmp,
        artifacts_dir,
        assertions: Vec::new(),
        notes: Vec::new(),
        timings: Timings::default(),
        post_exit_home: Vec::new(),
        post_exit_dconf: Vec::new(),
        post_pull_home: Vec::new(),
        post_pull_dconf: Vec::new(),
        post_reboot_home: Vec::new(),
        post_reboot_dconf: Vec::new(),
        artifacts: Vec::new(),
    };
    let completed = match session.execute() {
        Ok(()) => true,
        Err(error) => {
            session.notes.push(format!("contract aborted: {error:#}"));
            false
        }
    };
    let report_path = session.write_report()?;
    Ok(session.into_verdict(report_path, completed))
}

impl Session<'_> {
    fn execute(&mut self) -> Result<()> {
        stop_preinstalled_runtime(self.guest()?)?;
        self.baseline()?;
        let (mount, device, inserted_at) = self.insert_and_mount()?;
        let handle = self.first_session(&mount, inserted_at)?;
        self.graceful_exit_phase(&handle)?;
        self.crash_phase(&mount)?;
        self.live_pull_phase(&mount, &device)?;
        self.reboot_phase()
    }

    fn guest(&mut self) -> Result<&mut GuestControlClient> {
        self.guest.as_mut().context("guest control disconnected")
    }

    fn baseline(&mut self) -> Result<()> {
        let processes = self.qol_processes()?;
        self.assert_check(
            "baseline-quiesced",
            processes.is_empty(),
            summarize("qol processes before insert", &processes),
        );
        self.snapshot("baseline")
    }

    fn insert_and_mount(&mut self) -> Result<(String, String, Instant)> {
        let image = self
            .vm
            .launch
            .payload_image
            .clone()
            .context("portable-session requires the payload image as its portable medium")?;
        let inserted_at = Instant::now();
        self.qmp.attach_usb_medium(&image, true)?;
        step_label(
            "insert",
            StepKind::Success,
            "portable medium attached over usb",
        );
        let disk = self.wait_for_usb_disk()?;
        let device = format!("/dev/{disk}");
        let mount = self.mount_medium(&device)?;
        self.timings.insert_to_mount_ms = Some(ms(inserted_at.elapsed()));
        step_label("mount", StepKind::Success, &mount);
        Ok((mount, device, inserted_at))
    }

    fn wait_for_usb_disk(&mut self) -> Result<String> {
        let outcome = wait_for_command(
            self.guest()?,
            command("/usr/bin/lsblk", &["-nro", "NAME,TRAN,TYPE"]),
            MOUNT_TIMEOUT,
            |outcome| usb_disk_name(&outcome.stdout).is_some(),
            "the portable medium block device",
        )?;
        usb_disk_name(&outcome.stdout).context("usb disk disappeared after discovery")
    }

    fn mount_medium(&mut self, device: &str) -> Result<String> {
        if let Some(mount) = self.wait_for_automount(device, AUTOMOUNT_WAIT)? {
            return Ok(mount);
        }
        let outcome = exec(
            self.guest()?,
            command(
                "/usr/bin/udisksctl",
                &["mount", "-b", device, "--no-user-interaction"],
            ),
            COMMAND_TIMEOUT,
        )?;
        if let Some(mount) = parse_mount_point(&outcome.stdout) {
            return Ok(mount);
        }
        self.wait_for_automount(device, AUTOMOUNT_WAIT)?
            .with_context(|| format!("no mount point for {device}: {}", outcome.stderr.trim()))
    }

    fn wait_for_automount(&mut self, device: &str, budget: Duration) -> Result<Option<String>> {
        let deadline = Instant::now() + budget;
        loop {
            let outcome = exec(
                self.guest()?,
                command("/usr/bin/findmnt", &["-nro", "TARGET", "-S", device]),
                COMMAND_TIMEOUT,
            )?;
            let target = outcome.stdout.lines().next().unwrap_or("").trim();
            if outcome.exit_code == Some(0) && !target.is_empty() {
                return Ok(Some(target.to_string()));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(200));
        }
    }

    fn first_session(&mut self, mount: &str, inserted_at: Instant) -> Result<TrayHandle> {
        let tray = format!("{mount}/bin/qol-tray");
        let (auth, _, handle) = self.launch_and_ready(&tray)?;
        self.timings.insert_to_api_ms = Some(ms(inserted_at.elapsed()));
        match self.launcher_roundtrip(&auth, inserted_at) {
            Ok(usable) => {
                self.timings.insert_to_usable_ms = Some(ms(usable));
                self.assert_check(
                    "usable-after-ready",
                    true,
                    "launcher opened from a real tray action and dismissed".to_string(),
                );
                self.assert_check(
                    "ready-in-single-digit-seconds",
                    usable < SINGLE_DIGIT_SECONDS,
                    format!("insert to launcher window took {} ms", ms(usable)),
                );
            }
            Err(error) => {
                self.assert_check(
                    "usable-after-ready",
                    false,
                    format!("launcher roundtrip failed: {error:#}"),
                );
                self.assert_check(
                    "ready-in-single-digit-seconds",
                    false,
                    "launcher window never appeared, timing unavailable".to_string(),
                );
            }
        }
        self.screenshot("ready")?;
        Ok(handle)
    }

    fn launch_and_ready(&mut self, tray: &str) -> Result<(String, Duration, TrayHandle)> {
        require_exec(
            self.guest()?,
            command("/usr/bin/rm", &["--force", HTTP_TOKEN_PATH]),
            COMMAND_TIMEOUT,
        )?;
        let started = Instant::now();
        let handle = self.spawn_tray(tray)?;
        let token = wait_for_command(
            self.guest()?,
            command("/usr/bin/cat", &[HTTP_TOKEN_PATH]),
            READY_TIMEOUT,
            |outcome| !outcome.stdout.trim().is_empty(),
            "the portable tray HTTP token",
        )?;
        let auth = format!("X-Qol-Token: {}", token.stdout.trim());
        let api = format!("{}/api/shortcuts", local_base_url());
        wait_for_command(
            self.guest()?,
            command(
                "/usr/bin/curl",
                &["--fail", "--silent", "--header", &auth, &api],
            ),
            READY_TIMEOUT,
            |_| true,
            "the portable tray shortcuts API",
        )?;
        Ok((auth, started.elapsed(), handle))
    }

    fn launcher_roundtrip(&mut self, auth: &str, inserted_at: Instant) -> Result<Duration> {
        let api = format!("{}/api/installed", local_base_url());
        let installed = require_exec(
            self.guest()?,
            command(
                "/usr/bin/curl",
                &["--fail", "--silent", "--header", auth, &api],
            ),
            COMMAND_TIMEOUT,
        )?;
        if !installed.stdout.contains("plugin-launcher") {
            bail!(
                "plugin-launcher missing from adopted profile: {}",
                installed.stdout.trim()
            );
        }
        dispatch_plugin_action(
            self.guest()?,
            auth,
            "plugin-launcher",
            "open",
            "{}",
            ACTION_TIMEOUT,
        )?;
        wait_for_window_title(
            self.guest()?,
            &["getwindowfocus", "getwindowname"],
            |title| title.starts_with("qol-launcher@"),
            "Launcher",
            ACTION_TIMEOUT,
        )?;
        let usable = inserted_at.elapsed();
        xdotool_key(self.guest()?, "Escape", false)?;
        Ok(usable)
    }

    fn graceful_exit_phase(&mut self, handle: &TrayHandle) -> Result<()> {
        let pid = handle.guest_pid.to_string();
        require_exec(
            self.guest()?,
            command("/usr/bin/kill", &["--signal", "TERM", &pid]),
            COMMAND_TIMEOUT,
        )?;
        if !self.reap_tray(handle, EXIT_SETTLE)? {
            self.notes
                .push("graceful-exit: tray ignored sigterm, harness terminated it".to_string());
            self.terminate_tray(handle);
        }
        let leftovers = self.wait_processes_gone(EXIT_SETTLE)?;
        self.assert_check(
            "graceful-exit-processes",
            leftovers.is_empty(),
            summarize("processes after sigterm", &leftovers),
        );
        if !leftovers.is_empty() {
            self.harness_cleanup("graceful-exit")?;
        }
        self.snapshot("post-exit")?;
        let (home, dconf) = self.capture_deltas("post-exit")?;
        self.assert_check(
            "graceful-exit-residue",
            home.is_empty() && dconf.is_empty(),
            format!(
                "home delta {} lines, dconf delta {} lines",
                home.len(),
                dconf.len()
            ),
        );
        self.post_exit_home = home;
        self.post_exit_dconf = dconf;
        self.assert_no_host_profile_writes();
        self.screenshot("post-exit")
    }

    fn assert_no_host_profile_writes(&mut self) {
        let profile: Vec<String> = self
            .post_exit_home
            .iter()
            .filter(|line| {
                line.contains("/.config/qol-tray/") || line.contains("/.local/share/qol-tray/")
            })
            .cloned()
            .collect();
        self.assert_check(
            "no-host-profile-writes",
            profile.is_empty(),
            summarize("host profile writes", &profile),
        );
    }

    fn crash_phase(&mut self, mount: &str) -> Result<()> {
        let tray = format!("{mount}/bin/qol-tray");
        let (_, relaunch, handle) = self.launch_and_ready(&tray)?;
        self.timings.relaunch_to_ready_ms.push(ms(relaunch));
        let pid = handle.guest_pid.to_string();
        require_exec(
            self.guest()?,
            command("/usr/bin/kill", &["--signal", "KILL", &pid]),
            COMMAND_TIMEOUT,
        )?;
        if !self.reap_tray(&handle, COMMAND_TIMEOUT)? {
            self.terminate_tray(&handle);
        }
        let leftovers = self.wait_processes_gone(EXIT_SETTLE)?;
        self.assert_check(
            "crash-cleanup",
            leftovers.is_empty(),
            summarize("processes after sigkill", &leftovers),
        );
        if !leftovers.is_empty() {
            self.harness_cleanup("crash")?;
        }
        Ok(())
    }

    fn live_pull_phase(&mut self, mount: &str, device: &str) -> Result<()> {
        let tray = format!("{mount}/bin/qol-tray");
        let (_, relaunch, handle) = self.launch_and_ready(&tray)?;
        self.timings.relaunch_to_ready_ms.push(ms(relaunch));
        self.qmp.detach_usb_stick()?;
        step_label(
            "pull",
            StepKind::Success,
            "portable medium detached while the session was live",
        );
        let exited = self.reap_tray(&handle, EXIT_SETTLE)?;
        let survivors = self.qol_processes()?;
        self.assert_check(
            "live-pull-shutdown",
            exited && survivors.is_empty(),
            format!(
                "tray exited on its own: {exited}; {}",
                summarize("processes after live pull", &survivors)
            ),
        );
        if !exited {
            self.notes
                .push("live-pull: tray kept running, harness terminated it".to_string());
            self.terminate_tray(&handle);
        }
        if !survivors.is_empty() {
            self.harness_cleanup("live-pull")?;
        }
        self.assert_automount_cleared(device)?;
        self.snapshot("post-pull")?;
        let (home, dconf) = self.capture_deltas("post-pull")?;
        self.post_pull_home = home;
        self.post_pull_dconf = dconf;
        Ok(())
    }

    fn assert_automount_cleared(&mut self, device: &str) -> Result<()> {
        let deadline = Instant::now() + AUTOMOUNT_WAIT;
        let mut mounted = true;
        while mounted {
            let outcome = exec(
                self.guest()?,
                command("/usr/bin/findmnt", &["-nro", "TARGET", "-S", device]),
                COMMAND_TIMEOUT,
            )?;
            mounted = outcome.exit_code == Some(0) && !outcome.stdout.trim().is_empty();
            if !mounted || Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
        let listing = exec(
            self.guest()?,
            command("/usr/bin/ls", &["-A", "/media/qol"]),
            COMMAND_TIMEOUT,
        )?;
        let stale: Vec<String> = listing
            .stdout
            .split_whitespace()
            .map(str::to_string)
            .collect();
        self.assert_check(
            "live-pull-automount-cleared",
            !mounted && stale.is_empty(),
            format!(
                "device still mounted: {mounted}; {}",
                summarize("stale mount dirs", &stale)
            ),
        );
        Ok(())
    }

    fn reboot_phase(&mut self) -> Result<()> {
        require_exec(
            self.guest()?,
            command("/usr/bin/sync", &[]),
            COMMAND_TIMEOUT,
        )?;
        self.guest = None;
        self.qmp.system_reset()?;
        step_label(
            "reboot",
            StepKind::Pending,
            "guest resetting with the medium removed",
        );
        self.guest = Some(connect_desktop_guest(self.vm)?);
        stop_preinstalled_runtime(self.guest()?)?;
        self.snapshot("post-reboot")?;
        let (home, dconf) = self.capture_deltas("post-reboot")?;
        self.assert_check(
            "reboot-residue",
            home.is_empty() && dconf.is_empty(),
            format!(
                "home delta {} lines, dconf delta {} lines (includes preinstalled runtime churn between login and quiesce)",
                home.len(),
                dconf.len()
            ),
        );
        self.post_reboot_home = home;
        self.post_reboot_dconf = dconf;
        self.screenshot("post-reboot")
    }

    fn snapshot(&mut self, tag: &str) -> Result<()> {
        let script = snapshot_script(tag);
        require_exec(
            self.guest()?,
            command("/usr/bin/bash", &["-lc", &script]),
            COMMAND_TIMEOUT,
        )?;
        Ok(())
    }

    fn capture_deltas(&mut self, tag: &str) -> Result<(Vec<String>, Vec<String>)> {
        let home = self.diff_snapshot("home", tag)?;
        let dconf = self.diff_snapshot("dconf", tag)?;
        Ok((home, dconf))
    }

    fn diff_snapshot(&mut self, kind: &str, tag: &str) -> Result<Vec<String>> {
        let base = format!("{SNAPSHOT_DIR}/{kind}-baseline.txt");
        let current = format!("{SNAPSHOT_DIR}/{kind}-{tag}.txt");
        let witness = exec(
            self.guest()?,
            command("/usr/bin/test", &["-s", &base]),
            COMMAND_TIMEOUT,
        )?;
        if witness.exit_code != Some(0) {
            bail!("baseline witness {base} is missing or empty");
        }
        let outcome = exec(
            self.guest()?,
            command("/usr/bin/diff", &[&base, &current]),
            COMMAND_TIMEOUT,
        )?;
        match outcome.exit_code {
            Some(0 | 1) => Ok(truncate_lines(&outcome.stdout, MAX_DELTA_LINES)),
            code => bail!(
                "diff {kind} {tag} failed with exit {code:?}: {}",
                outcome.stderr.trim()
            ),
        }
    }

    fn spawn_tray(&mut self, tray: &str) -> Result<TrayHandle> {
        let request = RequestAction::Spawn {
            command: command(tray, &[]),
        };
        match self.guest()?.request(request, COMMAND_TIMEOUT)? {
            ResponseResult::Spawned {
                process_id,
                guest_pid,
            } => Ok(TrayHandle {
                process_id,
                guest_pid,
            }),
            result => bail!("guest spawn returned an unexpected response: {result:?}"),
        }
    }

    fn reap_tray(&mut self, handle: &TrayHandle, budget: Duration) -> Result<bool> {
        let timeout_ms = u64::try_from(budget.as_millis()).unwrap_or(u64::MAX);
        let request = RequestAction::Wait {
            process_id: handle.process_id,
            timeout_ms,
        };
        match self
            .guest()?
            .request(request, budget + Duration::from_secs(2))
        {
            Ok(ResponseResult::Process { outcome }) => Ok(matches!(
                outcome.state,
                ProcessState::Exited | ProcessState::Terminated
            )),
            Ok(result) => bail!("guest wait returned an unexpected response: {result:?}"),
            Err(error) => {
                self.notes
                    .push(format!("tray wait did not reap: {error:#}"));
                Ok(false)
            }
        }
    }

    fn terminate_tray(&mut self, handle: &TrayHandle) {
        let request = RequestAction::Terminate {
            process_id: handle.process_id,
        };
        let result = match self.guest() {
            Ok(guest) => guest.request(request, COMMAND_TIMEOUT).map(|_| ()),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            self.notes.push(format!("tray terminate failed: {error:#}"));
        }
    }

    fn qol_processes(&mut self) -> Result<Vec<String>> {
        let outcome = exec(
            self.guest()?,
            command(
                "/usr/bin/pgrep",
                &["--list-full", "--full", QOL_PROCESS_PATTERN],
            ),
            COMMAND_TIMEOUT,
        )?;
        match outcome.exit_code {
            Some(0) => Ok(outcome.stdout.lines().map(str::to_string).collect()),
            Some(1) => Ok(Vec::new()),
            code => bail!("pgrep failed with exit {code:?}: {}", outcome.stderr.trim()),
        }
    }

    fn wait_processes_gone(&mut self, budget: Duration) -> Result<Vec<String>> {
        let deadline = Instant::now() + budget;
        loop {
            let processes = self.qol_processes()?;
            if processes.is_empty() || Instant::now() >= deadline {
                return Ok(processes);
            }
            thread::sleep(Duration::from_millis(200));
        }
    }

    fn harness_cleanup(&mut self, phase: &str) -> Result<()> {
        let outcome = exec(
            self.guest()?,
            command(
                "/usr/bin/pkill",
                &["--signal", "KILL", "--full", QOL_PROCESS_PATTERN],
            ),
            COMMAND_TIMEOUT,
        )?;
        if outcome.exit_code == Some(0) {
            self.notes
                .push(format!("{phase}: harness killed surviving qol processes"));
        }
        Ok(())
    }

    fn screenshot(&mut self, tag: &str) -> Result<()> {
        let path = self.artifacts_dir.join(format!("{tag}.ppm"));
        self.qmp.screendump(&path)?;
        self.artifacts.push(path);
        Ok(())
    }

    fn assert_check(&mut self, id: &'static str, pass: bool, detail: String) {
        let kind = if pass {
            StepKind::Success
        } else {
            StepKind::Info
        };
        let verdict = if pass { "pass" } else { "FAIL" };
        step_label("check", kind, &format!("{id}: {verdict} - {detail}"));
        self.assertions.push(AssertionOutcome { id, pass, detail });
    }

    fn write_report(&self) -> Result<PathBuf> {
        let report = serde_json::json!({
            "schema": 1,
            "workflow": "portable-session",
            "environment": self.vm.environment.id,
            "image_revision": self.vm.launch.guest_image_revision,
            "payload_profile": "sandbox",
            "medium_transport": "usb-storage (payload iso, read-only)",
            "timings": self.timings,
            "assertions": self.assertions,
            "residue": {
                "post_exit": {"home": self.post_exit_home, "dconf": self.post_exit_dconf},
                "post_pull": {"home": self.post_pull_home, "dconf": self.post_pull_dconf},
                "post_reboot": {"home": self.post_reboot_home, "dconf": self.post_reboot_dconf},
                "home_manifest_exclusions": HOME_EXCLUDES,
            },
            "not_measured": [
                "x11 hotkey grabs",
                "profile persistence onto a writable portable medium (payload iso is read-only)",
                "macos and windows guests",
            ],
            "notes": self.notes,
        });
        let path = self.artifacts_dir.join("portable-session-report.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&report)?)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(path)
    }

    fn into_verdict(mut self, report_path: PathBuf, completed: bool) -> Verdict {
        let pass = completed && self.assertions.iter().all(|assertion| assertion.pass);
        let mut traces: Vec<String> = self
            .assertions
            .iter()
            .map(|assertion| {
                let verdict = if assertion.pass { "pass" } else { "fail" };
                format!("{}: {verdict} ({})", assertion.id, assertion.detail)
            })
            .collect();
        traces.append(&mut self.notes);
        let mut artifacts = std::mem::take(&mut self.artifacts);
        artifacts.push(report_path);
        Verdict {
            pass,
            traces,
            artifacts,
        }
    }
}

fn ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn summarize(label: &str, lines: &[String]) -> String {
    match lines.first() {
        None => format!("{label}: none"),
        Some(first) => format!("{label}: {} (first: {first})", lines.len()),
    }
}

fn usb_disk_name(lsblk: &str) -> Option<String> {
    lsblk.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let name = parts.next()?;
        let tran = parts.next()?;
        let kind = parts.next()?;
        (tran == "usb" && kind == "disk").then(|| name.to_string())
    })
}

fn parse_mount_point(output: &str) -> Option<String> {
    let (_, tail) = output.split_once(" at ")?;
    let mount = tail.trim().trim_end_matches('.');
    (!mount.is_empty()).then(|| mount.to_string())
}

fn truncate_lines(text: &str, max: usize) -> Vec<String> {
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    if lines.len() <= max {
        return lines;
    }
    let total = lines.len();
    let mut kept: Vec<String> = lines.into_iter().take(max).collect();
    kept.push(format!("... truncated, {total} delta lines total"));
    kept
}

fn snapshot_script(tag: &str) -> String {
    let mut prune = String::new();
    for (index, path) in HOME_EXCLUDES.iter().enumerate() {
        if index > 0 {
            prune.push_str(" -o ");
        }
        let _ = write!(prune, "-path {path}");
    }
    format!(
        "mkdir -p {SNAPSHOT_DIR} && \
         find /home/qol /etc/xdg/autostart -xdev \\( {prune} \\) -prune -o -printf '%p|%s|%T@\\n' | sort > {SNAPSHOT_DIR}/home-{tag}.txt && \
         dconf dump / > {SNAPSHOT_DIR}/dconf-{tag}.txt && \
         sync"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_disk_name_selects_only_usb_disks() {
        let cases = [
            ("sda usb disk\n", Some("sda")),
            ("vda virtio disk\nsdb usb disk\n", Some("sdb")),
            ("sda usb part\n", None),
            ("vda virtio disk\n", None),
            ("", None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                usb_disk_name(input).as_deref(),
                expected,
                "input: {input:?}"
            );
        }
    }

    #[test]
    fn parse_mount_point_handles_udisksctl_output_forms() {
        let cases = [
            (
                "Mounted /dev/sda at /media/qol/QOL_PAYLOAD",
                Some("/media/qol/QOL_PAYLOAD"),
            ),
            (
                "Mounted /dev/sda at /media/qol/QOL_PAYLOAD.\n",
                Some("/media/qol/QOL_PAYLOAD"),
            ),
            ("Mounted /dev/sda at ", None),
            ("unexpected", None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                parse_mount_point(input).as_deref(),
                expected,
                "input: {input:?}"
            );
        }
    }

    #[test]
    fn truncate_lines_caps_and_reports_totals() {
        let short = truncate_lines("a\nb\n", 4);
        assert_eq!(short, vec!["a".to_string(), "b".to_string()]);
        let long = truncate_lines("a\nb\nc\n", 2);
        assert_eq!(
            long,
            vec![
                "a".to_string(),
                "b".to_string(),
                "... truncated, 3 delta lines total".to_string()
            ]
        );
    }

    #[test]
    fn snapshot_script_prunes_every_exclusion_and_writes_both_snapshots() {
        let script = snapshot_script("baseline");
        assert!(script.ends_with("sync"));
        assert!(!script.contains("/media"));
        for exclusion in HOME_EXCLUDES {
            assert!(script.contains(exclusion), "missing: {exclusion}");
        }
        assert!(script.contains("home-baseline.txt"));
        assert!(script.contains("dconf-baseline.txt"));
        assert!(script.contains("-xdev"));
    }

    #[test]
    fn summarize_reports_counts_with_first_entry() {
        let empty: Vec<String> = Vec::new();
        assert_eq!(summarize("things", &empty), "things: none");
        let listed = vec!["one".to_string(), "two".to_string()];
        assert_eq!(summarize("things", &listed), "things: 2 (first: one)");
    }
}
