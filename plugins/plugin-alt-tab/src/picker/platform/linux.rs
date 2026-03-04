use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;

pub fn picker_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::PopUp
}

pub fn dismiss_picker(window: &mut gpui::Window) {
    window.minimize_window();
}

pub fn is_modifier_held() -> bool {
    let Ok((conn, _)) = x11rb::connect(None) else {
        return false;
    };
    let Ok(reply) = conn.query_keymap() else {
        return false;
    };
    let Ok(keymap) = reply.reply() else {
        return false;
    };
    let alt_l_held = keymap.keys[64 / 8] & (1 << (64 % 8)) != 0;
    let alt_r_held = keymap.keys[108 / 8] & (1 << (108 % 8)) != 0;
    alt_l_held || alt_r_held
}

pub fn is_shift_held() -> bool {
    let Ok((conn, _)) = x11rb::connect(None) else {
        return false;
    };
    let Ok(reply) = conn.query_keymap() else {
        return false;
    };
    let Ok(keymap) = reply.reply() else {
        return false;
    };
    // Shift_L = keycode 50, Shift_R = keycode 62
    let shift_l = keymap.keys[50 / 8] & (1 << (50 % 8)) != 0;
    let shift_r = keymap.keys[62 / 8] & (1 << (62 % 8)) != 0;
    shift_l || shift_r
}

pub fn set_accessory_policy() {}

pub fn reposition_picker_window(x: f64, y: f64) -> bool {
    move_app_window("qol-alt-tab-picker", x as i32, y as i32)
}

fn move_app_window(title: &str, x: i32, y: i32) -> bool {
    std::process::Command::new("xdotool")
        .arg("search")
        .arg("--name")
        .arg(title)
        .arg("windowmove")
        .arg(x.to_string())
        .arg(y.to_string())
        .status()
        .ok()
        .is_some_and(|s| s.success())
}

pub fn disable_window_shadow() {}
