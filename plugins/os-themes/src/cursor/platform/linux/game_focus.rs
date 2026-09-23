use std::ffi::CStr;
use std::fs;
use std::ptr;

use anyhow::{ensure, Result};
use x11::xlib;

const GAME_ENVIRONMENT_KEYS: &[&str] = &[
    "SteamGameId",
    "SteamAppId",
    "STEAM_COMPAT_APP_ID",
    "LUTRIS_GAME_UUID",
];

pub(super) struct GameFocusDetector {
    display: *mut xlib::Display,
    root: xlib::Window,
    active_window_atom: xlib::Atom,
    window_pid_atom: xlib::Atom,
    window_state_atom: xlib::Atom,
    window_fullscreen_atom: xlib::Atom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GameFocus {
    pub active: bool,
    pub window_id: Option<u64>,
    pub pid: Option<u32>,
    pub evidence: Option<&'static str>,
}

impl GameFocus {
    pub const fn inactive() -> Self {
        Self {
            active: false,
            window_id: None,
            pid: None,
            evidence: None,
        }
    }
}

impl GameFocusDetector {
    pub fn open() -> Result<Self> {
        let display = unsafe { xlib::XOpenDisplay(ptr::null()) };
        ensure!(!display.is_null(), "failed to open X11 display");

        Ok(Self {
            display,
            root: unsafe { xlib::XDefaultRootWindow(display) },
            active_window_atom: intern_atom(display, c"_NET_ACTIVE_WINDOW"),
            window_pid_atom: intern_atom(display, c"_NET_WM_PID"),
            window_state_atom: intern_atom(display, c"_NET_WM_STATE"),
            window_fullscreen_atom: intern_atom(display, c"_NET_WM_STATE_FULLSCREEN"),
        })
    }

    pub fn active_window_is_fullscreen(&self) -> bool {
        let Some(window_id) = property_ulong(
            self.display,
            self.root,
            self.active_window_atom,
            xlib::XA_WINDOW,
        )
        .filter(|window| *window != 0) else {
            return false;
        };
        let states = property_atoms(self.display, window_id, self.window_state_atom);
        contains_fullscreen(&states, self.window_fullscreen_atom)
    }

    pub fn probe(&self) -> GameFocus {
        let window_id = property_ulong(
            self.display,
            self.root,
            self.active_window_atom,
            xlib::XA_WINDOW,
        );
        let Some(window_id) = window_id.filter(|window| *window != 0) else {
            return GameFocus {
                active: false,
                window_id: None,
                pid: None,
                evidence: None,
            };
        };

        let classes = window_classes(self.display, window_id);
        let pid = property_ulong(
            self.display,
            window_id,
            self.window_pid_atom,
            xlib::XA_CARDINAL,
        )
        .and_then(|value| u32::try_from(value).ok())
        .filter(|pid| *pid != 0);
        let environment = pid.and_then(read_process_environment).unwrap_or_default();
        let evidence = game_evidence(classes.iter().map(String::as_str), &environment);

        GameFocus {
            active: evidence.is_some(),
            window_id: Some(window_id),
            pid,
            evidence,
        }
    }
}

impl Drop for GameFocusDetector {
    fn drop(&mut self) {
        unsafe { xlib::XCloseDisplay(self.display) };
    }
}

fn game_evidence<'a>(
    window_classes: impl IntoIterator<Item = &'a str>,
    environment: &[u8],
) -> Option<&'static str> {
    if window_classes.into_iter().any(is_steam_game_class) {
        return Some("steam_window_class");
    }

    for key in GAME_ENVIRONMENT_KEYS {
        let Some(value) = environment_value(environment, key.as_bytes()) else {
            continue;
        };
        let valid = if *key == "LUTRIS_GAME_UUID" {
            !value.is_empty()
        } else {
            numeric_id_is_nonzero(value)
        };
        if valid {
            return Some(key);
        }
    }

    environment_value(environment, b"LD_PRELOAD")
        .filter(|value| {
            value
                .windows(b"libgamemodeauto.so".len())
                .any(|window| window == b"libgamemodeauto.so")
        })
        .map(|_| "gamemode")
}

