use qol_windowing::platform::windows::Window;
use qol_windowing::WindowRect;
use windows_sys::Win32::Foundation::{CloseHandle, FALSE, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUERY_USER_NOTIFICATION_STATE, QUNS_BUSY, QUNS_PRESENTATION_MODE,
    QUNS_RUNNING_D3D_FULL_SCREEN,
};

use crate::cursor::platform::shake::{FocusProbe, GameFocus};

const IMAGE_PATH_CAPACITY: usize = 1024;
const EDGE_TOLERANCE_PX: f64 = 1.0;
const GAME_LIBRARIES: [(&str, &str); 3] = [
    (r"\steamapps\common\", "steam_library"),
    (r"\xboxgames\", "xbox_library"),
    (r"\epic games\", "epic_library"),
];
const LAUNCHER_DIRECTORY: &str = r"\launcher\";

pub(super) struct GameFocusDetector;

impl FocusProbe for GameFocusDetector {
    fn probe(&self) -> GameFocus {
        let Some(window) = Window::foreground() else {
            return GameFocus::inactive();
        };
        let pid = window.pid();
        let image = pid.and_then(process_image);
        let evidence = game_evidence(notification_state(), image.as_deref());
        GameFocus {
            active: evidence.is_some(),
            window_id: window.id().as_u32().map(u64::from),
            pid,
            evidence,
        }
    }

    fn active_window_is_fullscreen(&self) -> bool {
        if notification_state().is_some_and(is_fullscreen_state) {
            return true;
        }
        let Some(frame) = Window::foreground().and_then(Window::frame) else {
            return false;
        };
        monitor_at(&frame).is_some_and(|monitor| covers_monitor(&frame, &monitor))
    }
}

fn game_evidence(
    state: Option<QUERY_USER_NOTIFICATION_STATE>,
    image: Option<&str>,
) -> Option<&'static str> {
    if state == Some(QUNS_RUNNING_D3D_FULL_SCREEN) {
        return Some("d3d_fullscreen");
    }
    let path = image?.replace('/', r"\").to_lowercase();
    if path.contains(LAUNCHER_DIRECTORY) {
        return None;
    }
    GAME_LIBRARIES
        .iter()
        .find(|(fragment, _)| path.contains(fragment))
        .map(|(_, evidence)| *evidence)
}

fn is_fullscreen_state(state: QUERY_USER_NOTIFICATION_STATE) -> bool {
    matches!(
        state,
        QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE
    )
}

fn covers_monitor(frame: &WindowRect, monitor: &WindowRect) -> bool {
    frame.x <= monitor.x + EDGE_TOLERANCE_PX
        && frame.y <= monitor.y + EDGE_TOLERANCE_PX
        && frame.x + frame.width >= monitor.x + monitor.width - EDGE_TOLERANCE_PX
        && frame.y + frame.height >= monitor.y + monitor.height - EDGE_TOLERANCE_PX
}

fn notification_state() -> Option<QUERY_USER_NOTIFICATION_STATE> {
    let mut state: QUERY_USER_NOTIFICATION_STATE = 0;
    (unsafe { SHQueryUserNotificationState(&mut state) } >= 0).then_some(state)
}

fn monitor_at(frame: &WindowRect) -> Option<WindowRect> {
    let center = POINT {
        x: (frame.x + frame.width / 2.0) as i32,
        y: (frame.y + frame.height / 2.0) as i32,
    };
    let monitor = unsafe { MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_null() {
        return None;
    }
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    (unsafe { GetMonitorInfoW(monitor, &mut info) } != 0).then(|| rect_of(info.rcMonitor))
}

fn rect_of(rect: RECT) -> WindowRect {
    WindowRect {
        x: f64::from(rect.left),
        y: f64::from(rect.top),
        width: f64::from(rect.right - rect.left),
        height: f64::from(rect.bottom - rect.top),
    }
}

fn process_image(pid: u32) -> Option<String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    if process.is_null() {
        return None;
    }
    let mut buffer = [0u16; IMAGE_PATH_CAPACITY];
    let mut length = buffer.len() as u32;
    let read = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            buffer.as_mut_ptr(),
            &mut length,
        )
    };
    unsafe { CloseHandle(process) };
    (read != 0).then(|| String::from_utf16_lossy(&buffer[..length as usize]))
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::UI::Shell::{QUNS_ACCEPTS_NOTIFICATIONS, QUNS_QUIET_TIME};

    use super::*;

    #[test]
    fn game_evidence_table() {
        let cases = [
            (
                Some(QUNS_RUNNING_D3D_FULL_SCREEN),
                None,
                Some("d3d_fullscreen"),
            ),
            (
                Some(QUNS_ACCEPTS_NOTIFICATIONS),
                Some(r"D:\SteamLibrary\steamapps\common\Hades\Hades.exe"),
                Some("steam_library"),
            ),
            (
                None,
                Some("C:/XboxGames/Halo Infinite/Content/HaloInfinite.exe"),
                Some("xbox_library"),
            ),
            (
                None,
                Some(r"C:\Program Files\Epic Games\Fortnite\FortniteClient.exe"),
                Some("epic_library"),
            ),
            (
                None,
                Some(r"C:\Program Files (x86)\Epic Games\Launcher\Portal\EpicGamesLauncher.exe"),
                None,
            ),
            (None, Some(r"C:\Program Files (x86)\Steam\steam.exe"), None),
            (Some(QUNS_BUSY), Some(r"C:\Windows\explorer.exe"), None),
            (None, None, None),
        ];
        for (state, image, expected) in cases {
            assert_eq!(
                game_evidence(state, image),
                expected,
                "state={state:?} image={image:?}"
            );
        }
    }

    #[test]
    fn fullscreen_states_table() {
        let cases = [
            (QUNS_BUSY, true),
            (QUNS_RUNNING_D3D_FULL_SCREEN, true),
            (QUNS_PRESENTATION_MODE, true),
            (QUNS_ACCEPTS_NOTIFICATIONS, false),
            (QUNS_QUIET_TIME, false),
        ];
        for (state, expected) in cases {
            assert_eq!(is_fullscreen_state(state), expected, "state={state}");
        }
    }

    #[test]
    fn monitor_coverage_table() {
        let monitor = WindowRect::from_array([1920.0, 0.0, 2560.0, 1440.0]);
        let cases = [
            ([1920.0, 0.0, 2560.0, 1440.0], true),
            ([1919.0, -1.0, 2562.0, 1442.0], true),
            ([1920.5, 0.5, 2559.0, 1439.0], true),
            ([1920.0, 0.0, 2560.0, 1392.0], false),
            ([1928.0, 0.0, 2544.0, 1440.0], false),
            ([0.0, 0.0, 1920.0, 1080.0], false),
        ];
        for (frame, expected) in cases {
            assert_eq!(
                covers_monitor(&WindowRect::from_array(frame), &monitor),
                expected,
                "frame={frame:?}"
            );
        }
    }
}
