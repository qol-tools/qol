pub(crate) mod config;
pub(crate) mod daemon;
pub(crate) mod remap;

pub(crate) fn run() {
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

    let (tx, rx) = std::sync::mpsc::channel();
    let Some((mut current_key_rules, state)) =
        start_services_if_singleton(daemon::start_listener(tx), || {
            let app_tracker = super::app_tracker::AppTracker::start();
            let current_key_rules = resolved.key_rules.clone();
            let state = std::sync::Arc::new(super::tap::TapState::new(resolved, app_tracker));
            super::tap::start_tap(std::sync::Arc::clone(&state));
            (current_key_rules, state)
        })
    else {
        if daemon::send_reload() {
            eprintln!("[keyremap] another instance running, sent reload");
        }
        return;
    };

    eprintln!("[keyremap] daemon started");

    for command in rx {
        match command {
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
            daemon::Command::Settings => {
                if let Err(error) =
                    qol_apps::desktop_integration::open_plugin_settings(crate::cli::PLUGIN_ID)
                {
                    eprintln!("[keyremap] failed to open settings page: {error}");
                }
            }
        }
    }

    daemon::cleanup();
}

fn start_services_if_singleton<T>(is_singleton: bool, start: impl FnOnce() -> T) -> Option<T> {
    is_singleton.then(start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_start_only_when_singleton() {
        let mut started = false;
        let result = start_services_if_singleton(false, || {
            started = true;
            0
        });
        assert!(result.is_none());
        assert!(!started);

        let result = start_services_if_singleton(true, || {
            started = true;
            42
        });
        assert_eq!(result, Some(42));
        assert!(started);
    }
}
