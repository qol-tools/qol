use gpui::*;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct ActiveMonitor {
    bounds: Bounds<Pixels>,
    cursor: Option<Point<Pixels>>,
}

impl ActiveMonitor {
    pub fn centered_bounds(&self, win_size: Size<Pixels>) -> Bounds<Pixels> {
        let x = self.bounds.origin.x + (self.bounds.size.width - win_size.width) / 2.0;
        let y = self.bounds.origin.y + (self.bounds.size.height - win_size.height) / 3.0;
        Bounds::new(point(x, y), win_size)
    }

    pub fn cursor_centered_bounds(&self, win_size: Size<Pixels>) -> Bounds<Pixels> {
        let x = match self.cursor {
            Some(pt) => (pt.x - win_size.width / 2.0).max(self.bounds.origin.x),
            None => self.bounds.origin.x + (self.bounds.size.width - win_size.width) / 2.0,
        };
        let y = self.bounds.origin.y + (self.bounds.size.height - win_size.height) / 3.0;
        Bounds::new(point(x, y), win_size)
    }

    pub fn bounds(&self) -> &Bounds<Pixels> {
        &self.bounds
    }
}

pub fn active(cx: &App) -> Option<ActiveMonitor> {
    let monitors = physical_monitors(cx);
    if monitors.is_empty() {
        return None;
    }
    if monitors.len() == 1 {
        return Some(ActiveMonitor { bounds: monitors[0], cursor: None });
    }

    let focus = poll_focus_once();
    let monitor_bounds = focus
        .as_ref()
        .and_then(|snap| monitor_for_bounds(&monitors, &snap.bounds))
        .unwrap_or(monitors[0]);

    Some(ActiveMonitor { bounds: monitor_bounds, cursor: None })
}

fn physical_monitors(cx: &App) -> Vec<Bounds<Pixels>> {
    let gpui_displays = cx.displays();
    if gpui_displays.len() > 1 {
        return gpui_displays.iter().map(|d| d.bounds()).collect();
    }

    #[cfg(target_os = "linux")]
    {
        let xrandr = xrandr_monitors();
        if xrandr.len() > 1 {
            return xrandr;
        }
    }

    #[cfg(target_os = "macos")]
    {
        let cg = macos_display_bounds();
        if cg.len() > 1 {
            return cg;
        }
    }

    gpui_displays.iter().map(|d| d.bounds()).collect()
}

fn monitor_for_bounds(monitors: &[Bounds<Pixels>], window: &Bounds<Pixels>) -> Option<Bounds<Pixels>> {
    monitors
        .iter()
        .filter_map(|m| {
            let area = intersection_area(window, m);
            if area > 0.0 { Some((*m, area)) } else { None }
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(m, _)| m)
}

fn monitor_for_point(monitors: &[Bounds<Pixels>], pt: Point<Pixels>) -> Option<Bounds<Pixels>> {
    monitors.iter().find(|m| m.contains(&pt)).copied()
}

fn intersection_area(a: &Bounds<Pixels>, b: &Bounds<Pixels>) -> f64 {
    let inter = a.intersect(b);
    if inter.size.width <= px(0.) || inter.size.height <= px(0.) {
        return 0.0;
    }
    inter.size.width.to_f64() * inter.size.height.to_f64()
}

pub struct Tracker {
    rx: mpsc::Receiver<PollResult>,
    monitors: Vec<Bounds<Pixels>>,
    last_focus: Option<FocusSnapshot>,
    last_focus_at: Option<Instant>,
    last_click: Option<ClickInfo>,
    last_click_at: Option<Instant>,
}

impl Tracker {
    pub fn start(cx: &App) -> Self {
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let mut state = PollState::default();
            loop {
                let (result, new_state) = poll_once(state);
                state = new_state;
                if tx.send(result).is_err() {
                    break;
                }
            }
        });

        Self {
            rx,
            monitors: physical_monitors(cx),
            last_focus: None,
            last_focus_at: None,
            last_click: None,
            last_click_at: None,
        }
    }

    pub fn poll(&mut self) {
        while let Ok(result) = self.rx.try_recv() {
            self.update_focus(result.focus_snapshot);
            self.update_click(result.click_point);
        }
    }

    fn update_focus(&mut self, snap: Option<FocusSnapshot>) {
        let Some(snap) = snap else { return };
        let is_new = self
            .last_focus
            .as_ref()
            .map(|prev| prev.signature() != snap.signature())
            .unwrap_or(true);
        if is_new {
            self.last_focus_at = Some(Instant::now());
        }
        self.last_focus = Some(snap);
    }

    fn update_click(&mut self, point: Option<Point<Pixels>>) {
        let Some(point) = point else { return };
        self.last_click = Some(ClickInfo {
            global: point,
            monitor: monitor_for_point(&self.monitors, point),
        });
        self.last_click_at = Some(Instant::now());
    }

    pub fn active(&self) -> Option<ActiveMonitor> {
        if self.monitors.is_empty() {
            return None;
        }

        let focus_monitor = self
            .last_focus
            .as_ref()
            .and_then(|snap| monitor_for_bounds(&self.monitors, &snap.bounds));

        let click_monitor = self.last_click.as_ref().and_then(|c| c.monitor);

        let (active, cursor) = match resolve(
            focus_monitor,
            self.last_focus_at,
            click_monitor,
            self.last_click_at,
        ) {
            Some((m, ActiveSource::Click)) => {
                let pt = self.last_click.as_ref().map(|c| c.global);
                (Some(m), pt)
            }
            Some((m, ActiveSource::Focus)) => (Some(m), None),
            None => (None, None),
        };

        let bounds = active.unwrap_or(self.monitors[0]);
        Some(ActiveMonitor { bounds, cursor })
    }
}