fn is_steam_game_class(class: &str) -> bool {
    class
        .to_ascii_lowercase()
        .strip_prefix("steam_app_")
        .is_some_and(|app_id| numeric_id_is_nonzero(app_id.as_bytes()))
}

fn numeric_id_is_nonzero(value: &[u8]) -> bool {
    !value.is_empty()
        && value.iter().all(u8::is_ascii_digit)
        && value.iter().any(|byte| *byte != b'0')
}

fn environment_value<'a>(environment: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    environment.split(|byte| *byte == 0).find_map(|entry| {
        let separator = entry.iter().position(|byte| *byte == b'=')?;
        (entry.get(..separator)? == key).then(|| &entry[separator + 1..])
    })
}

fn read_process_environment(pid: u32) -> Option<Vec<u8>> {
    fs::read(format!("/proc/{pid}/environ")).ok()
}

fn intern_atom(display: *mut xlib::Display, name: &CStr) -> xlib::Atom {
    unsafe { xlib::XInternAtom(display, name.as_ptr(), xlib::False) }
}

fn property_ulong(
    display: *mut xlib::Display,
    window: xlib::Window,
    property: xlib::Atom,
    expected_type: xlib::Atom,
) -> Option<libc::c_ulong> {
    if property == 0 {
        return None;
    }

    let mut actual_type = 0;
    let mut actual_format = 0;
    let mut item_count = 0;
    let mut bytes_after = 0;
    let mut data = ptr::null_mut();
    let status = unsafe {
        xlib::XGetWindowProperty(
            display,
            window,
            property,
            0,
            1,
            xlib::False,
            expected_type,
            &mut actual_type,
            &mut actual_format,
            &mut item_count,
            &mut bytes_after,
            &mut data,
        )
    };
    if status != i32::from(xlib::Success)
        || actual_type != expected_type
        || actual_format != 32
        || item_count == 0
        || data.is_null()
    {
        if !data.is_null() {
            unsafe { xlib::XFree(data.cast()) };
        }
        return None;
    }

    let value = unsafe { *data.cast::<libc::c_ulong>() };
    unsafe { xlib::XFree(data.cast()) };
    Some(value)
}

fn contains_fullscreen(states: &[xlib::Atom], fullscreen: xlib::Atom) -> bool {
    states.contains(&fullscreen)
}

fn property_atoms(
    display: *mut xlib::Display,
    window: xlib::Window,
    property: xlib::Atom,
) -> Vec<xlib::Atom> {
    if property == 0 {
        return Vec::new();
    }

    let mut actual_type = 0;
    let mut actual_format = 0;
    let mut item_count = 0;
    let mut bytes_after = 0;
    let mut data = ptr::null_mut();
    let status = unsafe {
        xlib::XGetWindowProperty(
            display,
            window,
            property,
            0,
            64,
            xlib::False,
            xlib::XA_ATOM,
            &mut actual_type,
            &mut actual_format,
            &mut item_count,
            &mut bytes_after,
            &mut data,
        )
    };
    if status != i32::from(xlib::Success)
        || actual_type != xlib::XA_ATOM
        || actual_format != 32
        || item_count == 0
        || data.is_null()
    {
        if !data.is_null() {
            unsafe { xlib::XFree(data.cast()) };
        }
        return Vec::new();
    }

    let atoms = unsafe {
        std::slice::from_raw_parts(data.cast::<xlib::Atom>(), item_count as usize).to_vec()
    };
    unsafe { xlib::XFree(data.cast()) };
    atoms
}

