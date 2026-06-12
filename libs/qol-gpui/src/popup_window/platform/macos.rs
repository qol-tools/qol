use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplication, NSColor, NSPopUpMenuWindowLevel, NSScreen, NSWindow, NSWindowAnimationBehavior,
};
use objc2_foundation::{MainThreadMarker, NSPoint};

#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(debug_assertions)]
const GHOST_COLOR_UNSET: u32 = u32::MAX;

#[cfg(debug_assertions)]
static GHOST_DEBUG_ALPHA: AtomicU32 = AtomicU32::new(0);
#[cfg(debug_assertions)]
static GHOST_DEBUG_COLOR: AtomicU32 = AtomicU32::new(GHOST_COLOR_UNSET);

pub fn sync_window_layout(
    title: &str,
    window: &mut gpui::Window,
    origin: gpui::Point<gpui::Pixels>,
    size: gpui::Size<gpui::Pixels>,
) -> bool {
    let backing = window_backing_scale(title);
    crate::window::resize_or_sync_scale(window, size, backing);
    reposition_window_by_title(title, origin.x.to_f64(), origin.y.to_f64())
}

pub fn reposition_window_by_title(title: &str, gpui_x: f64, gpui_y: f64) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };

    let screens = NSScreen::screens(mtm);
    let primary_h = screens
        .iter()
        .next()
        .map(|s| s.frame().size.height)
        .unwrap_or(1080.0);
    let ns_point = NSPoint::new(gpui_x, primary_h - gpui_y);

    let Some(window) = find_window_by_title(mtm, title) else {
        return false;
    };
    window.setFrameTopLeftPoint(ns_point);
    sync_backing_properties(&window);
    true
}

pub fn hide_window_by_title(title: &str) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = find_window_by_title(mtm, title) else {
        return false;
    };
    window.setLevel(NSPopUpMenuWindowLevel);
    #[cfg(debug_assertions)]
    {
        let alpha = f32::from_bits(GHOST_DEBUG_ALPHA.load(Ordering::Relaxed));
        if alpha > 0.0 {
            match debug_ghost_color() {
                Some(color) => window.setBackgroundColor(Some(&color)),
                None => window.setBackgroundColor(Some(&NSColor::clearColor())),
            }
            window.setAlphaValue(alpha as f64);
            window.setIgnoresMouseEvents(true);
            qol_runtime::probe!(
                "HIDE_WIN",
                "title={title} path=rest alpha={alpha} reason={}",
                crate::popup_window::change_reason()
            );
            return true;
        }
    }
    window.setAlphaValue(0.0);
    window.setIgnoresMouseEvents(true);
    qol_runtime::probe!(
        "HIDE_WIN",
        "title={title} path=hidden reason={}",
        crate::popup_window::change_reason()
    );
    true
}

pub fn hide_invisible(title: &str) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = find_window_by_title(mtm, title) else {
        return false;
    };
    window.setLevel(NSPopUpMenuWindowLevel);
    window.setAlphaValue(0.0);
    window.setIgnoresMouseEvents(true);
    qol_runtime::probe!(
        "HIDE_WIN",
        "title={title} path=invisible reason={}",
        crate::popup_window::change_reason()
    );
    true
}

pub fn set_ghost_debug(opacity: Option<f32>, color_hex: Option<&str>) {
    #[cfg(debug_assertions)]
    {
        GHOST_DEBUG_ALPHA.store(opacity.unwrap_or(0.0).to_bits(), Ordering::Relaxed);
        let color = color_hex
            .and_then(parse_hex_rgb)
            .unwrap_or(GHOST_COLOR_UNSET);
        GHOST_DEBUG_COLOR.store(color, Ordering::Relaxed);
    }
    #[cfg(not(debug_assertions))]
    let _ = (opacity, color_hex);
}

#[cfg(debug_assertions)]
fn parse_hex_rgb(hex: &str) -> Option<u32> {
    let hex = hex.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

#[cfg(debug_assertions)]
fn debug_ghost_color() -> Option<Retained<NSColor>> {
    let rgb = GHOST_DEBUG_COLOR.load(Ordering::Relaxed);
    if rgb == GHOST_COLOR_UNSET {
        return None;
    }
    let r = ((rgb >> 16) & 0xFF) as f64 / 255.0;
    let g = ((rgb >> 8) & 0xFF) as f64 / 255.0;
    let b = (rgb & 0xFF) as f64 / 255.0;
    Some(NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0))
}

pub fn show_window_by_title(title: &str) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = find_window_by_title(mtm, title) else {
        return false;
    };
    window.setLevel(NSPopUpMenuWindowLevel);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setIgnoresMouseEvents(false);
    window.makeKeyAndOrderFront(None);
    window.setAlphaValue(1.0);
    qol_runtime::probe!(
        "SHOW_WIN",
        "title={title} reason={}",
        crate::popup_window::change_reason()
    );
    true
}

pub fn disable_window_shadow(title: &str) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = find_window_by_title(mtm, title) else {
        return false;
    };
    window.setHasShadow(false);
    #[cfg(debug_assertions)]
    {
        let alpha = f32::from_bits(GHOST_DEBUG_ALPHA.load(Ordering::Relaxed));
        if alpha > 0.0 {
            if let Some(color) = debug_ghost_color() {
                window.setBackgroundColor(Some(&color));
                return true;
            }
        }
    }
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    true
}

pub fn configure_popup_window(title: &str) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = find_window_by_title(mtm, title) else {
        return false;
    };
    window.setAnimationBehavior(NSWindowAnimationBehavior::None);
    window.setLevel(NSPopUpMenuWindowLevel);
    true
}

pub fn window_backing_scale(title: &str) -> Option<f32> {
    let mtm = MainThreadMarker::new()?;
    let window = find_window_by_title(mtm, title)?;
    Some(window.backingScaleFactor() as f32)
}

#[cfg(not(debug_assertions))]
pub fn dump_ghost_windows(_context: &str) {}

#[cfg(debug_assertions)]
pub fn dump_ghost_windows(context: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    for window in app.windows().iter() {
        let frame = window.frame();
        qol_runtime::probe!(
            "GHOST_DUMP",
            "ctx=({context}) title={:?} alpha={:.2} level={} mouse_ignored={} frame={}x{}@{},{}",
            window.title().to_string(),
            window.alphaValue(),
            window.level(),
            window.ignoresMouseEvents(),
            frame.size.width,
            frame.size.height,
            frame.origin.x,
            frame.origin.y
        );
    }
}

fn find_window_by_title(mtm: MainThreadMarker, title: &str) -> Option<Retained<NSWindow>> {
    let app = NSApplication::sharedApplication(mtm);
    let found = app
        .windows()
        .iter()
        .filter(|win| win.title().to_string() == title)
        .last();
    if found.is_none() {
        qol_runtime::probe!(
            "WIN_MISS",
            "title={title} reason={}",
            crate::popup_window::change_reason()
        );
    }
    found
}

fn sync_backing_properties(window: &NSWindow) {
    let Some(view) = window.contentView() else {
        return;
    };
    let Some(gpui_view) = view.subviews().firstObject() else {
        return;
    };
    gpui_view.viewDidChangeBackingProperties();
}
