use crate::RgbaImage;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use super::AppIconPlatform;

pub(super) struct Platform;

impl AppIconPlatform for Platform {
    fn icon_for_bundle_id(&self, bundle_id: &str, size: usize) -> Option<RgbaImage> {
        icon_for_bundle_id(bundle_id, size)
    }

    fn icon_for_pid(&self, pid: i32, size: usize) -> Option<RgbaImage> {
        icon_for_pid(pid, size)
    }

    fn app_display_name(&self, app_id: &str) -> Option<String> {
        app_display_name(app_id)
    }

    fn parent_pid(&self, _pid: i32) -> Option<i32> {
        None
    }

    fn process_start_time_us(&self, _pid: i32) -> Option<u64> {
        None
    }
}

fn icon_for_bundle_id(_bundle_id: &str, _size: usize) -> Option<RgbaImage> {
    None
}

fn icon_for_pid(_pid: i32, _size: usize) -> Option<RgbaImage> {
    None
}

fn app_display_name(app_id: &str) -> Option<String> {
    if app_id.is_empty() {
        return None;
    }
    desktop_names().get(&app_id.to_lowercase()).cloned()
}

static DESKTOP_NAMES: OnceLock<HashMap<String, String>> = OnceLock::new();

fn desktop_names() -> &'static HashMap<String, String> {
    DESKTOP_NAMES.get_or_init(build_desktop_index)
}

fn build_desktop_index() -> HashMap<String, String> {
    let mut index = HashMap::new();
    for dir in application_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(name) = desktop_entry_field(&content, "Name") else {
                continue;
            };
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                index
                    .entry(stem.to_lowercase())
                    .or_insert_with(|| name.clone());
            }
            if let Some(wm_class) = desktop_entry_field(&content, "StartupWMClass") {
                index.entry(wm_class.to_lowercase()).or_insert(name);
            }
        }
    }
    index
}

fn desktop_entry_field(content: &str, key: &str) -> Option<String> {
    let mut in_entry = false;
    for line in content.lines() {
        let line = line.trim();
        if let Some(section) = line.strip_prefix('[') {
            in_entry = section.starts_with("Desktop Entry");
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some(value) = line
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix('='))
        {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
    if let Some(data_home) = data_home {
        dirs.push(data_home.join("applications"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    for dir in data_dirs.split(':').filter(|s| !s.is_empty()) {
        dirs.push(PathBuf::from(dir).join("applications"));
    }
    if let Some(home) = home {
        dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
    }
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs
}

#[cfg(test)]
mod tests {
    use super::desktop_entry_field;

    #[test]
    fn desktop_entry_field_reads_plain_name_and_skips_localized_and_actions() {
        let content = "[Desktop Entry]\n\
            Name=DELTARUNE\n\
            Name[de]=DELTARUNE DE\n\
            StartupWMClass=steam_app_1671210\n\
            [Desktop Action new-window]\n\
            Name=New Window\n";
        let cases = [
            ("Name", Some("DELTARUNE".to_string())),
            ("StartupWMClass", Some("steam_app_1671210".to_string())),
            ("Icon", None),
        ];
        for (key, expected) in cases {
            assert_eq!(desktop_entry_field(content, key), expected, "key: {key}");
        }
    }
}
