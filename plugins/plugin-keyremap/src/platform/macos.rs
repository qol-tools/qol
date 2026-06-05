mod app_tracker;
mod tap;

use std::sync::Arc;

use crate::{config, daemon, remap};

pub(crate) fn run(args: Vec<String>) -> i32 {
    if args.iter().any(|a| a == "--kill") {
        if daemon::send_kill() {
            eprintln!("[keyremap] kill sent");
        } else {
            eprintln!("[keyremap] no daemon running");
        }
        return 0;
    }

    if args.iter().any(|a| a == "--reload") {
        if daemon::send_reload() {
            eprintln!("[keyremap] reload sent");
        } else {
            eprintln!("[keyremap] no daemon running");
        }
        return 0;
    }

    let raw_config = config::load_config();
    let resolved = remap::resolve(&raw_config);

    eprintln!(
        "[keyremap] loaded {} char rules, {} key rules, {} mouse rules, {} scroll rules, {} excluded apps",
        resolved.char_rules.len(),
        resolved.key_rules.len(),
        resolved.mouse_rules.len(),
        resolved.scroll_rules.len(),
        resolved.excluded_apps.len(),
    );

    let mut current_key_rules = resolved.key_rules.clone();
    let app_tracker = app_tracker::AppTracker::start();
    let state = Arc::new(tap::TapState::new(resolved, app_tracker));

    tap::start_tap(Arc::clone(&state));

    let (tx, rx) = std::sync::mpsc::channel();
    if !daemon::start_listener(tx) {
        if daemon::send_reload() {
            eprintln!("[keyremap] another instance running, sent reload");
        }
        return 0;
    }

    eprintln!("[keyremap] daemon started");

    for cmd in rx {
        match cmd {
            daemon::Command::Reload => {
                let new_raw = config::load_config();
                let new_resolved = remap::resolve(&new_raw);
                eprintln!(
                    "[keyremap] reloaded {} char rules, {} key rules, {} mouse rules, {} scroll rules",
                    new_resolved.char_rules.len(),
                    new_resolved.key_rules.len(),
                    new_resolved.mouse_rules.len(),
                    new_resolved.scroll_rules.len(),
                );
                for warning in remap::diff_key_rules(&current_key_rules, &new_resolved.key_rules) {
                    eprintln!("[keyremap] warning: {warning}");
                }
                current_key_rules = new_resolved.key_rules.clone();
                state.swap_config(new_resolved);
            }
            daemon::Command::Kill => {
                eprintln!("[keyremap] kill received, shutting down");
                break;
            }
        }
    }

    daemon::cleanup();
    0
}
