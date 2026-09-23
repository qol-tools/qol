#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback as imp;
#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

pub fn is_modifier_held() -> bool {
    imp::is_modifier_held()
}

pub fn is_shift_held() -> bool {
    imp::is_shift_held()
}

pub fn is_escape_held() -> bool {
    imp::is_escape_held()
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

pub fn process_focus_truth() -> Option<bool> {
    should_poll_focus().then(has_process_focus)
}

#[cfg(target_os = "macos")]
pub fn run_on_main(task: Box<dyn FnOnce() + Send + 'static>) {
    imp::run_on_main(task)
}

pub fn start_window_move(window: &mut gpui::Window) {
    imp::start_window_move(window);
}

pub fn square_window_corners(window: &mut gpui::Window) {
    imp::square_window_corners(window);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReassertStep {
    Settled,
    Reassert,
    Stop,
}

pub fn spawn_reassert_driver<F, G>(
    gen: &'static std::sync::atomic::AtomicU64,
    commit_gen: u64,
    delays_ms: &[u64],
    mut poll: F,
    mut reassert: G,
) where
    F: FnMut() -> ReassertStep + Send + 'static,
    G: FnMut() + Send + 'static,
{
    let delays = delays_ms.to_vec();
    std::thread::spawn(move || {
        for delay_ms in delays {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            if gen.load(std::sync::atomic::Ordering::SeqCst) != commit_gen {
                return;
            }
            match poll() {
                ReassertStep::Settled => continue,
                ReassertStep::Stop => return,
                ReassertStep::Reassert => {}
            }
            if gen.load(std::sync::atomic::Ordering::SeqCst) != commit_gen {
                return;
            }
            reassert();
        }
    });
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskbarIconSource {
    DesktopEntry { icon_id: &'static str },
    WindowClassResource,
    HostProcess,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SettingsSurfaceTaskbarIdentity {
    pub app_id: &'static str,
    pub display_name: &'static str,
    pub icon: TaskbarIconSource,
}

pub fn settings_surface_taskbar_identity() -> SettingsSurfaceTaskbarIdentity {
    imp::settings_surface_taskbar_identity()
}

pub fn apply_settings_surface_identity(window: &mut gpui::Window) {
    imp::apply_settings_surface_identity(window);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_surface_taskbar_identity_matches_the_shared_constants() {
        let identity = settings_surface_taskbar_identity();
        assert_eq!(identity.app_id, qol_conventions::SETTINGS_SURFACE_APP_ID);
        assert_eq!(
            identity.display_name,
            qol_conventions::SETTINGS_SURFACE_DISPLAY_NAME
        );
    }
}
