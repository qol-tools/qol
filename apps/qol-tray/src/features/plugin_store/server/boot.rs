use serde::Serialize;

use qol_theme::{css, dark_accent_presets, tray_theme_presets};

#[derive(Serialize)]
struct AccentEntry {
    key: &'static str,
    label: &'static str,
    rgb: String,
    hover: String,
}

/// Sentinel the served `index.html` ships with; the index handler swaps it for the
/// real boot document. Kept here so the handler and the asset never drift apart.
pub(super) const BOOT_PLACEHOLDER: &str = "window.__QOL_BOOT__ = null; /* QOL_BOOT_INJECT */";

#[derive(Serialize)]
struct AccentBoot {
    palette: Vec<AccentEntry>,
    #[serde(rename = "defaultKey")]
    default_key: String,
    #[serde(rename = "selectedKey")]
    selected_key: Option<String>,
}

fn accent_palette() -> Vec<AccentEntry> {
    dark_accent_presets()
        .iter()
        .map(|preset| AccentEntry {
            key: preset.key,
            label: preset.label,
            rgb: css::rgb_string(preset.rgb),
            hover: css::hex_string(preset.hover),
        })
        .collect()
}

#[cfg(target_os = "macos")]
const PLATFORM_LABEL: &str = "macOS";
#[cfg(target_os = "linux")]
const PLATFORM_LABEL: &str = "Linux";
#[cfg(target_os = "windows")]
const PLATFORM_LABEL: &str = "Windows";

#[derive(Serialize)]
struct ThemeEntry {
    key: &'static str,
    label: &'static str,
    #[serde(rename = "accentKey")]
    accent_key: &'static str,
    #[serde(rename = "identityKey")]
    identity_key: &'static str,
}

#[derive(Serialize)]
struct ThemeBoot {
    themes: Vec<ThemeEntry>,
    #[serde(rename = "defaultKey")]
    default_key: String,
    #[serde(rename = "selectedKey")]
    selected_key: Option<String>,
}

fn theme_boot() -> ThemeBoot {
    ThemeBoot {
        themes: tray_theme_presets()
            .iter()
            .map(|preset| ThemeEntry {
                key: preset.key,
                label: preset.label,
                accent_key: preset.accent_key,
                identity_key: preset.identity.key,
            })
            .collect(),
        default_key: crate::features::theme::current_theme_key(),
        selected_key: crate::features::theme::selected_theme_key().ok().flatten(),
    }
}

#[derive(Serialize)]
struct DeviceBoot {
    name: String,
    platform: &'static str,
}

fn device_boot() -> DeviceBoot {
    DeviceBoot {
        name: gethostname::gethostname().to_string_lossy().into_owned(),
        platform: PLATFORM_LABEL,
    }
}

#[derive(Serialize)]
struct BootState {
    dev: bool,
    accent: AccentBoot,
    theme: ThemeBoot,
    device: DeviceBoot,
}

pub(super) fn boot_json(dev: bool) -> String {
    let state = BootState {
        dev,
        accent: AccentBoot {
            palette: accent_palette(),
            default_key: crate::features::theme::resolved_accent_key(),
            selected_key: crate::features::theme::selected_accent_key().ok().flatten(),
        },
        theme: theme_boot(),
        device: device_boot(),
    };
    serde_json::to_string(&state).unwrap_or_else(|_| "null".to_string())
}

