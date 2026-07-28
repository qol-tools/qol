use std::path::Path;

use crate::discovery::search;

mod platform;

pub fn open_path(path: &Path) -> bool {
    qol_apps::desktop_integration::open_with_default_app(path).is_ok()
}

pub fn launch_item(item: &search::ResultItem<'_>) -> bool {
    match item {
        search::ResultItem::App(entry) => {
            eprintln!("[launch] app: {:?} exec: {:?}", entry.name, entry.exec);
            platform::launch_app(&entry.path, &entry.exec)
        }
        search::ResultItem::File(entry) => open_path(&entry.path),
    }
}
