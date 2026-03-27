use std::collections::HashMap;

use gpui::*;

use crate::monitor::ActiveMonitor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MonitorKey {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl MonitorKey {
    pub fn from_bounds(bounds: &Bounds<Pixels>) -> Self {
        Self {
            x: bounds.origin.x.to_f64().round() as i32,
            y: bounds.origin.y.to_f64().round() as i32,
            width: bounds.size.width.to_f64().round() as i32,
            height: bounds.size.height.to_f64().round() as i32,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WindowPlacement {
    pub target: MonitorKey,
    pub bounds: Bounds<Pixels>,
    pub display_id: Option<DisplayId>,
}

pub struct ActiveWindows<T> {
    windows: HashMap<MonitorKey, WindowHandle<T>>,
}

impl<T> Default for ActiveWindows<T> {
    fn default() -> Self {
        Self {
            windows: HashMap::new(),
        }
    }
}

impl<T: 'static> ActiveWindows<T> {
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn existing(&self, target: MonitorKey) -> Option<WindowHandle<T>> {
        self.windows.get(&target).cloned()
    }

    pub fn any_existing(&self) -> Option<(MonitorKey, WindowHandle<T>)> {
        self.windows.iter().next().map(|(k, v)| (*k, *v))
    }

    pub fn insert(&mut self, target: MonitorKey, handle: WindowHandle<T>) {
        self.windows.insert(target, handle);
    }

    pub fn remove(&mut self, target: MonitorKey) {
        self.windows.remove(&target);
    }

    pub fn iter(&self) -> Vec<(MonitorKey, WindowHandle<T>)> {
        self.windows.iter().map(|(k, v)| (*k, *v)).collect()
    }

    pub fn destroy_non_target(&mut self, target: MonitorKey, cx: &mut App)
    where
        T: Render,
    {
        let non_targets: Vec<MonitorKey> = self
            .windows
            .keys()
            .filter(|k| **k != target)
            .copied()
            .collect();
        for key in non_targets {
            if let Some(handle) = self.windows.remove(&key) {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        }
    }
}

pub fn open_window_with_focus<T, F>(
    cx: &mut App,
    options: WindowOptions,
    build: F,
) -> Result<WindowHandle<T>>
where
    T: Render + Focusable + 'static,
    F: FnOnce(&mut Window, &mut Context<T>) -> T + 'static,
{
    cx.open_window(options, |window, cx| {
        let view = cx.new(|cx| build(window, cx));
        window.focus(&view.focus_handle(cx));
        window.activate_window();
        view
    })
}

pub fn target_monitor_key(monitor: Option<&ActiveMonitor>) -> MonitorKey {
    let Some(monitor) = monitor else {
        return MonitorKey::default();
    };
    MonitorKey::from_bounds(&monitor.bounds())
}

pub fn centered_window_placement(
    monitor: Option<&ActiveMonitor>,
    win_size: Size<Pixels>,
    cx: &App,
) -> WindowPlacement {
    let bounds = match monitor {
        Some(active) => active.centered_bounds(win_size),
        None => Bounds::centered(None, win_size, cx),
    };
    WindowPlacement {
        target: target_monitor_key(monitor),
        bounds,
        display_id: display_id_for_monitor(monitor, cx),
    }
}

fn display_id_for_monitor(monitor: Option<&ActiveMonitor>, cx: &App) -> Option<DisplayId> {
    let monitor = monitor?;
    let target_bounds = monitor.bounds();
    cx.displays()
        .into_iter()
        .find(|display| bounds_match(&display.bounds(), &target_bounds))
        .map(|display| display.id())
}

fn bounds_match(a: &Bounds<Pixels>, b: &Bounds<Pixels>) -> bool {
    let a = MonitorKey::from_bounds(a);
    let b = MonitorKey::from_bounds(b);
    coord_diff(a.x, b.x)
        && coord_diff(a.y, b.y)
        && coord_diff(a.width, b.width)
        && coord_diff(a.height, b.height)
}

fn coord_diff(a: i32, b: i32) -> bool {
    (a - b).abs() <= 4
}
