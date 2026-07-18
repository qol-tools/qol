mod actions;
mod app;
mod capture;
mod config;
mod daemon;
mod discovery;
mod picker;
mod preview_plane;
mod rendering;
mod shared;

use crate::config::{load_alt_tab_config, PLUGIN_ID};
use std::sync::mpsc;

type PreviewMap = std::collections::HashMap<u32, std::sync::Arc<gpui::RenderImage>>;
type LiveFrameMap = std::collections::HashMap<u32, capture::LiveFrame>;
type IconMap = std::collections::HashMap<String, std::sync::Arc<gpui::RenderImage>>;
type SharedIconCache = std::sync::Arc<std::sync::Mutex<IconMap>>;
type PickerWindowState =
    std::rc::Rc<std::cell::RefCell<qol_gpui::window::ActiveWindows<app::AltTabApp>>>;

fn maybe_open_settings(args: &[String]) -> bool {
    if !args.iter().any(|arg| arg == "--settings") {
        return false;
    }
    open_settings_page();
    true
}

pub(crate) fn open_settings_page() {
    let settings_url = qol_conventions::settings_url(PLUGIN_ID);
    if let Err(error) = qol_apps::desktop_integration::open_with_default_app(&settings_url) {
        eprintln!("Failed to open settings page: {}", error);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if maybe_open_settings(&args) {
        return;
    }

    let is_show = args.iter().any(|a| a == "--show");
    let is_show_reverse = args.iter().any(|a| a == "--show-reverse");
    let is_kill = args.iter().any(|a| a == "--kill");

    if is_kill {
        daemon::send_kill();
        return;
    }

    if is_show_reverse && daemon::send_show_reverse() {
        return;
    }
    if is_show && daemon::send_show() {
        return;
    }

    let config = load_alt_tab_config();
    let (tx, rx) = mpsc::channel();

    if !daemon::start_listener(tx) {
        if is_show_reverse {
            daemon::send_show_reverse();
        } else if is_show {
            daemon::send_show();
        }
        return;
    }

    picker::run::run_app(config, rx, is_show);
    daemon::cleanup();
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
    }
}
