use crate::plugins::SharedPluginManager;
use qol_sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Look {
    native: String,
    accent: String,
}

impl Look {
    fn current() -> Self {
        Self {
            native: super::current_native_theme_key(),
            accent: super::native_accent_wire(),
        }
    }
}

static SHOWN: Mutex<Option<Look>> = Mutex::new(None);

pub fn remember_shown_look() {
    *SHOWN.lock() = Some(Look::current());
}

pub fn show_current_look(plugin_manager: &SharedPluginManager) {
    let Some(look) = newly_shown(&SHOWN, Look::current()) else {
        return;
    };
    if !crate::settings_surface::apply_theme(&look.native, &look.accent) {
        crate::settings_surface::stop();
        crate::settings_surface::prewarm();
    }
    let reached = plugin_manager
        .lock()
        .broadcast_theme_to_running_gpui_daemons(&look.native, &look.accent);
    if !reached.is_empty() {
        log::info!(
            "Broadcast theme to {} GPUI daemon(s) after theme change",
            reached.len()
        );
    }
    crate::features::launcher_apps::trigger_full_sync_with_manager(plugin_manager);
}

fn newly_shown(shown: &Mutex<Option<Look>>, look: Look) -> Option<Look> {
    let mut shown = shown.lock();
    if shown.as_ref() == Some(&look) {
        return None;
    }
    *shown = Some(look.clone());
    Some(look)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(native: &str, accent: &str) -> Look {
        Look {
            native: native.to_string(),
            accent: accent.to_string(),
        }
    }

    #[test]
    fn only_a_look_the_surfaces_do_not_show_yet_is_pushed() {
        let shown = Mutex::new(None);

        assert_eq!(
            newly_shown(&shown, look("bone", "amber")),
            Some(look("bone", "amber"))
        );
        assert_eq!(newly_shown(&shown, look("bone", "amber")), None);
        assert_eq!(
            newly_shown(&shown, look("bone", "blue")),
            Some(look("bone", "blue"))
        );
        assert_eq!(
            newly_shown(&shown, look("slate", "blue")),
            Some(look("slate", "blue"))
        );
        assert_eq!(newly_shown(&shown, look("slate", "blue")), None);
    }
}
