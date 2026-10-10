use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::Duration;

use gpui::{point, px, size, App, AppContext, AsyncApp, Bounds, Pixels, Size};
use qol_window_state::{Monitor, Reopen, WindowState, WindowStateStore, SCHEMA_VERSION};
use qol_windowing::{DisplayEnumerator, MonitorBounds, WindowRect};

use crate::monitor::{ActiveMonitor, MonitorTracker};
use crate::placement::MonitorPlacement;

mod platform;

const SAVE_DEBOUNCE: Duration = Duration::from_millis(200);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowMemory {
    key: String,
    reopen: Option<Reopen>,
}

impl WindowMemory {
    pub fn reopened_by_action(
        key: impl Into<String>,
        plugin: impl Into<String>,
        action: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            reopen: Some(Reopen::PluginAction {
                plugin: plugin.into(),
                action: action.into(),
            }),
        }
    }

    pub fn reopened_by_settings(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            reopen: Some(Reopen::Settings { page: None }),
        }
    }
}

pub(crate) struct RestoredPlacement {
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) monitor: Bounds<Pixels>,
}

pub(crate) struct Remembered {
    store: WindowStateStore,
    key: String,
    reopen: RefCell<Option<Reopen>>,
    default_placement: String,
    last: RefCell<Option<WindowState>>,
    save_generation: Cell<u64>,
}

impl Remembered {
    pub(crate) fn new(
        memory: WindowMemory,
        placement: MonitorPlacement,
        cx: &mut App,
    ) -> Option<Rc<Self>> {
        if !qol_window_state::is_valid_key(&memory.key) {
            log::warn!(
                "[window-state] ignoring invalid window key {:?}",
                memory.key
            );
            return None;
        }
        let store = WindowStateStore::shared()?;
        install_flush_hooks(cx);
        let default_placement = placement.memory_label();
        let last = store
            .load(&memory.key)
            .filter(|state| state.default_placement == default_placement);
        Some(Rc::new(Self {
            store,
            key: memory.key,
            reopen: RefCell::new(memory.reopen),
            default_placement,
            last: RefCell::new(last),
            save_generation: Cell::new(0),
        }))
    }

    pub(crate) fn restore(
        &self,
        content: Size<Pixels>,
        keep_content_size: bool,
        cx: &App,
    ) -> Option<RestoredPlacement> {
        let mut saved = self.last.borrow().clone()?;
        if keep_content_size {
            saved.bounds_on_monitor.width = content.width.to_f64();
            saved.bounds_on_monitor.height = content.height.to_f64();
        }
        let monitors = current_monitors(cx);
        let window = qol_window_state::resolve(&saved, &monitors)?;
        let monitor = monitors
            .iter()
            .find(|monitor| contains(monitor.bounds, window))
            .map(|monitor| monitor.bounds)?;
        Some(RestoredPlacement {
            bounds: to_bounds(window),
            monitor: to_bounds(rect_of(monitor)),
        })
    }

    pub(crate) fn record_bounds(self: &Rc<Self>, bounds: Bounds<Pixels>, scale: f32, cx: &mut App) {
        let open = self.last.borrow().as_ref().is_none_or(|state| state.open);
        if !self.capture(bounds, scale, open, cx) {
            return;
        }
        let generation = self.save_generation.get().wrapping_add(1);
        self.save_generation.set(generation);
        let this = self.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(SAVE_DEBOUNCE).await;
            if this.save_generation.get() == generation {
                let key = this.key.clone();
                cx.background_spawn(async move { flush_key(&key) }).await;
            }
        })
        .detach();
    }

    pub(crate) fn record_open(&self, bounds: Bounds<Pixels>, scale: f32, cx: &App) {
        if self.capture(bounds, scale, true, cx) {
            flush_key(&self.key);
        }
    }

    pub(crate) fn record_closed(&self) {
        let Some(mut state) = self.last.borrow().clone() else {
            return;
        };
        state.open = false;
        self.queue(state);
        flush_key(&self.key);
    }

    pub(crate) fn record_page(&self, page: String) {
        let reopen = Some(Reopen::Settings { page: Some(page) });
        if *self.reopen.borrow() == reopen {
            return;
        }
        *self.reopen.borrow_mut() = reopen.clone();
        let Some(mut state) = self.last.borrow().clone() else {
            return;
        };
        state.reopen = reopen;
        self.queue(state);
        flush_key(&self.key);
    }

    fn capture(&self, bounds: Bounds<Pixels>, scale: f32, open: bool, cx: &App) -> bool {
        let Some((monitor, bounds_on_monitor)) =
            qol_window_state::locate(rect_of_bounds(bounds), &current_monitors(cx))
        else {
            return false;
        };
        self.queue(WindowState {
            version: SCHEMA_VERSION,
            key: self.key.clone(),
            monitor,
            bounds_on_monitor,
            scale,
            open,
            owner_pid: std::process::id(),
            default_placement: self.default_placement.clone(),
            reopen: self.reopen.borrow().clone(),
        });
        true
    }

    fn queue(&self, state: WindowState) {
        *self.last.borrow_mut() = Some(state.clone());
        PENDING
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(self.key.clone(), (self.store.clone(), state));
    }
}

