use crate::picker::create::PICKER_WINDOW_TITLE;
use objc2::rc::Retained;
use objc2_app_kit::{NSPopUpMenuWindowLevel, NSWindow, NSWindowAnimationBehavior};
use objc2_foundation::MainThreadMarker;
#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(debug_assertions)]
static GHOST_ALPHA: AtomicU32 = AtomicU32::new(0);
#[cfg(debug_assertions)]
const GHOST_COLOR_DEFAULT: u32 = 0xFF0000;
#[cfg(debug_assertions)]
static GHOST_COLOR: AtomicU32 = AtomicU32::new(GHOST_COLOR_DEFAULT);

pub fn hide_picker() {
    with_picker_window(|win| {
        let alpha = {
            #[cfg(debug_assertions)]
            {
                f32::from_bits(GHOST_ALPHA.load(Ordering::Relaxed)) as f64
            }
            #[cfg(not(debug_assertions))]
            {
                0.0_f64
            }
        };
        win.setAlphaValue(alpha);
        win.setIgnoresMouseEvents(true);
    });
}

pub fn set_accessory_policy() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

pub fn picker_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::Normal
}

pub fn dismiss_picker(_window: &mut gpui::Window) {
    hide_picker();
}

pub fn reposition_picker_window(gpui_x: f64, gpui_y: f64) -> bool {
    use objc2_app_kit::NSScreen;
    use objc2_foundation::NSPoint;
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };

    // GPUI's coordinate origin is the top-left of the primary screen (the one
    // with the menu bar, i.e. NSScreen.screens[0]). Cocoa's global display
    // coordinate space is anchored to the bottom-left of that SAME screen.
    // Conversion: ns_y = primary_screen_height - gpui_y.
    //
    // Do NOT use `NSScreen::mainScreen()` - it returns "the screen containing
    // the window with keyboard focus", which moves with the picker and produces
    // an 89px (or whatever-secondary-monitor-tall) drift between show and ghost
    // because the two calls hit it on different screens.
    let screens = NSScreen::screens(mtm);
    let primary_h = screens
        .iter()
        .next()
        .map(|s| s.frame().size.height)
        .unwrap_or(1080.0);
    let ns_point = NSPoint::new(gpui_x, primary_h - gpui_y);

    let Some(window) = find_picker_window(mtm) else {
        return false;
    };
    window.setFrameTopLeftPoint(ns_point);
    sync_backing_properties(&window);
    #[cfg(debug_assertions)]
    {
        let f = window.frame();
        let actual_top_left_ns_y = f.origin.y + f.size.height;
        let actual_gpui_y = primary_h - actual_top_left_ns_y;
        let backing_scale = window.backingScaleFactor();
        let screen_scale = window
            .screen()
            .map(|screen| screen.backingScaleFactor())
            .unwrap_or(0.0);
        eprintln!(
            "[alt-tab/reposition] req gpui=({:.1},{:.1}) → ns=({:.1},{:.1}); actual frame ns_origin=({:.1},{:.1}) size={:.1}x{:.1} → gpui_top_left=({:.1},{:.1}) primary_h={:.1} backing_scale={:.1} screen_scale={:.1}",
            gpui_x,
            gpui_y,
            ns_point.x,
            ns_point.y,
            f.origin.x,
            f.origin.y,
            f.size.width,
            f.size.height,
            f.origin.x,
            actual_gpui_y,
            primary_h,
            backing_scale,
            screen_scale,
        );
    }
    true
}

pub fn picker_backing_scale() -> Option<f32> {
    let mtm = MainThreadMarker::new()?;
    let window = find_picker_window(mtm)?;
    Some(window.backingScaleFactor() as f32)
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

fn cg_event_flags() -> u64 {
    const K_CG_EVENT_SOURCE_STATE_COMBINED: i32 = 0;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceFlagsState(state_id: i32) -> u64;
    }

    unsafe { CGEventSourceFlagsState(K_CG_EVENT_SOURCE_STATE_COMBINED) }
}

pub fn is_modifier_held() -> bool {
    const K_CG_EVENT_FLAG_MASK_ALTERNATE: u64 = 0x0008_0000;
    cg_event_flags() & K_CG_EVENT_FLAG_MASK_ALTERNATE != 0
}

