use anyhow::{bail, Context, Result};
use qol_platform::windows_path::{parent, same_path, within};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::installer) enum UninstallPlan {
    RemoveRoot(PathBuf),
    RemoveAfterExit(PathBuf),
}

pub(in crate::installer) fn plan(
    binary: &Path,
    install_dir: &Path,
    current_exe: &Path,
) -> Result<UninstallPlan> {
    let binary_dir = parent(binary).unwrap_or_else(|| binary.to_path_buf());
    if !same_path(&binary_dir, install_dir) {
        bail!(
            "refusing to remove {}: it is not the QoL Tray install directory {}",
            binary.display(),
            install_dir.display()
        );
    }
    let root = parent(install_dir).context("Install directory has no parent")?;
    if within(current_exe, &root) {
        return Ok(UninstallPlan::RemoveAfterExit(root));
    }
    Ok(UninstallPlan::RemoveRoot(root))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::installer) enum FileRemoval {
    Done,
    Deferred,
}

pub(in crate::installer) trait UninstallSteps {
    fn end_residency(&mut self) -> Result<()>;
    fn release_plugins(&mut self) -> Result<()>;
    fn stop_running(&mut self) -> Result<()>;
    fn remove_autostart(&mut self) -> Result<()>;
    fn remove_shell_entries(&mut self) -> Result<()>;
    fn remove_toast_identity(&mut self) -> Result<()>;
    fn remove_files(&mut self) -> Result<FileRemoval>;
    fn remove_uninstall_entry(&mut self) -> Result<()>;
}

pub(in crate::installer) fn run_uninstall(steps: &mut impl UninstallSteps) -> Result<()> {
    let residency = steps.end_residency();
    if let Err(error) = &residency {
        log::error!("ending residency before uninstall failed: {error:#}");
    }
    let plugins = steps.release_plugins();
    if let Err(error) = &plugins {
        log::error!("releasing plugin host changes before uninstall failed: {error:#}");
    }
    steps.stop_running()?;
    steps.remove_autostart()?;
    steps.remove_shell_entries()?;
    steps.remove_toast_identity()?;
    if steps.remove_files()? == FileRemoval::Done {
        steps.remove_uninstall_entry()?;
    }
    residency.context("QoL Tray was removed but residency did not end cleanly")?;
    plugins.context("QoL Tray was removed but a plugin kept a host change")
}

