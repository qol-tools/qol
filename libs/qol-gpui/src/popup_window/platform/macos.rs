use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplication, NSColor, NSPopUpMenuWindowLevel, NSScreen, NSWindow, NSWindowAnimationBehavior,
};
use objc2_foundation::{MainThreadMarker, NSPoint};

#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(debug_assertions)]
static GHOST_DEBUG_ALPHA: AtomicU32 = AtomicU32::new(0);
#[cfg(debug_assertions)]
static GHOST_DEBUG_COLOR: AtomicU32 = AtomicU32::new(0x00FF00);

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
    #[cfg(debug_assertions)]
    {
        let alpha = f32::from_bits(GHOST_DEBUG_ALPHA.load(Ordering::Relaxed));
        if alpha > 0.0 {
            window.setBackgroundColor(Some(&debug_ghost_color()));
            window.setAlphaValue(alpha as f64);
            window.setIgnoresMouseEvents(true);
            return true;
        }
    }
    window.setAlphaValue(0.0);
    window.setIgnoresMouseEvents(true);
    true
}

pub fn set_ghost_debug(opacity: Option<f32>, color_hex: Option<&str>) {
    #[cfg(debug_assertions)]
    {
        GHOST_DEBUG_ALPHA.store(opacity.unwrap_or(0.0).to_bits(), Ordering::Relaxed);
        if let Some(rgb) = color_hex.and_then(parse_hex_rgb) {
            GHOST_DEBUG_COLOR.store(rgb, Ordering::Relaxed);
        }
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
fn debug_ghost_color() -> Retained<NSColor> {
    let rgb = GHOST_DEBUG_COLOR.load(Ordering::Relaxed);
    let r = ((rgb >> 16) & 0xFF) as f64 / 255.0;
    let g = ((rgb >> 8) & 0xFF) as f64 / 255.0;
    let b = (rgb & 0xFF) as f64 / 255.0;
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0)
}

pub fn show_window_by_title(title: &str) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = find_window_by_title(mtm, title) else {
        return false;
    };
    window.setIgnoresMouseEvents(false);
    window.makeKeyAndOrderFront(None);
    window.setAlphaValue(1.0);
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
            window.setBackgroundColor(Some(&debug_ghost_color()));
            return true;
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

pub fn dump_ghost_windows(_context: &str) {}

fn find_window_by_title(mtm: MainThreadMarker, title: &str) -> Option<Retained<NSWindow>> {
    let app = NSApplication::sharedApplication(mtm);
    app.windows()
        .iter()
        .find(|win| win.title().to_string() == title)
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
