use anyhow::Result;
use std::path::Path;
use std::path::PathBuf;

use crate::progress::{step_label, StepKind};

use super::guest::GuestOs;
use super::qmp::QmpClient;
use super::serial::SerialClient;
use super::BootedVm;

mod alt_tab;
mod bluetooth;
mod desktop;
mod hotkey_shadow;
mod hotkey_shadow_boot;
mod hotkeys;
mod launcher;
mod portable_session;
mod qol_shot;
mod qol_shot_cold_boot;
pub(crate) mod resident_wave2;
mod shortcuts;
mod window_actions;

pub(crate) struct Verdict {
    pub(crate) pass: bool,
    pub(crate) traces: Vec<String>,
    pub(crate) artifacts: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PayloadRecipe {
    None,
    Desktop,
    ResidentWave2,
}

#[derive(Clone, Copy)]
pub(crate) enum Definition {
    Serial {
        id: &'static str,
        run: SerialWorkflow,
        payload: PayloadRecipe,
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
        self.payload_recipe() != Some(PayloadRecipe::None)
    }

    pub(crate) fn requires_guest_revision(self) -> bool {
        matches!(self, Self::Desktop { .. })
    }

    pub(crate) fn payload_recipe(self) -> Option<PayloadRecipe> {
        match self {
            Self::Desktop { .. } => Some(PayloadRecipe::Desktop),
            Self::Serial { payload, .. } => Some(payload),
        }
    }
}

pub(crate) struct Run<'a> {
    pub(crate) qmp: &'a mut QmpClient,
    pub(crate) serial: &'a mut SerialClient,
    pub(crate) os: &'a dyn GuestOs,
    pub(crate) stick: &'a Path,
    pub(crate) image_path: &'a Path,
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
    AltTabPerformance,
    AltTabStorm,
    BluetoothStorm,
    HotkeyShadow,
    HotkeyShadowBoot,
    HotkeyStorm,
    LauncherStorm,
    PortableSession,
    QolShotCapture,
    QolShotColdBoot,
    QolShotStorm,
    ShortcutStorm,
    WindowActionsStorm,
}

pub(super) use super::strategy::DesktopStrategy as DesktopGuestPlatform;

pub(crate) fn run_desktop(
    vm: &BootedVm,
    workflow: DesktopWorkflow,
    platform: DesktopGuestPlatform,
) -> Result<Verdict> {
    match workflow {
        DesktopWorkflow::AltTabPerformance => alt_tab::run_performance(vm, platform),
        DesktopWorkflow::AltTabStorm => alt_tab::run(vm, platform),
        DesktopWorkflow::BluetoothStorm => bluetooth::run(vm, platform),
        DesktopWorkflow::HotkeyShadow => hotkey_shadow::run(vm, platform),
        DesktopWorkflow::HotkeyShadowBoot => hotkey_shadow_boot::run(vm, platform),
        DesktopWorkflow::HotkeyStorm => hotkeys::run(vm, platform),
        DesktopWorkflow::LauncherStorm => launcher::run(vm, platform),
        DesktopWorkflow::PortableSession => portable_session::run(vm, platform),
        DesktopWorkflow::QolShotCapture => desktop::run(vm, platform),
        DesktopWorkflow::QolShotColdBoot => qol_shot_cold_boot::run(vm, platform),
        DesktopWorkflow::QolShotStorm => qol_shot::run(vm, platform),
        DesktopWorkflow::ShortcutStorm => shortcuts::run(vm, platform),
        DesktopWorkflow::WindowActionsStorm => window_actions::run(vm, platform),
    }
}

