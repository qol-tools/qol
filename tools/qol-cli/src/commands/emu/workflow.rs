use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::progress::{run_step, step_label, StepKind};

use super::arch::GuestArch;
use super::guest::GuestOs;
use super::qmp::QmpClient;
use super::serial::SerialClient;

const PAYLOAD_SOURCE: &str = "#!/bin/sh\nexit 0\n";

pub(crate) struct Verdict {
    pub(crate) pass: bool,
    pub(crate) traces: Vec<String>,
    pub(crate) evidence: Value,
}

pub(crate) struct Run<'a> {
    pub(crate) qmp: &'a mut QmpClient,
    pub(crate) serial: &'a mut SerialClient,
    pub(crate) os: &'a dyn GuestOs,
    pub(crate) stick: &'a Path,
}

impl Run<'_> {
    fn insert(&mut self) -> Result<()> {
        self.qmp.attach_usb_stick(self.stick)?;
        step_label(
            "insert",
            StepKind::Success,
            &self.stick.display().to_string(),
        );
        Ok(())
    }

    fn verify_uninstall(&mut self) -> Result<Value> {
        let evidence = self.os.verify_uninstall_from_stick(self.serial)?;
        step_label(
            "verify",
            StepKind::Success,
            "real installer report is complete",
        );
        Ok(evidence)
    }

    fn pull(&mut self) -> Result<()> {
        self.qmp.detach_usb_stick()?;
        step_label("pull", StepKind::Success, "usb stick detached");
        Ok(())
    }

    fn reboot(&mut self) -> Result<()> {
        step_label("reboot", StepKind::Pending, "rebooting guest");
        self.os.reboot_and_relogin(self.serial)?;
        step_label("reboot", StepKind::Success, "guest back at root shell");
        Ok(())
    }

    fn list_traces(&mut self) -> Result<Vec<String>> {
        let traces = self.os.list_qol_traces(self.serial)?;
        step_label(
            "traces",
            StepKind::Success,
            &format!("{} found", traces.len()),
        );
        Ok(traces)
    }
}

pub(crate) struct PrepareContext<'a> {
    pub(crate) root: &'a Path,
    pub(crate) run_dir: &'a Path,
    pub(crate) qemu_img: &'a Path,
    pub(crate) guest_arch: GuestArch,
    pub(crate) verbose: bool,
}

type WorkflowFn = fn(&mut Run) -> Result<Verdict>;
type PrepareFn = for<'a> fn(&PrepareContext<'a>) -> Result<PathBuf>;

pub(crate) struct Workflow {
    id: &'static str,
    run: WorkflowFn,
    prepare: PrepareFn,
}

impl Workflow {
    pub(crate) fn run(&self, run: &mut Run) -> Result<Verdict> {
        (self.run)(run)
    }

    pub(crate) fn prepare(&self, context: &PrepareContext<'_>) -> Result<PathBuf> {
        (self.prepare)(context)
    }
}

const REGISTRY: &[Workflow] = &[Workflow {
    id: "leaves-no-trace",
    run: leaves_no_trace,
    prepare: prepare_verified_uninstall,
}];

pub(crate) fn find(id: &str) -> Option<&'static Workflow> {
    REGISTRY.iter().find(|workflow| workflow.id == id)
}

pub(crate) fn ids() -> Vec<&'static str> {
    REGISTRY.iter().map(|workflow| workflow.id).collect()
}

fn leaves_no_trace(run: &mut Run) -> Result<Verdict> {
    run.insert()?;
    let evidence = run.verify_uninstall()?;
    run.pull()?;
    run.reboot()?;
    let traces = run.list_traces()?;
    Ok(Verdict {
        pass: traces.is_empty(),
        traces,
        evidence,
    })
}

fn prepare_verified_uninstall(context: &PrepareContext<'_>) -> Result<PathBuf> {
    if !super::platform::supports_native_linux_payload(context.guest_arch) {
        bail!(
            "verified Linux uninstall needs a Linux guest matching the host architecture; host={}, guest={}",
            std::env::consts::ARCH,
            context.guest_arch.as_str()
        );
    }
    let installer = build_installer(context)?;
    let staging = context.run_dir.join("payload");
    let prepare_result = (|| {
        stage_payload(&installer, &staging)?;
        super::machine::ensure_payload_stick(context.run_dir, context.qemu_img, &staging)
    })();
    let cleanup_result = remove_staging(&staging);
    let stick = match (prepare_result, cleanup_result) {
        (Ok(stick), Ok(())) => stick,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
        (Err(error), Err(cleanup_error)) => {
            return Err(error.context(format!(
                "payload preparation also failed to clean staging: {cleanup_error:#}"
            )))
        }
    };
    step_label("payload", StepKind::Success, &stick.display().to_string());
    Ok(stick)
}

fn build_installer(context: &PrepareContext<'_>) -> Result<PathBuf> {
    let mut build = Command::new("cargo");
    build.current_dir(context.root).args([
        "build",
        "--release",
        "-p",
        "qol-tray",
        "--bin",
        "qol-tray-install",
    ]);
    run_step(
        "payload",
        StepKind::Pending,
        "release qol-tray-install",
        &mut build,
        context.verbose,
    )?;
    let installer = context
        .root
        .join("target/release")
        .join(crate::host_facade::exe_name("qol-tray-install"));
    if installer.is_file() {
        return Ok(installer);
    }
    bail!("built installer is missing at {}", installer.display())
}

fn stage_payload(installer: &Path, staging: &Path) -> Result<()> {
    if staging.exists() {
        fs::remove_dir_all(staging)
            .with_context(|| format!("failed to reset payload staging at {}", staging.display()))?;
    }
    fs::create_dir_all(staging)
        .with_context(|| format!("failed to create payload staging at {}", staging.display()))?;
    fs::copy(installer, staging.join("qol-tray-install"))
        .with_context(|| format!("failed to stage {}", installer.display()))?;
    fs::write(staging.join("qol-tray-source"), PAYLOAD_SOURCE)
        .with_context(|| format!("failed to write payload source at {}", staging.display()))?;
    Ok(())
}

fn remove_staging(staging: &Path) -> Result<()> {
    match fs::remove_dir_all(staging) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to remove payload staging at {}", staging.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_resolves_only_registered_workflows() {
        let cases = [("leaves-no-trace", true), ("unknown", false), ("", false)];
        for (id, expected) in cases {
            assert_eq!(find(id).is_some(), expected, "id: {id}");
        }
    }

    #[test]
    fn ids_lists_every_registered_workflow() {
        assert_eq!(ids(), vec!["leaves-no-trace"]);
    }

    #[test]
    fn stage_payload_copies_the_real_installer_and_inert_source() {
        let tmp = tempfile::TempDir::new().unwrap();
        let installer = tmp.path().join("installer");
        let staging = tmp.path().join("payload");
        fs::write(&installer, b"real installer").unwrap();

        stage_payload(&installer, &staging).unwrap();

        assert_eq!(
            fs::read(staging.join("qol-tray-install")).unwrap(),
            b"real installer"
        );
        assert_eq!(
            fs::read_to_string(staging.join("qol-tray-source")).unwrap(),
            PAYLOAD_SOURCE
        );
    }
}
