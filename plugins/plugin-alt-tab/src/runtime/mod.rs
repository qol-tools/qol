pub(crate) mod daemon;

use std::sync::mpsc;

use crate::config::{load_alt_tab_config, PLUGIN_ID};

fn maybe_open_settings(args: &[String]) -> bool {
    if !args.iter().any(|argument| argument == "--settings") {
        return false;
    }
    open_settings_page();
    true
}

pub(crate) fn open_settings_page() {
    if let Err(error) = qol_apps::desktop_integration::open_plugin_settings(PLUGIN_ID) {
        eprintln!("Failed to open settings page: {error}");
    }
}

pub(crate) fn run(args: Vec<String>) {
    if maybe_open_settings(&args) {
        return;
    }

    let is_show = args.iter().any(|argument| argument == "--show");
    let is_show_reverse = args.iter().any(|argument| argument == "--show-reverse");
    let is_kill = args.iter().any(|argument| argument == "--kill");

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

    crate::picker::run::run_app(config, rx, is_show);
    daemon::cleanup();
}
