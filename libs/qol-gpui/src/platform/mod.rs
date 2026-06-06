#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use fallback as imp;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;

pub fn is_modifier_held() -> bool {
    imp::is_modifier_held()
}

pub fn is_shift_held() -> bool {
    imp::is_shift_held()
}

pub fn set_accessory_policy() {
    imp::set_accessory_policy()
}

pub fn ghost_window_kind() -> gpui::WindowKind {
    imp::ghost_window_kind()
}

pub fn ghost_window_decorations(transparent: bool) -> gpui::WindowDecorations {
    imp::ghost_window_decorations(transparent)
}

pub fn adjust_ghost_bounds(bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
    imp::adjust_ghost_bounds(bounds)
}

pub fn should_poll_focus() -> bool {
    imp::should_poll_focus()
}

pub fn has_process_focus() -> bool {
    imp::has_process_focus()
}

pub fn spawn_reassert_driver<F, G>(
    gen: &'static std::sync::atomic::AtomicU64,
    commit_gen: u64,
    steps_ms: &[u64],
    mut is_active: F,
    mut reassert: G,
) where
    F: FnMut() -> bool + Send + 'static,
    G: FnMut() + Send + 'static,
{
    let steps = steps_ms.to_vec();
    std::thread::spawn(move || {
        for step_ms in steps {
            std::thread::sleep(std::time::Duration::from_millis(step_ms));
            if gen.load(std::sync::atomic::Ordering::SeqCst) != commit_gen {
                return;
            }
            if is_active() {
                continue;
            }
            reassert();
        }
    });
}
