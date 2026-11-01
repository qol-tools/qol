use anyhow::Result;
use gpui::{point, px, App, Bounds, DisplayId, Pixels, Point};
use qol_gpui::monitor::ActiveMonitor;
use qol_gpui::placement::monitor_at_point;
use qol_gpui::platform::{ghost_window_decorations, ghost_window_kind, is_escape_held};
use qol_platform::native::input::key_held;
use qol_windowing::WindowRect;
use std::rc::Rc;
use std::sync::mpsc;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_LBUTTON, VK_RBUTTON};
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
    full_screen_bounds, logical_cursor, logical_rect, monitor_from_rect, native_displays,
    NativeDisplay,
};

const MIN_TARGET_PX: i32 = 24;

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

pub fn monitor_capture_rect(monitor: Bounds<Pixels>) -> Option<Rect> {
    logical_rect(rect_from_bounds(monitor), &native_displays())
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
        .map(|screen| screen.bounds)
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
        .map(|screen| {
            SelectorWindow::new(
                screen.bounds,
                monitor_bounds.clone(),
                active,
                default_target,
                SelectorWindowOptions {
                    display_id: screen.display_id,
                    placement: Some(screen.placement),
                    kind: ghost_window_kind(),
                    decorations: ghost_window_decorations(false),
                    focus: active.is_none_or(|active| active == screen.bounds),
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
            qol_gpui::popup_window::configure_overlay_window(title, false, false);
        }
    });
    cx.activate(true);
}

struct SelectorScreen {
    display_id: Option<DisplayId>,
    bounds: Bounds<Pixels>,
    placement: Bounds<Pixels>,
}

fn selector_screens(cx: &App) -> Vec<SelectorScreen> {
    let displays = native_displays();
    let screens = cx
        .displays()
        .iter()
        .filter_map(|screen| {
            let placement = screen.bounds();
            let index = u32::from(screen.id()) as usize;
            let display = display_placed_at(&displays, index, placement)?;
            Some(SelectorScreen {
                display_id: Some(screen.id()),
                bounds: bounds_from_monitor(monitor_from_rect(display.logical())),
                placement,
            })
        })
        .collect::<Vec<_>>();
    if !screens.is_empty() {
        return screens;
    }
    let bounds = full_screen_bounds()
        .map(bounds_from_monitor)
        .unwrap_or_else(|_| fallback_bounds());
    vec![SelectorScreen {
        display_id: None,
        bounds,
        placement: bounds,
    }]
}

fn display_placed_at(
    displays: &[NativeDisplay],
    index: usize,
    placement: Bounds<Pixels>,
) -> Option<NativeDisplay> {
    displays
        .get(index)
        .into_iter()
        .chain(displays)
        .copied()
        .find(|display| is_placed_at(display, placement))
}

fn is_placed_at(display: &NativeDisplay, placement: Bounds<Pixels>) -> bool {
    let physical = display.physical;
    let scaled = [physical.x, physical.y, physical.w, physical.h]
        .map(|value| (value as f32 / display.scale).round());
    let placed = [
        placement.origin.x,
        placement.origin.y,
        placement.size.width,
        placement.size.height,
    ]
    .map(|value| f32::from(value).round());
    scaled == placed
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
        key_held(primary_button())
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

#[cfg(test)]
mod tests {
    use gpui::{point, px, size, Bounds, Pixels};

    use super::display_placed_at;
    use crate::platform::windows::display::NativeDisplay;
    use crate::Rect;

    fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    fn display(x: i32, w: i32, h: i32, scale: f32) -> NativeDisplay {
        NativeDisplay {
            physical: Rect { x, y: 0, w, h },
            scale,
        }
    }

    #[test]
    fn a_selector_opens_on_the_display_gpui_placed_it_on() {
        let displays = [
            display(0, 1920, 1080, 1.0),
            display(1920, 1920, 1080, 1.0),
            display(3840, 3840, 2160, 2.0),
        ];
        let shared = bounds(1920.0, 0.0, 1920.0, 1080.0);
        let cases = [
            (
                "display at its own index",
                0,
                bounds(0.0, 0.0, 1920.0, 1080.0),
                Some(displays[0]),
            ),
            (
                "bounds shared by two displays",
                1,
                shared,
                Some(displays[1]),
            ),
            (
                "the other display with the same bounds",
                2,
                shared,
                Some(displays[2]),
            ),
            (
                "index out of step with the native list",
                0,
                shared,
                Some(displays[1]),
            ),
            (
                "selector space of a scaled display",
                2,
                bounds(3840.0, 0.0, 1920.0, 1080.0),
                None,
            ),
        ];
        for (name, index, placement, expected) in cases {
            assert_eq!(
                display_placed_at(&displays, index, placement),
                expected,
                "{name}"
            );
        }
    }
}
