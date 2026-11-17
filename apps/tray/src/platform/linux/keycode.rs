use evdev::KeyCode;

pub(crate) fn keycode_name(code: u16) -> &'static str {
    const ESC: u16 = KeyCode::KEY_ESC.0;
    const ENTER: u16 = KeyCode::KEY_ENTER.0;
    const SPACE: u16 = KeyCode::KEY_SPACE.0;
    const TAB: u16 = KeyCode::KEY_TAB.0;
    const UP: u16 = KeyCode::KEY_UP.0;
    const DOWN: u16 = KeyCode::KEY_DOWN.0;
    const LEFT: u16 = KeyCode::KEY_LEFT.0;
    const RIGHT: u16 = KeyCode::KEY_RIGHT.0;
    const LEFTCTRL: u16 = KeyCode::KEY_LEFTCTRL.0;
    const RIGHTCTRL: u16 = KeyCode::KEY_RIGHTCTRL.0;
    const LEFTALT: u16 = KeyCode::KEY_LEFTALT.0;
    const RIGHTALT: u16 = KeyCode::KEY_RIGHTALT.0;
    const LEFTSHIFT: u16 = KeyCode::KEY_LEFTSHIFT.0;
    const RIGHTSHIFT: u16 = KeyCode::KEY_RIGHTSHIFT.0;
    const LEFTMETA: u16 = KeyCode::KEY_LEFTMETA.0;
    const RIGHTMETA: u16 = KeyCode::KEY_RIGHTMETA.0;
    const A: u16 = KeyCode::KEY_A.0;
    const Z: u16 = KeyCode::KEY_Z.0;
    match code {
        ESC => "esc",
        ENTER => "enter",
        SPACE => "space",
        TAB => "tab",
        UP => "up",
        DOWN => "down",
        LEFT => "left",
        RIGHT => "right",
        LEFTCTRL => "ctrl",
        RIGHTCTRL => "rctrl",
        LEFTALT => "alt",
        RIGHTALT => "ralt",
        LEFTSHIFT => "shift",
        RIGHTSHIFT => "rshift",
        LEFTMETA => "super",
        RIGHTMETA => "rsuper",
        _ => {
            if (A..=Z).contains(&code) {
                static LETTERS: [&str; 26] = [
                    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p",
                    "q", "r", "s", "t", "u", "v", "w", "x", "y", "z",
                ];
                LETTERS[(code - A) as usize]
            } else {
                "key"
            }
        }
    }
}
