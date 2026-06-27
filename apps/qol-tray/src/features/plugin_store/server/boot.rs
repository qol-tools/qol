use serde::Serialize;

#[derive(Serialize)]
struct AccentEntry {
    key: &'static str,
    label: &'static str,
    rgb: &'static str,
    hover: &'static str,
}

const PALETTE: &[AccentEntry] = &[
    AccentEntry {
        key: "amber",
        label: "Amber",
        rgb: "255, 180, 84",
        hover: "#ffc77a",
    },
    AccentEntry {
        key: "green",
        label: "Green",
        rgb: "70, 224, 138",
        hover: "#7ff0ab",
    },
    AccentEntry {
        key: "cyan",
        label: "Cyan",
        rgb: "86, 214, 224",
        hover: "#8fe8f0",
    },
    AccentEntry {
        key: "magenta",
        label: "Magenta",
        rgb: "232, 121, 198",
        hover: "#f49ad6",
    },
    AccentEntry {
        key: "blue",
        label: "Blue",
        rgb: "74, 158, 255",
        hover: "#68b0ff",
    },
];

const PROD_DEFAULT: &str = "amber";
const DEV_DEFAULT: &str = "green";

/// Sentinel the served `index.html` ships with; the index handler swaps it for the
/// real boot document. Kept here so the handler and the asset never drift apart.
pub(super) const BOOT_PLACEHOLDER: &str = "window.__QOL_BOOT__ = null; /* QOL_BOOT_INJECT */";

#[derive(Serialize)]
struct AccentBoot {
    palette: &'static [AccentEntry],
    #[serde(rename = "defaultKey")]
    default_key: &'static str,
}

#[cfg(target_os = "macos")]
const PLATFORM_LABEL: &str = "macOS";
#[cfg(target_os = "linux")]
const PLATFORM_LABEL: &str = "Linux";
#[cfg(target_os = "windows")]
const PLATFORM_LABEL: &str = "Windows";

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
    device: DeviceBoot,
}

pub(super) fn boot_json(dev: bool) -> String {
    let state = BootState {
        dev,
        accent: AccentBoot {
            palette: PALETTE,
            default_key: if dev { DEV_DEFAULT } else { PROD_DEFAULT },
        },
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

    fn default_key(json: &str) -> String {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        v["accent"]["defaultKey"].as_str().unwrap().to_string()
    }

    #[test]
    fn dev_resolves_default_to_green() {
        assert_eq!(default_key(&boot_json(true)), "green");
    }

    #[test]
    fn prod_resolves_default_to_amber() {
        assert_eq!(default_key(&boot_json(false)), "amber");
    }

    #[test]
    fn boot_json_carries_full_palette_and_dev_flag() {
        let v: serde_json::Value = serde_json::from_str(&boot_json(true)).unwrap();
        assert_eq!(v["dev"], true);
        let palette = v["accent"]["palette"].as_array().unwrap();
        assert_eq!(palette.len(), PALETTE.len());
        for entry in palette {
            assert!(entry["key"].is_string());
            assert!(entry["rgb"].is_string());
            assert!(entry["hover"].is_string());
        }
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
    fn served_index_still_contains_boot_placeholder() {
        let index = super::super::assets::index_html_for_test();
        assert!(
            index.contains(BOOT_PLACEHOLDER),
            "index.html lost the boot placeholder; the boot document would never be injected"
        );
    }
}
