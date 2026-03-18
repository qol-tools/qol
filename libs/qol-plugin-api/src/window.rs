use std::collections::HashMap;

use gpui::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
