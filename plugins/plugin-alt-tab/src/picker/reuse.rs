use super::GatheredWindows;
use crate::app::AltTabApp;
use crate::config::AltTabConfig;
use crate::shared::layout::*;
use gpui::*;
use qol_gpui::window::PopupPlacement;

pub(crate) struct ReuseLayout {
    pub bounds: Bounds<Pixels>,
    pub size: Size<Pixels>,
}

pub(crate) struct ReuseRequest<'a> {
    pub handle: &'a WindowHandle<AltTabApp>,
    pub layout: &'a ReuseLayout,
    pub config: &'a AltTabConfig,
    pub gathered: &'a GatheredWindows,
    pub reverse: bool,
}

pub(super) struct LayoutInput<'a> {
    pub config: &'a AltTabConfig,
    pub window_count: usize,
    pub placement: &'a PopupPlacement,
}

pub(super) fn try_reuse(req: &ReuseRequest, cx: &mut App) -> bool {
    // The picker window is pre-created at boot and kept alive across dismisses, so the
    // normal path updates an existing view instead of paying GPUI window creation cost.
    // Stale handles can still happen after platform failures; those fall through to create.
    req.handle
        .update(cx, |view, window: &mut Window, cx| {
            if !view.apply_reuse(req, window, cx) {
                return false;
            }
            let backing =
                qol_gpui::popup_window::window_backing_scale(super::create::PICKER_WINDOW_TITLE);
            qol_gpui::window::resize_or_sync_scale(window, req.layout.size, backing);
            window.focus(&view.focus_handle(cx));
            window.activate_window();
            super::platform::show_picker();
            true
        })
        .unwrap_or(false)
}

pub(super) fn compute_layout(input: &LayoutInput, cx: &mut App) -> ReuseLayout {
    let size = picker_size(input);
    let bounds = input.placement.centered_bounds(size, cx);
    ReuseLayout { bounds, size }
}

fn picker_size(input: &LayoutInput) -> Size<Pixels> {
    let count = input.window_count.max(1);
    let (w, h) = picker_dimensions(
        count,
        input.config.display.max_columns,
        input.placement.monitor_size(),
        input.config.display.show_hotkey_hints,
    );
    size(px(w), px(h))
}

#[cfg(test)]
mod tests {
    use gpui::{point, px, size, Bounds};
    use qol_gpui::window::MonitorKey;

    // Regression: origin-only divergence missed resolution shrinks while the monitor
    // stayed pinned at (0,0). MonitorKey::from_bounds(&centered_bounds) must encode
    // both origin AND size so the comparator catches every shape of monitor change.
    #[test]
    fn target_key_distinguishes_centered_bounds_changes() {
        // Each row: (label, before_bounds, after_bounds, must_differ).
        // before/after model the picker's centered placement bounds before and after the
        // monitor change. must_differ asserts whether the new comparator should fire.
        let cases = [
            (
                "resolution_shrink_at_origin",
                Bounds {
                    origin: point(px(810.0), px(442.0)),
                    size: size(px(300.0), px(200.0)),
                },
                Bounds {
                    origin: point(px(490.0), px(282.0)),
                    size: size(px(300.0), px(200.0)),
                },
                true,
            ),
            (
                "monitor_changed_origin_shift",
                Bounds {
                    origin: point(px(810.0), px(442.0)),
                    size: size(px(300.0), px(200.0)),
                },
                Bounds {
                    origin: point(px(2730.0), px(442.0)),
                    size: size(px(300.0), px(200.0)),
                },
                true,
            ),
            (
                "picker_size_grew_more_windows",
                Bounds {
                    origin: point(px(810.0), px(442.0)),
                    size: size(px(300.0), px(200.0)),
                },
                Bounds {
                    origin: point(px(700.0), px(370.0)),
                    size: size(px(520.0), px(344.0)),
                },
                true,
            ),
            (
                "identical_placement_no_change",
                Bounds {
                    origin: point(px(810.0), px(442.0)),
                    size: size(px(300.0), px(200.0)),
                },
                Bounds {
                    origin: point(px(810.0), px(442.0)),
                    size: size(px(300.0), px(200.0)),
                },
                false,
            ),
            (
                "subpixel_jitter_rounds_to_same_key",
                Bounds {
                    origin: point(px(810.0), px(442.0)),
                    size: size(px(300.0), px(200.0)),
                },
                Bounds {
                    origin: point(px(810.4), px(442.3)),
                    size: size(px(299.9), px(200.4)),
                },
                false,
            ),
        ];

        for (label, before, after, must_differ) in cases {
            let a = MonitorKey::from_bounds(&before);
            let b = MonitorKey::from_bounds(&after);
            if must_differ {
                assert_ne!(
                    a, b,
                    "{label}: comparator must trigger reposition (a={a:?} b={b:?})"
                );
            } else {
                assert_eq!(
                    a, b,
                    "{label}: comparator must NOT thrash on equivalent placement (a={a:?} b={b:?})"
                );
            }
        }
    }
}