#[derive(Clone)]
struct ClickInfo {
    global: Point<Pixels>,
    monitor: Option<Bounds<Pixels>>,
}

#[derive(Clone)]
struct FocusSnapshot {
    window_id: Option<u64>,
    bounds: Bounds<Pixels>,
}

impl FocusSnapshot {
    fn signature(&self) -> (Option<u64>, Bounds<Pixels>) {
        (self.window_id, self.bounds)
    }
}

#[derive(Clone, Copy)]
enum ActiveSource {
    Focus,
    Click,
}

struct PollResult {
    focus_snapshot: Option<FocusSnapshot>,
    click_point: Option<Point<Pixels>>,
}

#[derive(Default)]
struct PollState {
    #[cfg(target_os = "linux")]
    xinput_device: Option<String>,
    #[cfg(target_os = "linux")]
    button_down: bool,
}

const POLL_INTERVAL_MS: u64 = 100;

fn resolve(
    focus: Option<Bounds<Pixels>>,
    focus_time: Option<Instant>,
    click: Option<Bounds<Pixels>>,
    click_time: Option<Instant>,
) -> Option<(Bounds<Pixels>, ActiveSource)> {
    match (focus.zip(focus_time), click.zip(click_time)) {
        (Some((fb, ft)), Some((cb, ct))) => {
            if ft >= ct {
                Some((fb, ActiveSource::Focus))
            } else {
                Some((cb, ActiveSource::Click))
            }
        }
        (Some((fb, _)), None) => Some((fb, ActiveSource::Focus)),
        (None, Some((cb, _))) => Some((cb, ActiveSource::Click)),
        _ => None,
    }
}

fn poll_once(mut state: PollState) -> (PollResult, PollState) {
    let focus_snapshot = poll_focus_once();
    let click_point = poll_click_once(&mut state);
    std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    (PollResult { focus_snapshot, click_point }, state)
}

#[cfg(target_os = "linux")]
fn poll_focus_once() -> Option<FocusSnapshot> {
    use std::process::Command;

    if is_wayland() {
        return None;
    }

    let out = Command::new("xdotool")
        .args(["getactivewindow", "getwindowgeometry", "--shell"])
        .output()
        .ok()?;

    if !out.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let window_id = parse_shell_var::<u64>(&stdout, "WINDOW");
    let x = parse_shell_var::<i32>(&stdout, "X")?;
    let y = parse_shell_var::<i32>(&stdout, "Y")?;
    let w = parse_shell_var::<i32>(&stdout, "WIDTH")?;
    let h = parse_shell_var::<i32>(&stdout, "HEIGHT")?;

    if w <= 0 || h <= 0 {
        return None;
    }

    Some(FocusSnapshot {
        window_id,
        bounds: Bounds::new(
            point(px(x as f32), px(y as f32)),
            size(px(w as f32), px(h as f32)),
        ),
    })
}

