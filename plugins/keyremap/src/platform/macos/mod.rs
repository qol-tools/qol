mod app;
mod app_tracker;
mod doctor;
mod hid_helper;
mod input;
mod karabiner;
mod layout;
mod secure_input;
mod tap;
mod virtual_hid;

use anyhow::{Context, Result};
use qol_headless::{CommandResult, DoctorCheckResult};

use super::engine::{self, config, remap};
use super::{ConfigInspection, PlatformAdapter};
use doctor::{KarabinerRules, LayoutGap, Probe};
use input::selection::{self, Availability, Selection};
use input::BackendId;

#[derive(Clone, Copy)]
pub(crate) struct Adapter;

impl PlatformAdapter for Adapter {
    fn name(&self) -> &'static str {
        qol_platform::Os::Macos.label()
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

    fn karabiner_rules(&self) -> Result<CommandResult> {
        let compiled = compiled_rules()?;
        Ok(CommandResult::success(format!(
            "{}\n",
            serde_json::to_string_pretty(&karabiner::report(&compiled))?
        )))
    }

    fn karabiner_sync(&self) -> Result<CommandResult> {
        let compiled = compiled_rules()?;
        let store = karabiner::store::Store::locate()?;
        let synced = store.sync(&compiled)?;
        Ok(CommandResult::success(karabiner::sync_summary(
            &synced,
            &compiled,
            store.config_path(),
        )))
    }

    fn karabiner_remove(&self) -> Result<CommandResult> {
        let store = karabiner::store::Store::locate()?;
        let removed = store.remove()?;
        Ok(CommandResult::success(karabiner::remove_summary(
            &removed,
            store.config_path(),
        )))
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

    fn key_backend(&self) -> DoctorCheckResult {
        doctor::key_backend_result(&selected_backend(&hid_helper::query_status()))
    }

    fn karabiner_block(&self) -> DoctorCheckResult {
        let chosen =
            config::load_config().key_backend == config::KeyBackendChoice::KarabinerElements;
        doctor::karabiner_rules_result(chosen, &karabiner_rules_probe())
    }

    fn secure_input(&self) -> DoctorCheckResult {
        let helper = hid_helper::query_status();
        let seized = matches!(
            &helper,
            Probe::Known(doctor::HelperState::Running(report)) if !report.seized.is_empty()
        );
        let backend = match selected_backend(&helper).backend {
            BackendId::HidHelper if !seized => BackendId::EventTap,
            backend => backend,
        };
        doctor::secure_input_result(&Probe::Known(qol_platform::secure_input_holder()), backend)
    }

    fn layout_characters(&self) -> DoctorCheckResult {
        doctor::layout_result(&layout_gaps())
    }
}

fn compiled_rules() -> Result<karabiner::compile::Compiled> {
    let snapshot = layout::LayoutSnapshot::read_current()?;
    let resolved = remap::resolve(&config::load_config());
    let hotkeys = karabiner::hotkeys::from_tray().context(
        "qol-tray did not send its hotkeys; start qol-tray so they go into Karabiner-Elements with the rules",
    )?;
    let physical = snapshot.physical;
    Ok(karabiner::compile::compile(&resolved, &snapshot)
        .with_system_modifier_keys(karabiner::system_modifier_keys::read())
        .with_hotkeys(&hotkeys, physical))
}

fn selected_backend(helper: &Probe<doctor::HelperState>) -> Selection {
    selection::select(config::load_config().key_backend, |backend| match backend {
        BackendId::EventTap => Availability::Ready,
        BackendId::HidHelper => doctor::helper_availability(helper),
        BackendId::KarabinerElements => match karabiner::store::Store::locate() {
            Ok(store) => karabiner::service::probe(&store).availability,
            Err(error) => Availability::Missing {
                what: "a place for its rules".to_string(),
                fix: format!("{error:#}"),
            },
        },
    })
}

fn karabiner_rules_probe() -> Probe<KarabinerRules> {
    let planned = compiled_rules().and_then(|compiled| {
        let synced = karabiner::store::Store::locate()?.plan(&compiled)?;
        Ok((compiled, synced))
    });
    match planned {
        Ok((compiled, synced)) => Probe::Known(KarabinerRules {
            current: !synced.changed,
            profile: synced.profile,
            count: compiled.rule["manipulators"].as_array().map_or(0, Vec::len),
            conflicts: synced.conflicts,
            gaps: compiled
                .gaps
                .iter()
                .map(|gap| format!("{}: {}", gap.rule, gap.reason))
                .collect(),
        }),
        Err(error) => Probe::Unknown(format!("{error:#}")),
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