pub fn is_shift_held() -> bool {
    const K_CG_EVENT_FLAG_MASK_SHIFT: u64 = 0x0002_0000;
    cg_event_flags() & K_CG_EVENT_FLAG_MASK_SHIFT != 0
}

pub fn disable_window_shadow() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(window) = find_picker_window(mtm) else {
        return;
    };
    window.setHasShadow(false);
    apply_ghost_background(&window);
}

fn apply_ghost_background(win: &NSWindow) {
    use objc2_app_kit::NSColor;
    #[cfg(debug_assertions)]
    {
        let alpha = f32::from_bits(GHOST_ALPHA.load(Ordering::Relaxed));
        if alpha > 0.0 {
            let rgb = GHOST_COLOR.load(Ordering::Relaxed);
            let r = ((rgb >> 16) & 0xFF) as f64 / 255.0;
            let g = ((rgb >> 8) & 0xFF) as f64 / 255.0;
            let b = (rgb & 0xFF) as f64 / 255.0;
            let color = NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0);
            win.setBackgroundColor(Some(&color));
            return;
        }
    }
    win.setBackgroundColor(Some(&NSColor::clearColor()));
}

pub fn show_picker() {
    #[cfg(debug_assertions)]
    let t = std::time::Instant::now();
    with_picker_window(|win| {
        win.setIgnoresMouseEvents(false);
        win.makeKeyAndOrderFront(None);
        win.setAlphaValue(1.0);
    });
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/show] ignores=false+orderFront+alpha=1 took {}us",
        t.elapsed().as_micros()
    );
}

fn with_picker_window(body: impl FnOnce(&NSWindow)) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(window) = find_picker_window(mtm) else {
        return false;
    };
    body(&window);
    true
}

fn find_picker_window(mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
    use objc2_app_kit::NSApplication;
    let app = NSApplication::sharedApplication(mtm);
    app.windows()
        .iter()
        .find(|win| win.title().to_string() == PICKER_WINDOW_TITLE)
}

pub fn pre_create(
    config: &crate::config::AltTabConfig,
    current: &crate::PickerWindowState,
    cx: &mut gpui::App,
) {
    set_ghost_opacity(config.display.ghost_opacity);
    set_ghost_color(config.display.ghost_debug_color.as_deref());
    crate::picker::create::pre_create_offscreen(config, current, cx);
    with_picker_window(|win| {
        win.setAnimationBehavior(NSWindowAnimationBehavior::None);
        // Always-on-top: NSPopUpMenuWindowLevel (101) sits above normal + floating
        // windows but below screen savers. In release, alpha=0 + ignoresMouseEvents
        // makes the ghost invisible+click-through, so this is a no-op visually; in
        // debug builds with ghost_opacity > 0 it keeps the red ghost above other apps.
        win.setLevel(NSPopUpMenuWindowLevel);
        apply_ghost_background(win);
    });
}

pub fn offscreen_origin() -> (f64, f64) {
    (-32000.0, -32000.0)
}

pub fn set_ghost_opacity(opacity: Option<f32>) {
    #[cfg(debug_assertions)]
    GHOST_ALPHA.store(opacity.unwrap_or(0.0).to_bits(), Ordering::Relaxed);
    #[cfg(not(debug_assertions))]
    let _ = opacity;
}

pub fn set_ghost_color(hex: Option<&str>) {
    #[cfg(debug_assertions)]
    {
        let rgb = hex
            .and_then(crate::config::parse_hex_color)
            .map(|(r, g, b)| ((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
            .unwrap_or(GHOST_COLOR_DEFAULT);
        GHOST_COLOR.store(rgb, Ordering::Relaxed);
    }
    #[cfg(not(debug_assertions))]
    let _ = hex;
}

pub fn destroy_non_target_windows(
    _current: &crate::PickerWindowState,
    _target: qol_plugin_api::window::MonitorKey,
    _cx: &mut gpui::App,
) {
}

/// Drop the stale `ActiveWindows` slot so a subsequent `create_from_request` fallback doesn't
/// leave a dangling sentinel key. The keep-alive NSWindow is never destroyed.
pub fn discard_old_window(
    current: &crate::PickerWindowState,
    target: qol_plugin_api::window::MonitorKey,
    _handle: gpui::WindowHandle<crate::app::AltTabApp>,
    _cx: &mut gpui::App,
) {
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/open] keep-alive reuse failed; dropping stale slot {:?}",
        target
    );
    current.borrow_mut().remove(target);
}