type PendingWrites = HashMap<String, (WindowStateStore, WindowState)>;

static PENDING: LazyLock<Mutex<PendingWrites>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Held across take-and-write so an older pending state can never land after a newer one.
static WRITE_ORDER: Mutex<()> = Mutex::new(());

fn flush_key(key: &str) {
    let _order = WRITE_ORDER.lock().unwrap_or_else(PoisonError::into_inner);
    let pending = PENDING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(key);
    if let Some((store, state)) = pending {
        write(&store, &state);
    }
}

pub(crate) fn flush_all() {
    let _order = WRITE_ORDER.lock().unwrap_or_else(PoisonError::into_inner);
    let pending: Vec<_> = PENDING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .drain()
        .map(|(_, entry)| entry)
        .collect();
    for (store, state) in pending {
        write(&store, &state);
    }
}

fn write(store: &WindowStateStore, state: &WindowState) {
    if let Err(error) = store.save(state) {
        log::warn!("[window-state] failed to save {}: {error}", state.key);
    }
}

fn install_flush_hooks(cx: &mut App) {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    cx.on_app_quit(|_| async { flush_all() }).detach();
    platform::flush_on_termination(flush_all);
}

fn current_monitors(cx: &App) -> Vec<Monitor> {
    match qol_windowing::Platform.snapshot() {
        Ok(snapshots) if !snapshots.is_empty() => snapshots
            .into_iter()
            .map(|snapshot| {
                let (bounds, work_area) =
                    logical_areas(&ActiveMonitor::from_bounds(snapshot.bounds));
                Monitor {
                    id: snapshot.handle.id().to_string(),
                    connector: snapshot.handle.connector().to_string(),
                    work_area,
                    bounds,
                    primary: snapshot.primary,
                }
            })
            .collect(),
        _ => MonitorTracker::start(cx)
            .all_monitors_or_snapshot()
            .into_iter()
            .enumerate()
            .map(|(index, monitor)| {
                let id = geometry_id(monitor_bounds(monitor.bounds()));
                let (bounds, work_area) = logical_areas(&monitor);
                Monitor {
                    id: id.clone(),
                    connector: id,
                    work_area,
                    bounds,
                    primary: index == 0,
                }
            })
            .collect(),
    }
}

fn geometry_id(bounds: MonitorBounds) -> String {
    format!(
        "geometry-{}-{}-{}x{}",
        bounds.x, bounds.y, bounds.width, bounds.height
    )
}

fn logical_areas(monitor: &ActiveMonitor) -> (MonitorBounds, MonitorBounds) {
    let native = monitor.bounds();
    let work_area = crate::popup_window::work_area_within(native).unwrap_or(native);
    (
        monitor_bounds(monitor.logical(native)),
        monitor_bounds(monitor.logical(work_area)),
    )
}

fn contains(monitor: MonitorBounds, window: WindowRect) -> bool {
    let x = window.x + window.width / 2.0;
    let y = window.y + window.height / 2.0;
    x >= f64::from(monitor.x)
        && x < f64::from(monitor.x + monitor.width)
        && y >= f64::from(monitor.y)
        && y < f64::from(monitor.y + monitor.height)
}

fn rect_of(bounds: MonitorBounds) -> WindowRect {
    WindowRect {
        x: f64::from(bounds.x),
        y: f64::from(bounds.y),
        width: f64::from(bounds.width),
        height: f64::from(bounds.height),
    }
}

fn rect_of_bounds(bounds: Bounds<Pixels>) -> WindowRect {
    WindowRect {
        x: bounds.origin.x.to_f64(),
        y: bounds.origin.y.to_f64(),
        width: bounds.size.width.to_f64(),
        height: bounds.size.height.to_f64(),
    }
}

fn monitor_bounds(bounds: Bounds<Pixels>) -> MonitorBounds {
    let rect = rect_of_bounds(bounds);
    MonitorBounds {
        x: rect.x as f32,
        y: rect.y as f32,
        width: rect.width as f32,
        height: rect.height as f32,
    }
}

fn to_bounds(rect: WindowRect) -> Bounds<Pixels> {
    Bounds::new(
        point(px(rect.x as f32), px(rect.y as f32)),
        size(px(rect.width as f32), px(rect.height as f32)),
    )
}

#[cfg(test)]
mod tests;
