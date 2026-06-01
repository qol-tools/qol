use super::objc::{cls, msg_ptr, msg_ptr_usize, msg_rect, msg_usize, sel, CGRect};

#[derive(Clone, Copy, Debug)]
pub(super) struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

fn primary_screen_height() -> f64 {
    unsafe {
        let screens = msg_ptr(cls("NSScreen"), sel("screens"));
        if screens.is_null() {
            return 0.0;
        }
        let count = msg_usize(screens, sel("count"));
        if count == 0 {
            return 0.0;
        }
        let primary = msg_ptr_usize(screens, sel("objectAtIndex:"), 0);
        let frame = msg_rect(primary, sel("frame"));
        frame.size.height
    }
}

fn cocoa_to_ax(frame: CGRect, primary_h: f64) -> Rect {
    Rect {
        x: frame.origin.x,
        y: primary_h - frame.origin.y - frame.size.height,
        w: frame.size.width,
        h: frame.size.height,
    }
}

pub(super) fn screen_for_point(cx: f64, cy: f64) -> Option<Rect> {
    unsafe {
        let primary_h = primary_screen_height();
        if primary_h == 0.0 {
            return None;
        }

        let screens = msg_ptr(cls("NSScreen"), sel("screens"));
        if screens.is_null() {
            return None;
        }

        let count = msg_usize(screens, sel("count"));
        let mut fallback = None;

        for i in 0..count {
            let screen = msg_ptr_usize(screens, sel("objectAtIndex:"), i);
            let vf = msg_rect(screen, sel("visibleFrame"));
            let ax = cocoa_to_ax(vf, primary_h);
            if fallback.is_none() {
                fallback = Some(ax);
            }
            if cx >= ax.x && cx < ax.x + ax.w && cy >= ax.y && cy < ax.y + ax.h {
                return Some(ax);
            }
        }

        fallback
    }
}

pub(super) fn all_screens_sorted() -> Vec<Rect> {
    unsafe {
        let primary_h = primary_screen_height();
        if primary_h == 0.0 {
            return vec![];
        }

        let screens = msg_ptr(cls("NSScreen"), sel("screens"));
        if screens.is_null() {
            return vec![];
        }

        let count = msg_usize(screens, sel("count"));
        let mut result = Vec::with_capacity(count);

        for i in 0..count {
            let screen = msg_ptr_usize(screens, sel("objectAtIndex:"), i);
            let vf = msg_rect(screen, sel("visibleFrame"));
            result.push(cocoa_to_ax(vf, primary_h));
        }

        result.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
        result
    }
}
