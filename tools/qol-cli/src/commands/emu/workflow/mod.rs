use anyhow::Result;
use std::path::Path;
use std::path::PathBuf;

use crate::progress::{step_label, StepKind};

use super::guest::GuestOs;
use super::qmp::QmpClient;
use super::serial::SerialClient;
use super::BootedVm;

mod desktop;
mod window_actions;

pub(crate) struct Verdict {
    pub(crate) pass: bool,
    pub(crate) traces: Vec<String>,
    pub(crate) artifacts: Vec<PathBuf>,
}

#[derive(Clone, Copy)]
pub(crate) enum Definition {
    Serial {
        id: &'static str,
        run: SerialWorkflow,
    },
    Desktop {
        id: &'static str,
        run: DesktopWorkflow,
    },
}

impl Definition {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Serial { id, .. } | Self::Desktop { id, .. } => id,
        }
    }

    pub(crate) fn requires_payload(self) -> bool {
        matches!(self, Self::Desktop { .. })
    }
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

    fn launch_qol(&mut self) -> Result<()> {
        self.os.launch_qol_from_stick(self.serial)?;
        step_label("launch", StepKind::Success, "qol stub ran from the stick");
        Ok(())
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

pub(crate) type SerialWorkflow = fn(&mut Run) -> Result<Verdict>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DesktopWorkflow {
    QolShotCapture,
    WindowActionsStorm,
}

pub(crate) fn run_desktop(vm: &BootedVm, workflow: DesktopWorkflow) -> Result<Verdict> {
    match workflow {
        DesktopWorkflow::QolShotCapture => desktop::run(vm),
        DesktopWorkflow::WindowActionsStorm => window_actions::run(vm),
    }
}

const REGISTRY: &[Definition] = &[
    Definition::Serial {
        id: "leaves-no-trace",
        run: leaves_no_trace,
    },
    Definition::Desktop {
        id: "qol-shot-capture",
        run: DesktopWorkflow::QolShotCapture,
    },
    Definition::Desktop {
        id: "window-actions-storm",
        run: DesktopWorkflow::WindowActionsStorm,
    },
];

pub(crate) fn find(id: &str) -> Option<Definition> {
    REGISTRY
        .iter()
        .copied()
        .find(|workflow| workflow.id() == id)
}

pub(crate) fn ids() -> Vec<&'static str> {
    REGISTRY.iter().map(|workflow| workflow.id()).collect()
}

fn leaves_no_trace(run: &mut Run) -> Result<Verdict> {
    run.insert()?;
    run.launch_qol()?;
    run.pull()?;
    run.reboot()?;
    let traces = run.list_traces()?;
    Ok(Verdict {
        pass: traces.is_empty(),
        traces,
        artifacts: Vec::new(),
    })
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
        assert_eq!(
            ids(),
            vec![
                "leaves-no-trace",
                "qol-shot-capture",
                "window-actions-storm"
            ]
        );
    }

    #[test]
    fn only_desktop_workflows_require_a_payload() {
        assert!(!find("leaves-no-trace").unwrap().requires_payload());
        assert!(find("qol-shot-capture").unwrap().requires_payload());
        assert!(find("window-actions-storm").unwrap().requires_payload());
    }
}
