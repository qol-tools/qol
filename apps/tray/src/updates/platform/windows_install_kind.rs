use std::path::Path;

use qol_platform::windows_path::path_key;

use super::InstallKind;

pub(super) fn install_kind_for(executable: &Path, install_dir: Option<&Path>) -> InstallKind {
    let executable = path_key(executable);
    let in_install_dir = install_dir.is_some_and(|dir| {
        executable
            .rsplit_once('\\')
            .is_some_and(|(parent, _)| parent == path_key(dir))
    });
    InstallKind::for_path(&executable.replace('\\', "/"), None, in_install_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_kind_follows_the_executable_location() {
        let install_dir = Path::new(r"C:\Users\x\AppData\Local\Programs\qol-tray\bin");
        let cases = [
            (
                r"C:\Users\x\AppData\Local\Programs\qol-tray\bin\qol-tray.exe",
                InstallKind::UserLocal,
            ),
            (
                r"\\?\C:\USERS\X\AppData\Local\Programs\qol-tray\bin\qol-tray.exe",
                InstallKind::UserLocal,
            ),
            (
                r"C:/Users/x/AppData/Local/Programs/qol-tray/bin/qol-tray.exe",
                InstallKind::UserLocal,
            ),
            (
                r"C:\Users\x\repos\qol\target\release\qol-tray.exe",
                InstallKind::Development,
            ),
            (
                r"C:\Users\x\repos\qol\target\debug\qol-tray.exe",
                InstallKind::Development,
            ),
            (
                r"C:\Users\x\Downloads\qol-tray-windows-x86_64.exe",
                InstallKind::SystemWide,
            ),
            (
                r"C:\Users\x\AppData\Local\Programs\qol-tray\bin\nested\qol-tray.exe",
                InstallKind::SystemWide,
            ),
            (
                r"C:\Users\x\AppData\Local\Programs\qol-tray\bin-old\qol-tray.exe",
                InstallKind::SystemWide,
            ),
        ];
        for (executable, expected) in cases {
            assert_eq!(
                install_kind_for(Path::new(executable), Some(install_dir)),
                expected,
                "{executable}"
            );
        }
        assert_eq!(
            install_kind_for(
                Path::new(r"C:\Users\x\AppData\Local\Programs\qol-tray\bin\qol-tray.exe"),
                None
            ),
            InstallKind::SystemWide
        );
    }
}
