use super::{monitor_for_bounds, ActiveMonitor, InputState, Stamped};
use gpui::*;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FOCUS_POLL_MS: u64 = 300;

pub(super) fn start_focus_tracking(state: Arc<Mutex<InputState>>, monitors: Vec<Bounds<Pixels>>) {
    std::thread::spawn(move || {
        focus_poller(state, monitors);
    });
}

pub(super) fn display_bounds() -> Vec<Bounds<Pixels>> {
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    type CGDirectDisplayID = u32;

    extern "C" {
        fn CGGetActiveDisplayList(
            max: u32,
            displays: *mut CGDirectDisplayID,
            count: *mut u32,
        ) -> i32;
        fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    }

    let mut ids = [0u32; 16];
    let mut count = 0u32;

    let ret = unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut count) };
    if ret != 0 {
        return Vec::new();
    }

    (0..count as usize)
        .map(|i| {
            let rect = unsafe { CGDisplayBounds(ids[i]) };
            Bounds::new(
                point(px(rect.origin.x as f32), px(rect.origin.y as f32)),
                size(px(rect.size.width as f32), px(rect.size.height as f32)),
            )
        })
        .collect()
}

struct FocusSnapshot {
    bounds: Bounds<Pixels>,
}

fn focus_poller(state: Arc<Mutex<InputState>>, monitors: Vec<Bounds<Pixels>>) {
    loop {
        let bounds = poll_focus_once()
            .as_ref()
            .and_then(|snap| monitor_for_bounds(&monitors, &snap.bounds));

        if let Some(bounds) = bounds {
            let Ok(mut guard) = state.lock() else { return };
            guard.focus = Some(Stamped {
                monitor: ActiveMonitor { bounds },
                at: Instant::now(),
            });
        }

        std::thread::sleep(Duration::from_millis(FOCUS_POLL_MS));
    }
}

fn poll_focus_once() -> Option<FocusSnapshot> {
    use std::process::Command;

    let script = r#"
        tell application "System Events"
            set frontApp to first application process whose frontmost is true
            try
                set frontWindow to first window of frontApp
                set {x, y} to position of frontWindow
                set {w, h} to size of frontWindow
                return (x as string) & "," & (y as string) & "," & (w as string) & "," & (h as string)
            on error
                return "none"
            end try
        end tell
    "#;

    let out = Command::new("osascript")
        .args(["-e", script])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if stdout == "none" || stdout.is_empty() {
        return None;
    }

    let parts: Vec<i32> = stdout
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if parts.len() != 4 || parts[2] <= 0 || parts[3] <= 0 {
        return None;
    }

    Some(FocusSnapshot {
        bounds: Bounds::new(
            point(px(parts[0] as f32), px(parts[1] as f32)),
            size(px(parts[2] as f32), px(parts[3] as f32)),
        ),
    })
}
