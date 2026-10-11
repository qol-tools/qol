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

pub(crate) const LAUNCHER_FOLDER: &str = "QoL";

pub(crate) struct Shortcut<'a> {
    pub(crate) target: &'a Path,
    pub(crate) arguments: &'a str,
    pub(crate) icon: &'a Path,
    pub(crate) description: &'a str,
}

pub(crate) fn programs_dir() -> Result<PathBuf> {
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

pub(crate) fn write(link: &Path, shortcut: &Shortcut<'_>) -> Result<()> {
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
        shell_link.SetArguments(&HSTRING::from(shortcut.arguments))?;
        shell_link.SetWorkingDirectory(&HSTRING::from(working_dir))?;
        shell_link.SetDescription(&HSTRING::from(shortcut.description))?;
        shell_link.SetIconLocation(&HSTRING::from(shortcut.icon), 0)?;
    }
    let file: IPersistFile = shell_link.cast()?;
    unsafe { file.Save(&HSTRING::from(link), true) }?;
    Ok(())
}

pub(crate) fn ico_from_pngs(images: &[(u32, &[u8])]) -> Vec<u8> {
    const HEADER: usize = 6;
    const ENTRY: usize = 16;
    let mut icon = Vec::new();
    icon.extend(0u16.to_le_bytes());
    icon.extend(1u16.to_le_bytes());
    icon.extend((images.len() as u16).to_le_bytes());
    let mut offset = HEADER + ENTRY * images.len();
    for (size, png) in images {
        let dimension = if *size >= 256 { 0 } else { *size as u8 };
        icon.extend([dimension, dimension, 0, 0]);
        icon.extend(1u16.to_le_bytes());
        icon.extend(32u16.to_le_bytes());
        icon.extend((png.len() as u32).to_le_bytes());
        icon.extend((offset as u32).to_le_bytes());
        offset += png.len();
    }
    for (_, png) in images {
        icon.extend_from_slice(png);
    }
    icon
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
                arguments: "exec core settings",
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
        assert_eq!(parsed.arguments.as_deref(), Some("exec core settings"));
    }

    #[test]
    fn icon_directory_points_at_each_embedded_png() {
        let images: [(u32, &[u8]); 2] = [(64, b"first"), (256, b"second!")];
        let icon = ico_from_pngs(&images);
        let u16_at = |at: usize| u16::from_le_bytes([icon[at], icon[at + 1]]);
        let u32_at = |at: usize| u32::from_le_bytes(icon[at..at + 4].try_into().unwrap());
        assert_eq!((u16_at(0), u16_at(2), u16_at(4)), (0, 1, 2));
        for (index, (size, png)) in images.iter().enumerate() {
            let entry = 6 + index * 16;
            let expected_dimension = if *size >= 256 { 0 } else { *size as u8 };
            assert_eq!(icon[entry], expected_dimension);
            assert_eq!(u32_at(entry + 8) as usize, png.len());
            let offset = u32_at(entry + 12) as usize;
            assert_eq!(&icon[offset..offset + png.len()], *png);
        }
    }

    #[test]
    fn programs_folder_resolves() {
        assert!(programs_dir().unwrap().is_absolute());
    }
}