pub(in crate::installer) fn deferred_removal_script(
    root: &str,
    log: &str,
    reg: &str,
    uninstall_key: &str,
) -> Option<String> {
    if [root, log, reg, uninstall_key]
        .iter()
        .any(|part| part.contains(['"', '%', '!', '\r', '\n']))
    {
        return None;
    }
    Some(format!(
        "/d /v:off /c ping -n 3 127.0.0.1 >nul & echo removing \"{root}\">\"{log}\" & rmdir /s /q \"{root}\" >>\"{log}\" 2>&1 & if not exist \"{root}\\\" \"{reg}\" delete \"HKCU\\{uninstall_key}\" /f >>\"{log}\" 2>&1"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[derive(Default)]
    struct RecordedSteps {
        calls: Vec<&'static str>,
        failing: Vec<&'static str>,
        files: Option<FileRemoval>,
    }

    impl RecordedSteps {
        fn step(&mut self, name: &'static str) -> Result<()> {
            self.calls.push(name);
            if self.failing.contains(&name) {
                return Err(anyhow!("{name} failed"));
            }
            Ok(())
        }
    }

    impl UninstallSteps for RecordedSteps {
        fn end_residency(&mut self) -> Result<()> {
            self.step("end_residency")
        }

        fn release_plugins(&mut self) -> Result<()> {
            self.step("release_plugins")
        }

        fn stop_running(&mut self) -> Result<()> {
            self.step("stop_running")
        }

        fn remove_autostart(&mut self) -> Result<()> {
            self.step("remove_autostart")
        }

        fn remove_shell_entries(&mut self) -> Result<()> {
            self.step("remove_shell_entries")
        }

        fn remove_toast_identity(&mut self) -> Result<()> {
            self.step("remove_toast_identity")
        }

        fn remove_files(&mut self) -> Result<FileRemoval> {
            self.step("remove_files")?;
            Ok(self.files.unwrap_or(FileRemoval::Done))
        }

        fn remove_uninstall_entry(&mut self) -> Result<()> {
            self.step("remove_uninstall_entry")
        }
    }

    const ALL_STEPS: [&str; 8] = [
        "end_residency",
        "release_plugins",
        "stop_running",
        "remove_autostart",
        "remove_shell_entries",
        "remove_toast_identity",
        "remove_files",
        "remove_uninstall_entry",
    ];

    #[test]
    fn uninstall_ends_residency_first_and_drops_the_apps_entry_last() {
        let mut steps = RecordedSteps::default();

        run_uninstall(&mut steps).unwrap();

        assert_eq!(steps.calls, ALL_STEPS);
    }

    #[test]
    fn a_failed_file_removal_keeps_the_apps_entry() {
        let mut steps = RecordedSteps {
            failing: vec!["remove_files"],
            ..RecordedSteps::default()
        };

        assert!(run_uninstall(&mut steps).is_err());
        assert_eq!(steps.calls, ALL_STEPS[..7]);
    }

    #[test]
    fn a_deferred_file_removal_leaves_the_apps_entry_to_the_deferred_script() {
        let mut steps = RecordedSteps {
            files: Some(FileRemoval::Deferred),
            ..RecordedSteps::default()
        };

        run_uninstall(&mut steps).unwrap();

        assert_eq!(steps.calls, ALL_STEPS[..7]);
    }

    #[test]
    fn residency_and_plugin_release_failures_are_reported_after_the_uninstall_finishes() {
        for step in ["end_residency", "release_plugins"] {
            let mut steps = RecordedSteps {
                failing: vec![step],
                ..RecordedSteps::default()
            };

            let error = run_uninstall(&mut steps).unwrap_err();

            assert_eq!(steps.calls, ALL_STEPS, "{step}");
            assert!(
                format!("{error:#}").contains(&format!("{step} failed")),
                "{error:#}"
            );
        }
    }

    #[test]
    fn deferred_removal_logs_and_drops_the_apps_entry_only_once_the_root_is_gone() {
        let script = deferred_removal_script(
            r"C:\Users\x y\AppData\Local\Programs\qol-tray",
            r"C:\Users\x y\AppData\Local\Temp\qol-tray-uninstall.log",
            r"C:\Windows\System32\reg.exe",
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall\qol-tray",
        )
        .unwrap();

        assert_eq!(
            script,
            concat!(
                r#"/d /v:off /c ping -n 3 127.0.0.1 >nul"#,
                r#" & echo removing "C:\Users\x y\AppData\Local\Programs\qol-tray">"C:\Users\x y\AppData\Local\Temp\qol-tray-uninstall.log""#,
                r#" & rmdir /s /q "C:\Users\x y\AppData\Local\Programs\qol-tray" >>"C:\Users\x y\AppData\Local\Temp\qol-tray-uninstall.log" 2>&1"#,
                r#" & if not exist "C:\Users\x y\AppData\Local\Programs\qol-tray\" "C:\Windows\System32\reg.exe" delete "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\qol-tray" /f >>"C:\Users\x y\AppData\Local\Temp\qol-tray-uninstall.log" 2>&1"#,
            )
        );
    }

    #[test]
    fn deferred_removal_refuses_paths_cmd_cannot_quote() {
        let reg = r"C:\Windows\System32\reg.exe";
        let key = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\qol-tray";
        for root in [r"C:\a%b", r"C:\a!b", "C:\\a\"b", "C:\\a\nb"] {
            assert_eq!(
                deferred_removal_script(root, r"C:\t\x.log", reg, key),
                None,
                "{root:?}"
            );
            assert_eq!(
                deferred_removal_script(r"C:\ok", root, reg, key),
                None,
                "{root:?}"
            );
        }
    }

    const INSTALL_DIR: &str = r"C:\Users\x\AppData\Local\Programs\qol-tray\bin";
    const ROOT: &str = r"C:\Users\x\AppData\Local\Programs\qol-tray";
    const BINARY: &str = r"C:\Users\x\AppData\Local\Programs\qol-tray\bin\qol-tray.exe";

    #[test]
    fn uninstall_plan_deletes_the_root_or_defers_while_running_inside_it() {
        let cases = [
            (
                BINARY,
                r"C:\Users\x\Downloads\qol-tray-install.exe",
                Some(UninstallPlan::RemoveRoot(PathBuf::from(ROOT))),
            ),
            (
                BINARY,
                r"C:\Users\x\AppData\Local\Programs\qol-tray-old\bin\qol-tray-install.exe",
                Some(UninstallPlan::RemoveRoot(PathBuf::from(ROOT))),
            ),
            (
                BINARY,
                r"C:\Users\x\AppData\Local\Programs\qol-tray\bin\qol-tray-install.exe",
                Some(UninstallPlan::RemoveAfterExit(PathBuf::from(ROOT))),
            ),
            (
                BINARY,
                r"\\?\C:\USERS\X\AppData\Local\Programs\qol-tray\bin\qol-tray-install.exe",
                Some(UninstallPlan::RemoveAfterExit(PathBuf::from(ROOT))),
            ),
            (
                r"c:/users/x/appdata/local/programs/qol-tray/bin/qol-tray.exe",
                r"C:\Users\x\AppData\Local\Programs\qol-tray\uninstall.exe",
                Some(UninstallPlan::RemoveAfterExit(PathBuf::from(ROOT))),
            ),
            (
                r"C:\Users\x\Downloads\qol-tray.exe",
                r"C:\Users\x\Downloads\qol-tray-install.exe",
                None,
            ),
            (
                r"C:\Users\x\AppData\Local\Programs\qol-tray\bin\nested\qol-tray.exe",
                r"C:\Users\x\Downloads\qol-tray-install.exe",
                None,
            ),
            (
                r"C:\Users\x\AppData\Local\Programs\qol-tray\bin-old\qol-tray.exe",
                r"C:\Users\x\Downloads\qol-tray-install.exe",
                None,
            ),
        ];
        for (binary, current_exe, expected) in cases {
            let planned = plan(
                Path::new(binary),
                Path::new(INSTALL_DIR),
                Path::new(current_exe),
            );
            assert_eq!(
                planned.ok(),
                expected,
                "binary: {binary} current_exe: {current_exe}"
            );
        }
    }

    #[test]
    fn uninstall_plan_never_targets_a_drive_root() {
        let planned = plan(
            Path::new(r"C:\bin\qol-tray.exe"),
            Path::new(r"C:\bin"),
            Path::new(r"C:\Users\x\Downloads\qol-tray-install.exe"),
        );
        assert!(planned.is_err(), "{planned:?}");
    }
}