#[cfg(target_os = "macos")]
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

    let parts: Vec<i32> = stdout.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if parts.len() != 4 || parts[2] <= 0 || parts[3] <= 0 {
        return None;
    }

    Some(FocusSnapshot {
        window_id: None,
        bounds: Bounds::new(
            point(px(parts[0] as f32), px(parts[1] as f32)),
            size(px(parts[2] as f32), px(parts[3] as f32)),
        ),
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn poll_focus_once() -> Option<FocusSnapshot> {
    None
}

#[cfg(target_os = "linux")]
fn poll_click_once(state: &mut PollState) -> Option<Point<Pixels>> {
    use std::process::Command;

    if is_wayland() {
        return None;
    }

    let device_id = match state.xinput_device.clone() {
        Some(id) => id,
        None => {
            let id = detect_xinput_device()?;
            state.xinput_device = Some(id.clone());
            id
        }
    };

    let out = Command::new("xinput")
        .args(["--query-state", &device_id])
        .output()
        .ok()?;

    if !out.status.success() {
        state.xinput_device = None;
        return None;
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let button_down = stdout.lines().any(|l| l.contains("button[") && l.contains("]=down"));

    let click = if button_down && !state.button_down {
        query_mouse_location()
    } else {
        None
    };

    state.button_down = button_down;
    click
}

#[cfg(not(target_os = "linux"))]
fn poll_click_once(_state: &mut PollState) -> Option<Point<Pixels>> {
    None
}

#[cfg(target_os = "linux")]
fn xrandr_monitors() -> Vec<Bounds<Pixels>> {
    use std::process::Command;

    let out = match Command::new("xrandr").arg("--current").output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };

    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(parse_xrandr_line)
        .collect()
}

#[cfg(target_os = "linux")]
fn parse_xrandr_line(line: &str) -> Option<Bounds<Pixels>> {
    if !line.contains(" connected") {
        return None;
    }

    let geom = line.split_whitespace().find(|s| s.contains('+') && s.contains('x'))?;
    let (res, offsets) = geom.split_once('+')?;
    let (w, h) = res.split_once('x')?;
    let (ox, oy) = offsets.split_once('+')?;

    Some(Bounds::new(
        point(px(ox.parse::<f32>().ok()?), px(oy.parse::<f32>().ok()?)),
        size(px(w.parse::<f32>().ok()?), px(h.parse::<f32>().ok()?)),
    ))
}

#[cfg(target_os = "linux")]
fn detect_xinput_device() -> Option<String> {
    use std::process::Command;

    let out = Command::new("xinput").args(["list", "--short"]).output().ok()?;
    if !out.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&out.stdout);

    let preferred = |line: &str| {
        let lower = line.to_lowercase();
        line.contains("slave  pointer")
            && !line.contains("XTEST")
            && (lower.contains("mouse") || lower.contains("logitech") || lower.contains("razer"))
    };

    let fallback = |line: &str| {
        line.contains("slave  pointer")
            && !line.contains("XTEST")
            && !line.contains("Consumer")
            && !line.contains("Keyboard")
    };

    stdout
        .lines()
        .find(|l| preferred(l))
        .or_else(|| stdout.lines().find(|l| fallback(l)))
        .and_then(|line| {
            line.split("id=").nth(1)?.split_whitespace().next().map(String::from)
        })
}

#[cfg(target_os = "linux")]
fn query_mouse_location() -> Option<Point<Pixels>> {
    use std::process::Command;

    let out = Command::new("xdotool")
        .args(["getmouselocation", "--shell"])
        .output()
        .ok()?;

    if !out.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let x = parse_shell_var::<i32>(&stdout, "X")?;
    let y = parse_shell_var::<i32>(&stdout, "Y")?;
    Some(point(px(x as f32), px(y as f32)))
}

fn parse_shell_var<T: std::str::FromStr>(output: &str, key: &str) -> Option<T> {
    output
        .lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('=')?.trim().parse().ok())
}

#[cfg(target_os = "linux")]
fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").map(|v| v == "wayland").unwrap_or(false)
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

#[cfg(target_os = "macos")]
fn macos_display_bounds() -> Vec<Bounds<Pixels>> {
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGRect { origin: CGPoint, size: CGSize }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGPoint { x: f64, y: f64 }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct CGSize { width: f64, height: f64 }

    type CGDirectDisplayID = u32;

    extern "C" {
        fn CGGetActiveDisplayList(max: u32, displays: *mut CGDirectDisplayID, count: *mut u32) -> i32;
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