const REGISTRY: &[Definition] = &[
    Definition::Serial {
        id: "leaves-no-trace",
        run: leaves_no_trace,
        payload: PayloadRecipe::None,
    },
    Definition::Serial {
        id: "resident-wave2-apt-preferences",
        run: resident_wave2::run,
        payload: PayloadRecipe::ResidentWave2,
    },
    Definition::Serial {
        id: "resident-wave2-package-contract",
        run: resident_wave2::run_package_contract,
        payload: PayloadRecipe::ResidentWave2,
    },
    Definition::Desktop {
        id: "alt-tab-performance",
        run: DesktopWorkflow::AltTabPerformance,
    },
    Definition::Desktop {
        id: "alt-tab-storm",
        run: DesktopWorkflow::AltTabStorm,
    },
    Definition::Desktop {
        id: "bluetooth-storm",
        run: DesktopWorkflow::BluetoothStorm,
    },
    Definition::Desktop {
        id: "hotkey-shadow",
        run: DesktopWorkflow::HotkeyShadow,
    },
    Definition::Desktop {
        id: "hotkey-shadow-boot",
        run: DesktopWorkflow::HotkeyShadowBoot,
    },
    Definition::Desktop {
        id: "hotkey-storm",
        run: DesktopWorkflow::HotkeyStorm,
    },
    Definition::Desktop {
        id: "launcher-storm",
        run: DesktopWorkflow::LauncherStorm,
    },
    Definition::Desktop {
        id: "portable-session",
        run: DesktopWorkflow::PortableSession,
    },
    Definition::Desktop {
        id: "qol-shot-capture",
        run: DesktopWorkflow::QolShotCapture,
    },
    Definition::Desktop {
        id: "qol-shot-cold-boot",
        run: DesktopWorkflow::QolShotColdBoot,
    },
    Definition::Desktop {
        id: "qol-shot-storm",
        run: DesktopWorkflow::QolShotStorm,
    },
    Definition::Desktop {
        id: "shortcut-storm",
        run: DesktopWorkflow::ShortcutStorm,
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
        let cases = [
            ("alt-tab-performance", true),
            ("alt-tab-storm", true),
            ("bluetooth-storm", true),
            ("hotkey-shadow", true),
            ("hotkey-shadow-boot", true),
            ("hotkey-storm", true),
            ("leaves-no-trace", true),
            ("resident-wave2-package-contract", true),
            ("launcher-storm", true),
            ("portable-session", true),
            ("qol-shot-cold-boot", true),
            ("qol-shot-storm", true),
            ("shortcut-storm", true),
            ("unknown", false),
            ("", false),
        ];
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
                "resident-wave2-apt-preferences",
                "resident-wave2-package-contract",
                "alt-tab-performance",
                "alt-tab-storm",
                "bluetooth-storm",
                "hotkey-shadow",
                "hotkey-shadow-boot",
                "hotkey-storm",
                "launcher-storm",
                "portable-session",
                "qol-shot-capture",
                "qol-shot-cold-boot",
                "qol-shot-storm",
                "shortcut-storm",
                "window-actions-storm"
            ]
        );
    }

    #[test]
    fn guest_revision_is_required_only_for_desktop_workflows() {
        for id in [
            "leaves-no-trace",
            "resident-wave2-apt-preferences",
            "resident-wave2-package-contract",
        ] {
            assert!(!find(id).unwrap().requires_guest_revision(), "{id}");
        }
        for id in [
            "alt-tab-performance",
            "alt-tab-storm",
            "bluetooth-storm",
            "hotkey-shadow",
            "hotkey-shadow-boot",
            "hotkey-storm",
            "launcher-storm",
            "portable-session",
            "qol-shot-capture",
            "qol-shot-cold-boot",
            "qol-shot-storm",
            "shortcut-storm",
            "window-actions-storm",
        ] {
            assert!(find(id).unwrap().requires_guest_revision(), "{id}");
        }
    }

    #[test]
    fn payload_recipe_coverage_is_typed_and_exhaustive() {
        assert_eq!(
            find("leaves-no-trace").unwrap().payload_recipe(),
            Some(PayloadRecipe::None)
        );
        assert!(!find("leaves-no-trace").unwrap().requires_payload());
        assert_eq!(
            find("resident-wave2-apt-preferences")
                .unwrap()
                .payload_recipe(),
            Some(PayloadRecipe::ResidentWave2)
        );
        assert!(find("resident-wave2-apt-preferences")
            .unwrap()
            .requires_payload());
        assert_eq!(
            find("resident-wave2-package-contract")
                .unwrap()
                .payload_recipe(),
            Some(PayloadRecipe::ResidentWave2)
        );
        assert!(find("resident-wave2-package-contract")
            .unwrap()
            .requires_payload());
        for id in [
            "alt-tab-performance",
            "alt-tab-storm",
            "bluetooth-storm",
            "hotkey-shadow",
            "hotkey-shadow-boot",
            "hotkey-storm",
            "launcher-storm",
            "portable-session",
            "qol-shot-capture",
            "qol-shot-cold-boot",
            "qol-shot-storm",
            "shortcut-storm",
            "window-actions-storm",
        ] {
            assert_eq!(
                find(id).unwrap().payload_recipe(),
                Some(PayloadRecipe::Desktop),
                "{id}"
            );
            assert!(find(id).unwrap().requires_payload(), "{id}");
        }
    }

    #[test]
    fn desktop_guest_platform_resolution_is_runtime_guest_specific() {
        assert_eq!(
            super::super::GuestAdapter::MintCinnamon
                .plan()
                .desktop()
                .unwrap(),
            DesktopGuestPlatform::Linux
        );
        assert_eq!(
            super::super::GuestAdapter::MacosDesktop
                .plan()
                .desktop()
                .unwrap(),
            DesktopGuestPlatform::Macos
        );
        assert_eq!(
            super::super::GuestAdapter::WindowsDesktop
                .plan()
                .desktop()
                .unwrap(),
            DesktopGuestPlatform::Windows
        );
        assert!(super::super::GuestAdapter::DebianNocloud
            .plan()
            .desktop()
            .is_err());
    }
}