pub(crate) async fn current_dev() -> bool {
    let mode_is_dev = tokio::task::spawn_blocking(|| {
        crate::mode::ModeConfig::load().unwrap_or_default().is_dev()
    })
    .await
    .unwrap_or(false);
    cfg!(feature = "dev") && mode_is_dev
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb_string(color: u32) -> String {
        let red = (color >> 16) & 0xff;
        let green = (color >> 8) & 0xff;
        let blue = color & 0xff;
        format!("{red}, {green}, {blue}")
    }

    fn hex_string(color: u32) -> String {
        format!("#{:06x}", color & 0x00ff_ffff)
    }

    fn default_key(json: &str) -> String {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        v["accent"]["defaultKey"].as_str().unwrap().to_string()
    }

    #[test]
    fn auto_accent_follows_theme_in_both_modes() {
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert_eq!(default_key(&boot_json(true)), "amber");
        assert_eq!(default_key(&boot_json(false)), "amber");

        crate::features::theme::save_selected_theme_key("midnight").unwrap();
        assert_eq!(default_key(&boot_json(true)), "violet");
        assert_eq!(default_key(&boot_json(false)), "violet");
    }

    #[test]
    fn boot_json_prefers_saved_accent_over_theme_default() {
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        crate::features::theme::save_selected_accent_key("blue").unwrap();

        let dev: serde_json::Value = serde_json::from_str(&boot_json(true)).unwrap();
        let prod: serde_json::Value = serde_json::from_str(&boot_json(false)).unwrap();
        assert_eq!(dev["accent"]["defaultKey"], "blue");
        assert_eq!(dev["accent"]["selectedKey"], "blue");
        assert_eq!(prod["accent"]["defaultKey"], "blue");
        assert_eq!(prod["accent"]["selectedKey"], "blue");
    }

    #[test]
    fn boot_json_marks_auto_accent_with_null_selected_key() {
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        let dev: serde_json::Value = serde_json::from_str(&boot_json(true)).unwrap();
        assert_eq!(dev["accent"]["defaultKey"], "amber");
        assert_eq!(dev["accent"]["selectedKey"], serde_json::Value::Null);
    }

    #[test]
    fn boot_json_carries_full_palette_and_dev_flag() {
        let v: serde_json::Value = serde_json::from_str(&boot_json(true)).unwrap();
        assert_eq!(v["dev"], true);
        let palette = v["accent"]["palette"].as_array().unwrap();
        assert_eq!(palette.len(), dark_accent_presets().len());
        for entry in palette {
            assert!(entry["key"].is_string());
            assert!(entry["rgb"].is_string());
            assert!(entry["hover"].is_string());
        }
    }

    #[test]
    fn boot_palette_is_serialized_from_qol_theme() {
        let v: serde_json::Value = serde_json::from_str(&boot_json(false)).unwrap();
        let amber = v["accent"]["palette"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["key"] == "amber")
            .expect("amber preset present");
        let preset = qol_theme::dark_accent_preset("amber").unwrap();
        assert_eq!(amber["rgb"], rgb_string(preset.rgb));
        assert_eq!(amber["hover"], hex_string(preset.hover));
    }

    #[test]
    fn boot_json_carries_device_name_and_platform_label() {
        let v: serde_json::Value = serde_json::from_str(&boot_json(false)).unwrap();
        assert!(v["device"]["name"].is_string(), "device.name present");
        let platform = v["device"]["platform"].as_str().unwrap();
        assert!(
            ["macOS", "Linux", "Windows"].contains(&platform),
            "platform label is a known OS, got {platform}"
        );
    }

    #[test]
    fn boot_json_carries_theme_selection_and_palette() {
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        crate::features::theme::save_selected_theme_key("graphite").unwrap();

        let v: serde_json::Value = serde_json::from_str(&boot_json(false)).unwrap();
        assert_eq!(v["theme"]["defaultKey"], "graphite");
        assert_eq!(v["theme"]["selectedKey"], "graphite");
        let themes = v["theme"]["themes"].as_array().unwrap();
        assert_eq!(themes.len(), qol_theme::tray_theme_presets().len());
        assert_eq!(themes[0]["key"], "slate");
        assert_eq!(themes[0]["label"], "Slate");
        assert_eq!(themes[0]["accentKey"], "amber");
        assert_eq!(themes[0]["identityKey"], "retro");
    }

    #[test]
    fn boot_json_marks_auto_theme_with_default_key() {
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        let v: serde_json::Value = serde_json::from_str(&boot_json(false)).unwrap();
        assert_eq!(v["theme"]["defaultKey"], qol_theme::DEFAULT_TRAY_THEME_KEY);
        assert_eq!(v["theme"]["selectedKey"], serde_json::Value::Null);
    }

    #[test]
    fn served_index_still_contains_boot_placeholder() {
        let index = super::super::assets::index_html_for_test();
        assert!(
            index.contains(BOOT_PLACEHOLDER),
            "index.html lost the boot placeholder; the boot document would never be injected"
        );
    }

    #[test]
    fn served_index_does_not_use_world_settings_as_accent_source() {
        let index = super::super::assets::index_html_for_test();
        assert!(
            !index.contains("qol-world-settings"),
            "accent bootstrapping must use backend boot state, not world-settings localStorage"
        );
        assert!(
            !index.contains("ws.accent"),
            "accent bootstrapping must not prefer the legacy world-settings accent"
        );
    }
}