fn window_classes(display: *mut xlib::Display, window: xlib::Window) -> Vec<String> {
    let mut hint = xlib::XClassHint {
        res_name: ptr::null_mut(),
        res_class: ptr::null_mut(),
    };
    if unsafe { xlib::XGetClassHint(display, window, &mut hint) } == 0 {
        return Vec::new();
    }

    let mut classes = Vec::with_capacity(2);
    for value in [hint.res_name, hint.res_class] {
        if value.is_null() {
            continue;
        }
        classes.push(
            unsafe { CStr::from_ptr(value) }
                .to_string_lossy()
                .into_owned(),
        );
        unsafe { xlib::XFree(value.cast()) };
    }
    classes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_confidence_game_signals_are_recognized() {
        let cases = [
            (
                "Steam window class",
                vec!["steam_app_1771300"],
                b"HOME=/home/user\0".as_slice(),
                Some("steam_window_class"),
            ),
            (
                "Steam native game process",
                vec!["KingdomCome"],
                b"SteamGameId=1771300\0SteamAppId=1771300\0".as_slice(),
                Some("SteamGameId"),
            ),
            (
                "Steam Proton process",
                vec!["Main"],
                b"STEAM_COMPAT_APP_ID=1771300\0WINEPREFIX=/games/pfx\0".as_slice(),
                Some("STEAM_COMPAT_APP_ID"),
            ),
            (
                "Lutris game process",
                vec!["game"],
                b"LUTRIS_GAME_UUID=fb9b3438-c570-4c2b-a5df-1a80bd753b10\0".as_slice(),
                Some("LUTRIS_GAME_UUID"),
            ),
            (
                "GameMode process",
                vec!["native-game"],
                b"LD_PRELOAD=libgamemodeauto.so.0\0".as_slice(),
                Some("gamemode"),
            ),
        ];

        for (label, classes, environment, expected) in cases {
            assert_eq!(
                game_evidence(classes, environment),
                expected,
                "case={label}"
            );
        }
    }

    #[test]
    fn launchers_and_lookalikes_are_not_games() {
        let cases = [
            (
                "Steam client",
                vec!["steamwebhelper", "steam"],
                b"SteamEnv=1\0".as_slice(),
            ),
            (
                "Lutris client",
                vec!["lutris", "Lutris"],
                b"HOME=/home/user\0".as_slice(),
            ),
            (
                "Wine application without game evidence",
                vec!["notepad.exe"],
                b"WINEPREFIX=/home/user/.wine\0".as_slice(),
            ),
            (
                "Steam class without numeric app id",
                vec!["steam_app_settings"],
                b"HOME=/home/user\0".as_slice(),
            ),
            (
                "empty Steam app id",
                vec!["Main"],
                b"SteamAppId=0\0".as_slice(),
            ),
            (
                "environment key embedded in a value",
                vec!["editor"],
                b"NOTE=SteamGameId=1771300\0".as_slice(),
            ),
        ];

        for (label, classes, environment) in cases {
            assert_eq!(game_evidence(classes, environment), None, "case={label}");
        }
    }

    #[test]
    fn contains_fullscreen_detects_a_present_fullscreen_atom() {
        assert!(contains_fullscreen(&[1, 2, 3], 2));
    }

    #[test]
    fn contains_fullscreen_rejects_an_absent_fullscreen_atom() {
        assert!(!contains_fullscreen(&[1, 2, 3], 4));
        assert!(!contains_fullscreen(&[], 0));
    }

    #[test]
    fn malformed_environment_is_fail_open() {
        for environment in [
            b"SteamGameId".as_slice(),
            b"SteamGameId=\0".as_slice(),
            b"SteamGameId=not-a-number\0".as_slice(),
            b"\xffSteamGameId=1771300\0".as_slice(),
        ] {
            assert_eq!(game_evidence(["editor"], environment), None);
        }
    }

    #[test]
    #[ignore = "requires an X11 session with a game window focused"]
    fn live_focused_game_is_recognized() {
        let detector = GameFocusDetector::open().expect("X11 game-focus detector must open");
        let focus = detector.probe();
        assert!(
            focus.active,
            "focused window was not recognized as a game: {focus:?}"
        );
    }
}
