use anyhow::Result;
use gpui::{point, px, App, Bounds, DisplayId, Pixels, Point};
use qol_gpui::monitor::ActiveMonitor;
use qol_gpui::placement::monitor_at_point;
use qol_gpui::platform::{ghost_window_decorations, ghost_window_kind, is_escape_held};
use qol_windowing::WindowRect;
use std::rc::Rc;
use std::sync::mpsc;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VIRTUAL_KEY, VK_LBUTTON, VK_RBUTTON,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};

use crate::capture::frozen_frame::FrozenFrame;
use crate::capture::space::CaptureKind;
use crate::ui::region_selector::{
    bounds_from_monitor, fallback_bounds, open_all, rect_from_bounds, select_region_blocking_with,
    ActiveBounds, GlobalPointer, HoverTargetSource, SelectorWindow, SelectorWindowOptions,
    SelectorWindowSources, SnapshotTargets,
};
use crate::Rect;

use super::display::{
    full_screen_bounds, logical_cursor, logical_rect, native_displays, NativeDisplay,
};

const MIN_TARGET_PX: i32 = 24;
const KEY_DOWN_MASK: u16 = 0x8000;

pub fn select_region(kind: CaptureKind, frozen_frame: Option<FrozenFrame>) -> Result<Option<Rect>> {
    select_region_blocking_with(move |tx, cx| {
        open_selectors(tx, true, kind, frozen_frame, cx);
    })
}

pub fn select_region_in_app(
    cx: &mut App,
    kind: CaptureKind,
    _cursor: Option<(ActiveMonitor, Option<Point<Pixels>>)>,
    _monitors: Vec<ActiveMonitor>,
    frozen_frame: Option<FrozenFrame>,
) -> Option<mpsc::Receiver<Option<Rect>>> {
    let (tx, rx) = mpsc::channel();
    open_selectors(tx, false, kind, frozen_frame, cx);
    Some(rx)
}

fn open_selectors(
    tx: mpsc::Sender<Option<Rect>>,
    quit_on_finish: bool,
    kind: CaptureKind,
    frozen_frame: Option<FrozenFrame>,
    cx: &mut App,
) {
    let screens = selector_screens(cx);
    let monitor_bounds = screens
        .iter()
        .map(|(_, bounds)| *bounds)
        .collect::<Vec<_>>();
    let cursor = Rc::new(CursorSource {
        displays: native_displays(),
        monitor_bounds: monitor_bounds.clone(),
    });
    let pointer = cursor.point();
    let active = pointer.and_then(|pointer| monitor_at_point(&monitor_bounds, pointer));
    let hover_target = snapshot_targets(&cursor.displays, &monitor_bounds);
    let default_target = pointer.and_then(|pointer| hover_target.as_ref()?.target_at(pointer));
    let sources = SelectorWindowSources {
        map_rect: Rc::new(Some),
        global_pointer: Some(cursor.clone()),
        cancel_signal: Some(Rc::new(is_escape_held)),
        active_bounds: Some(cursor),
        hover_target,
        frozen_frame,
    };
    let selectors = screens
        .into_iter()
        .map(|(display_id, bounds)| {
            SelectorWindow::new(
                bounds,
                monitor_bounds.clone(),
                active,
                default_target,
                SelectorWindowOptions {
                    display_id,
                    kind: ghost_window_kind(),
                    decorations: ghost_window_decorations(false),
                    focus: active.is_none_or(|active| active == bounds),
                },
                sources.clone(),
            )
        })
        .collect::<Vec<_>>();
    let titles = selectors
        .iter()
        .map(|selector| selector.title().to_string())
        .collect::<Vec<_>>();
    qol_runtime::probe!(
        "SHOT_SELECT_PLATFORM",
        "platform=windows selectors={} pointer={} quit_on_finish={quit_on_finish}",
        titles.len(),
        pointer.is_some()
    );
    if !open_all(tx, quit_on_finish, selectors, kind, cx) {
        if quit_on_finish {
            cx.quit();
        }
        return;
    }
    cx.defer(move |_| {
        for title in &titles {
            qol_gpui::popup_window::configure_overlay_window(title);
        }
    });
    cx.activate(true);
}

fn selector_screens(cx: &App) -> Vec<(Option<DisplayId>, Bounds<Pixels>)> {
    let screens = cx
        .displays()
        .iter()
        .map(|display| (Some(display.id()), display.bounds()))
        .collect::<Vec<_>>();
    if !screens.is_empty() {
        return screens;
    }
    let bounds = full_screen_bounds()
        .map(bounds_from_monitor)
        .unwrap_or_else(|_| fallback_bounds());
    vec![(None, bounds)]
}

fn snapshot_targets(
    displays: &[NativeDisplay],
    monitor_bounds: &[Bounds<Pixels>],
) -> Option<HoverTargetSource> {
    let own_pid = std::process::id();
    let include_frame = crate::config::load().capture.include_window_frame;
    let windows = qol_windowing::platform::windows::top_level_windows()
        .into_iter()
        .filter(|window| {
            window.is_switchable() && !window.is_minimized() && window.pid() != Some(own_pid)
        })
        .filter_map(|window| {
            if include_frame {
                window.frame()
            } else {
                window.client_frame()
            }
        })
        .filter_map(|frame| logical_rect(rect_from_frame(frame), displays))
        .filter(|rect| rect.w >= MIN_TARGET_PX && rect.h >= MIN_TARGET_PX)
        .collect::<Vec<_>>();
    let monitors = monitor_bounds
        .iter()
        .copied()
        .map(rect_from_bounds)
        .collect::<Vec<_>>();
    qol_runtime::probe!(
        "SHOT_SELECT_DISCOVERY",
        "platform=windows windows={} monitors={} frame={include_frame}",
        windows.len(),
        monitors.len()
    );
    if windows.is_empty() && monitors.is_empty() {
        return None;
    }
    Some(Rc::new(SnapshotTargets { windows, monitors }))
}

fn rect_from_frame(frame: WindowRect) -> Rect {
    Rect {
        x: frame.x.round() as i32,
        y: frame.y.round() as i32,
        w: frame.width.round() as i32,
        h: frame.height.round() as i32,
    }
}

struct CursorSource {
    displays: Vec<NativeDisplay>,
    monitor_bounds: Vec<Bounds<Pixels>>,
}

impl CursorSource {
    fn point(&self) -> Option<Point<Pixels>> {
        logical_cursor(&self.displays).map(|(x, y)| point(px(x), px(y)))
    }
}

impl GlobalPointer for CursorSource {
    fn position(&self) -> Option<Point<Pixels>> {
        self.point()
    }

    fn primary_button_down(&self) -> bool {
        key_down(primary_button())
    }
}

impl ActiveBounds for CursorSource {
    fn active_bounds(&self) -> Option<Bounds<Pixels>> {
        self.point()
            .and_then(|pointer| monitor_at_point(&self.monitor_bounds, pointer))
    }
}

fn primary_button() -> VIRTUAL_KEY {
    if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 {
        return VK_RBUTTON;
    }
    VK_LBUTTON
}

fn key_down(key: VIRTUAL_KEY) -> bool {
    let state = unsafe { GetAsyncKeyState(i32::from(key)) };
    state as u16 & KEY_DOWN_MASK != 0
}
