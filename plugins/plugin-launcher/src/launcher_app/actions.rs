use super::search;
use crate::platform;

pub fn launch_item(item: &search::ResultItem<'_>) -> bool {
    match item {
        search::ResultItem::App(entry) => {
            eprintln!("[launch] app: {:?} exec: {:?}", entry.name, entry.exec);
            platform::launch_app(&entry.path, &entry.exec)
        }
        search::ResultItem::File(entry) => platform::open_path(&entry.path),
    }
}
