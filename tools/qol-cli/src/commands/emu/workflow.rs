use anyhow::Result;
use std::path::Path;

use crate::progress::{step_label, StepKind};

use super::guest::GuestOs;
use super::qmp::QmpClient;
use super::serial::SerialClient;

pub(crate) struct Verdict {
    pub(crate) pass: bool,
    pub(crate) traces: Vec<String>,
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

pub(crate) type Workflow = fn(&mut Run) -> Result<Verdict>;

const REGISTRY: &[(&str, Workflow)] = &[("leaves-no-trace", leaves_no_trace)];

pub(crate) fn find(id: &str) -> Option<Workflow> {
    REGISTRY
        .iter()
        .find_map(|(name, workflow)| (*name == id).then_some(*workflow))
}

pub(crate) fn ids() -> Vec<&'static str> {
    REGISTRY.iter().map(|(name, _)| *name).collect()
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
        assert_eq!(ids(), vec!["leaves-no-trace"]);
    }
}
