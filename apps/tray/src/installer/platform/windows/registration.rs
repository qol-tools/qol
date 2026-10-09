use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use super::registry::{self, Value};
use super::shortcut::{self, Shortcut};

const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\qol-tray";
const PUBLISHER: &str = "qol-tools";
const ABOUT_URL: &str = "https://github.com/qol-tools/qol";
const ICON_FILE: &str = "qol-tray.ico";
const ICONS: [(u32, &[u8]); 3] = [
    (64, include_bytes!("../../../../assets/icons/64.png")),
    (128, include_bytes!("../../../../assets/icons/128.png")),
    (256, include_bytes!("../../../../assets/icons/256.png")),
];

pub(super) struct Layout {
    pub(super) binary: PathBuf,
    pub(super) uninstaller: PathBuf,
    pub(super) icon: PathBuf,
    pub(super) shortcut: PathBuf,
}

impl Layout {
    pub(super) fn for_binary(binary: &Path) -> Result<Self> {
        let dir = binary
            .parent()
            .context("Installed binary has no parent directory")?;
        Ok(Self {
            binary: binary.to_path_buf(),
            uninstaller: dir.join(uninstaller_filename()),
            icon: dir.join(ICON_FILE),
            shortcut: shortcut::programs_dir()?
                .join(format!("{}.lnk", qol_conventions::TRAY_DISPLAY_NAME)),
        })
    }

    fn install_location(&self) -> &Path {
        let bin = self.binary.parent().unwrap_or(&self.binary);
        bin.parent().unwrap_or(bin)
    }
}

pub(super) fn uninstaller_filename() -> String {
    format!(
        "{}.exe",
        qol_conventions::artifact::TRAY_INSTALLER_BINARY_NAME
    )
}

pub(super) fn register(layout: &Layout) -> Result<()> {
    write_icon(&layout.icon)?;
    write_shortcut(layout)?;
    write_uninstall_entry(layout)
}

pub(super) fn refresh(layout: &Layout) -> Result<()> {
    write_icon(&layout.icon)?;
    if !layout.shortcut.exists() {
        write_shortcut(layout)?;
    }
    write_uninstall_entry(layout)
}

pub(super) fn remove(layout: &Layout) -> Result<()> {
    registry::delete_key(UNINSTALL_KEY)?;
    remove_file_if_present(&layout.shortcut)?;
    remove_file_if_present(&layout.icon)
}

pub(super) fn remove_file_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("Failed to remove {}", path.display()))
        }
        _ => Ok(()),
    }
}

fn write_shortcut(layout: &Layout) -> Result<()> {
    shortcut::write(
        &layout.shortcut,
        &Shortcut {
            target: &layout.binary,
            icon: &layout.icon,
            description: qol_conventions::TRAY_DISPLAY_NAME,
        },
    )
}

fn write_icon(path: &Path) -> Result<()> {
    let icon = ico_from_pngs(&ICONS);
    if std::fs::read(path).is_ok_and(|existing| existing == icon) {
        return Ok(());
    }
    std::fs::write(path, icon).with_context(|| format!("Failed to write {}", path.display()))
}

fn write_uninstall_entry(layout: &Layout) -> Result<()> {
    if !layout.uninstaller.is_file() {
        log::warn!(
            "skipping the Apps & Features entry: {} is missing",
            layout.uninstaller.display()
        );
        return Ok(());
    }
    for (name, value) in uninstall_entry(layout, env!("CARGO_PKG_VERSION")) {
        registry::set(UNINSTALL_KEY, name, &value)?;
    }
    Ok(())
}

fn uninstall_entry(layout: &Layout, version: &str) -> Vec<(&'static str, Value)> {
    let uninstall = format!("\"{}\" uninstall", layout.uninstaller.display());
    vec![
        (
            "DisplayName",
            Value::Text(qol_conventions::TRAY_DISPLAY_NAME.to_string()),
        ),
        (
            "DisplayIcon",
            Value::Text(layout.icon.display().to_string()),
        ),
        ("DisplayVersion", Value::Text(version.to_string())),
        ("Publisher", Value::Text(PUBLISHER.to_string())),
        ("URLInfoAbout", Value::Text(ABOUT_URL.to_string())),
        (
            "InstallLocation",
            Value::Text(layout.install_location().display().to_string()),
        ),
        ("UninstallString", Value::Text(uninstall.clone())),
        ("QuietUninstallString", Value::Text(uninstall)),
        ("NoModify", Value::Number(1)),
        ("NoRepair", Value::Number(1)),
    ]
}

fn ico_from_pngs(images: &[(u32, &[u8])]) -> Vec<u8> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        let bin = PathBuf::from(r"C:\Users\x y\AppData\Local\Programs\qol-tray\bin");
        Layout {
            binary: bin.join("qol-tray.exe"),
            uninstaller: bin.join("qol-tray-install.exe"),
            icon: bin.join(ICON_FILE),
            shortcut: PathBuf::from(r"C:\Start Menu\Programs\QoL Tray.lnk"),
        }
    }

    #[test]
    fn uninstall_entry_names_the_install_and_its_uninstall_command() {
        let entry = uninstall_entry(&layout(), "3.41.0");
        let text = |name: &str| {
            entry.iter().find_map(|(key, value)| match value {
                Value::Text(text) if *key == name => Some(text.as_str()),
                _ => None,
            })
        };
        let cases = [
            ("DisplayName", "QoL Tray"),
            (
                "DisplayIcon",
                r"C:\Users\x y\AppData\Local\Programs\qol-tray\bin\qol-tray.ico",
            ),
            ("DisplayVersion", "3.41.0"),
            ("Publisher", PUBLISHER),
            (
                "InstallLocation",
                r"C:\Users\x y\AppData\Local\Programs\qol-tray",
            ),
            (
                "UninstallString",
                r#""C:\Users\x y\AppData\Local\Programs\qol-tray\bin\qol-tray-install.exe" uninstall"#,
            ),
        ];
        for (name, expected) in cases {
            assert_eq!(text(name), Some(expected), "{name}");
        }
        for name in ["NoModify", "NoRepair"] {
            assert!(entry.contains(&(name, Value::Number(1))), "{name}");
        }
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
    fn shipped_icon_sizes_are_png_images() {
        for (size, png) in ICONS {
            assert!(png.starts_with(b"\x89PNG"), "{size}");
        }
    }
}
