mod app;
mod app_tracker;
mod doctor;
mod hid_helper;
mod input;
mod layout;
mod secure_input;
mod tap;
mod virtual_hid;

use anyhow::Result;
use qol_headless::{CommandResult, DoctorCheckResult};

use super::engine::{self, config, remap};
use super::{ConfigInspection, PlatformAdapter};
use doctor::{LayoutGap, Probe};

#[derive(Clone, Copy)]
pub(crate) struct Adapter;

impl PlatformAdapter for Adapter {
    fn name(&self) -> &'static str {
        "macOS"
    }

    fn supported(&self) -> bool {
        true
    }

    fn launch(&self) -> Result<CommandResult> {
        app::run()?;
        Ok(CommandResult::success(""))
    }

    fn hid_helper(&self) -> Result<CommandResult> {
        hid_helper::run()?;
        Ok(CommandResult::success(""))
    }

    fn install_hid_helper(&self) -> Result<CommandResult> {
        Ok(summary_result(hid_helper::install::install()))
    }

    fn uninstall_hid_helper(&self) -> Result<CommandResult> {
        Ok(summary_result(hid_helper::install::uninstall()))
    }

    fn inspect_config(&self) -> Result<ConfigInspection> {
        engine::inspection(|_| Vec::new())
    }

    fn virtual_hid_driver(&self) -> Result<DoctorCheckResult> {
        Ok(doctor::driver_result(
            &virtual_hid::driver::driver_state(),
            &doctor::required_driver()?,
        ))
    }

    fn virtual_hid_daemon(&self) -> DoctorCheckResult {
        doctor::daemon_result(&virtual_hid::driver::daemon_running())
    }

    fn hid_helper_state(&self) -> DoctorCheckResult {
        doctor::helper_result(&hid_helper::query_status())
    }

    fn secure_input(&self) -> DoctorCheckResult {
        doctor::secure_input_result(
            &Probe::Known(secure_input::holder()),
            &hid_helper::query_status(),
        )
    }

    fn layout_characters(&self) -> DoctorCheckResult {
        doctor::layout_result(&layout_gaps())
    }
}

fn layout_gaps() -> Probe<Vec<LayoutGap>> {
    let snapshot = match layout::LayoutSnapshot::read_current() {
        Ok(snapshot) => snapshot,
        Err(error) => return Probe::Unknown(error.to_string()),
    };
    let resolved = remap::resolve(&config::load_config());
    let gaps = remap::character_targets(&resolved)
        .into_iter()
        .flat_map(|(rule, text)| {
            layout::missing_characters(&snapshot.table, [text.as_str()])
                .into_iter()
                .map(move |character| LayoutGap {
                    rule: rule.clone(),
                    character,
                })
        })
        .collect();
    Probe::Known(gaps)
}

fn summary_result(result: Result<String>) -> CommandResult {
    match result {
        Ok(summary) => CommandResult::success(format!("{summary}\n")),
        Err(error) => CommandResult::runtime_error(format!("keyremap: {error:#}")),
    }
}
