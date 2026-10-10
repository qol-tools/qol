use std::path::PathBuf;

use super::directory::{inspect_directory, inspect_paths};
use super::PlatformInspection;

const RECYCLE_BIN: &str = "$Recycle.Bin";

pub(crate) fn inspect() -> PlatformInspection {
    let inventory_roots = inspect_paths(
        qol_apps::start_menu::start_menu_roots()
            .into_iter()
            .map(|root| root.path),
    );
    let drive = system_drive();
    PlatformInspection {
        name: "Windows",
        supported: true,
        inventory_roots,
        trash: drive
            .as_ref()
            .map(|drive| inspect_directory(drive.join(RECYCLE_BIN))),
        trash_creation_anchor: drive.map(inspect_directory),
    }
}

fn system_drive() -> Option<PathBuf> {
    std::env::var_os("SystemDrive")
        .filter(|drive| !drive.is_empty())
        .map(|drive| {
            let mut root = drive;
            root.push("\\");
            PathBuf::from(root)
        })
}
