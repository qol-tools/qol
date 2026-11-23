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
    let pushed = push_new_look(&SHOWN, Look::current, |look| {
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
    });
    if pushed {
        crate::features::launcher_apps::trigger_full_sync_with_manager(plugin_manager);
    }
}

fn push_new_look(
    shown: &Mutex<Option<Look>>,
    current: impl FnOnce() -> Look,
    push: impl FnOnce(&Look),
) -> bool {
    let mut shown = shown.lock();
    let look = current();
    if shown.as_ref() == Some(&look) {
        return false;
    }
    push(&look);
    *shown = Some(look);
    true
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
        let mut pushed = Vec::new();
        let cases = [
            ("bone", "amber", true),
            ("bone", "amber", false),
            ("bone", "blue", true),
            ("slate", "blue", true),
            ("slate", "blue", false),
        ];

        for (native, accent, expected) in cases {
            let was_pushed = push_new_look(
                &shown,
                || look(native, accent),
                |new_look| pushed.push(new_look.clone()),
            );
            assert_eq!(was_pushed, expected, "{native} {accent}");
        }

        assert_eq!(
            pushed,
            [
                look("bone", "amber"),
                look("bone", "blue"),
                look("slate", "blue")
            ]
        );
    }
}
