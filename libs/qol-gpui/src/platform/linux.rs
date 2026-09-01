use std::sync::{Mutex, OnceLock};
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::rust_connection::RustConnection;

fn keymap_conn() -> &'static Mutex<Option<RustConnection>> {
    static CONN: OnceLock<Mutex<Option<RustConnection>>> = OnceLock::new();
    CONN.get_or_init(|| Mutex::new(x11rb::connect(None).map(|(c, _)| c).ok()))
}

fn query_keymap_keys() -> Option<[u8; 32]> {
    let mut guard = keymap_conn().lock().ok()?;
    let keys = {
        let conn = guard.as_ref()?;
        conn.query_keymap().ok()?.reply().ok().map(|r| r.keys)
    };
    if keys.is_none() {
        *guard = x11rb::connect(None).map(|(c, _)| c).ok();
    }
    keys
}

pub fn is_modifier_held() -> bool {
    let Some(keys) = query_keymap_keys() else {
        return false;
    };
    keycode_held(&keys, 64) || keycode_held(&keys, 108)
}

pub fn is_shift_held() -> bool {
    let Some(keys) = query_keymap_keys() else {
        return false;
    };
    keycode_held(&keys, 50) || keycode_held(&keys, 62)
}

pub fn is_escape_held() -> bool {
    query_keymap_keys()
        .map(|keys| keycode_held(&keys, 9))
        .unwrap_or(false)
}

fn keycode_held(keys: &[u8; 32], keycode: u8) -> bool {
    keys[usize::from(keycode / 8)] & (1 << (keycode % 8)) != 0
}

pub fn set_accessory_policy() {}

pub fn ghost_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::PopUp
}

pub fn ghost_window_decorations(_transparent: bool) -> gpui::WindowDecorations {
    gpui::WindowDecorations::Client
}

pub fn adjust_ghost_bounds(bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
    let x = bounds.origin.x.to_f64() + 1.0;
    let y = bounds.origin.y.to_f64() + 1.0;
    let width = (bounds.size.width.to_f64() - 2.0).max(1.0);
    let height = (bounds.size.height.to_f64() - 2.0).max(1.0);
    gpui::Bounds::new(
        gpui::point(gpui::px(x as f32), gpui::px(y as f32)),
        gpui::size(gpui::px(width as f32), gpui::px(height as f32)),
    )
}

pub fn should_poll_focus() -> bool {
    let session = std::env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_ascii_lowercase();
    std::env::var_os("DISPLAY").is_some() || session == "x11"
}

pub fn has_process_focus() -> bool {
    if !should_poll_focus() {
        return true;
    }
    let mut guard = match keymap_conn().lock() {
        Ok(g) => g,
        Err(_) => return true,
    };
    let focus_opt = {
        let conn = match &*guard {
            Some(c) => c,
            None => {
                *guard = x11rb::connect(None).map(|(c, _)| c).ok();
                if guard.is_none() {
                    return true;
                }
                guard.as_ref().unwrap()
            }
        };
        conn.get_input_focus()
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| reply.focus)
    };
    let focus = match focus_opt {
        Some(f) => f,
        None => {
            *guard = None;
            return true;
        }
    };
    if focus == 0 {
        return false;
    }
    static KNOWN_OWNED_WINDOW: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if focus == KNOWN_OWNED_WINDOW.load(std::sync::atomic::Ordering::Relaxed) {
        return true;
    }
    let owns = owns_window(guard.as_ref().unwrap(), focus, std::process::id());
    if owns {
        KNOWN_OWNED_WINDOW.store(focus, std::sync::atomic::Ordering::Relaxed);
    }
    owns
}

pub fn square_window_corners(_window: &mut gpui::Window) {}

pub fn start_window_move(window: &mut gpui::Window) {
    window.start_window_move();
}

pub fn settings_surface_taskbar_identity() -> super::SettingsSurfaceTaskbarIdentity {
    super::SettingsSurfaceTaskbarIdentity {
        app_id: qol_conventions::SETTINGS_SURFACE_APP_ID,
        display_name: qol_conventions::SETTINGS_SURFACE_DISPLAY_NAME,
        icon: super::TaskbarIconSource::DesktopEntry {
            icon_id: qol_conventions::TRAY_ICON_NAME,
        },
    }
}

pub fn apply_settings_surface_identity(_window: &mut gpui::Window) {}

fn owns_window(conn: &RustConnection, mut window: u32, target_pid: u32) -> bool {
    loop {
        if window_pid(conn, window) == Some(target_pid) {
            return true;
        }
        let Ok(reply) = conn.query_tree(window) else {
            return false;
        };
        let Ok(tree) = reply.reply() else {
            return false;
        };
        if tree.parent == 0 || tree.parent == tree.root || tree.parent == window {
            return false;
        }
        window = tree.parent;
    }
}

fn get_pid_atom(conn: &RustConnection) -> Option<u32> {
    static PID_ATOM: OnceLock<Option<u32>> = OnceLock::new();
    *PID_ATOM.get_or_init(|| {
        conn.intern_atom(false, b"_NET_WM_PID")
            .ok()?
            .reply()
            .ok()
            .map(|r| r.atom)
    })
}

fn window_pid(conn: &RustConnection, window: u32) -> Option<u32> {
    use x11rb::protocol::xproto::AtomEnum;
    let atom = get_pid_atom(conn)?;
    let prop = conn
        .get_property(false, window, atom, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    prop.value32().and_then(|mut value| value.next())
}

#[cfg(test)]
mod tests {
    use super::keycode_held;

    #[test]
    fn keymap_reads_escape_and_modifier_keycodes() {
        let cases = [9u8, 50, 62, 64, 108, 255];
        for keycode in cases {
            let mut keys = [0u8; 32];
            keys[usize::from(keycode / 8)] |= 1 << (keycode % 8);
            assert!(keycode_held(&keys, keycode), "keycode: {keycode}");
            assert!(!keycode_held(&[0u8; 32], keycode), "keycode: {keycode}");
        }
    }
}
