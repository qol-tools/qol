use anyhow::{anyhow, Context, Result};
use qol_platform::native::com::{Apartment, ComApartment};
use std::path::{Path, PathBuf};
use windows::core::{Interface, HSTRING};
use windows::Win32::System::Com::{
    CoCreateInstance, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Programs, IShellLinkW, SHGetKnownFolderPath, ShellLink, KF_FLAG_DEFAULT,
};

pub(super) struct Shortcut<'a> {
    pub(super) target: &'a Path,
    pub(super) icon: &'a Path,
    pub(super) description: &'a str,
}

pub(super) fn programs_dir() -> Result<PathBuf> {
    on_com_thread(|| {
        let raw = unsafe { SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None) }
            .context("Failed to resolve the Start menu Programs folder")?;
        let path = unsafe { raw.to_string() };
        unsafe { CoTaskMemFree(Some(raw.0.cast_const().cast())) };
        Ok(PathBuf::from(path.context(
            "Start menu Programs folder is not valid UTF-16",
        )?))
    })
}

pub(super) fn write(link: &Path, shortcut: &Shortcut<'_>) -> Result<()> {
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    on_com_thread(|| save(link, shortcut))
        .with_context(|| format!("Failed to write Start menu shortcut {}", link.display()))
}

fn save(link: &Path, shortcut: &Shortcut<'_>) -> Result<()> {
    let shell_link: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }?;
    let working_dir = shortcut
        .target
        .parent()
        .ok_or_else(|| anyhow!("shortcut target has no parent directory"))?;
    unsafe {
        shell_link.SetPath(&HSTRING::from(shortcut.target))?;
        shell_link.SetWorkingDirectory(&HSTRING::from(working_dir))?;
        shell_link.SetDescription(&HSTRING::from(shortcut.description))?;
        shell_link.SetIconLocation(&HSTRING::from(shortcut.icon), 0)?;
    }
    let file: IPersistFile = shell_link.cast()?;
    unsafe { file.Save(&HSTRING::from(link), true) }?;
    Ok(())
}

fn on_com_thread<T: Send>(work: impl FnOnce() -> Result<T> + Send) -> Result<T> {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let _com = ComApartment::enter(Apartment::SingleThreaded)
                    .context("Failed to initialize COM for the Start menu shortcut")?;
                work()
            })
            .join()
            .map_err(|_| anyhow!("Start menu shortcut worker panicked"))?
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_shortcut_points_at_the_target_with_its_icon() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("bin").join("qol-tray.exe");
        let icon = temp.path().join("bin").join("qol-tray.ico");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"MZ").unwrap();
        std::fs::write(&icon, b"ico").unwrap();
        let link = temp.path().join("Programs").join("QoL Tray.lnk");

        write(
            &link,
            &Shortcut {
                target: &target,
                icon: &icon,
                description: "QoL Tray",
            },
        )
        .unwrap();

        let parsed = qol_apps::shell_link::ShellLink::read(&link).expect("parse written link");
        let qol_apps::shell_link::LinkTarget::Path(resolved) =
            parsed.target(&link, |name| std::env::var(name).ok())
        else {
            panic!("written link has no file target: {parsed:?}");
        };
        let canonical = |path: &Path| std::fs::canonicalize(path).unwrap();
        assert_eq!(canonical(&resolved), canonical(&target));
        let icon_location = parsed.icon_location.expect("icon location");
        assert_eq!(canonical(Path::new(&icon_location)), canonical(&icon));
    }

    #[test]
    fn programs_folder_resolves() {
        assert!(programs_dir().unwrap().is_absolute());
    }
}
