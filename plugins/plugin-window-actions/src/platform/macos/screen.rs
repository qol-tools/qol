use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use super::objc::{cls, msg_ptr, msg_ptr_usize, msg_rect, msg_usize, sel, CGRect};
use super::trace::timed_opt;

const SCREEN_CACHE_TTL: Duration = Duration::from_secs(10);

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
    timed_opt("screen_for_point", 0, || {
        if let Some(screens) = read_cached_screens() {
            if let Some(screen) = screen_containing_point(&screens, cx, cy) {
                return Some(screen);
            }
        }

        let screens = system_screens();
        if screens.is_empty() {
            return None;
        }

        write_cached_screens(&screens);
        screen_containing_point(&screens, cx, cy).or_else(|| screens.first().copied())
    })
}

pub(super) fn all_screens_sorted() -> Vec<Rect> {
    let mut result = system_screens();
    write_cached_screens(&result);
    result.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    result
}

fn system_screens() -> Vec<Rect> {
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

        result
    }
}

fn screen_containing_point(screens: &[Rect], cx: f64, cy: f64) -> Option<Rect> {
    screens
        .iter()
        .copied()
        .find(|s| cx >= s.x && cx < s.x + s.w && cy >= s.y && cy < s.y + s.h)
}

fn screen_cache_path() -> PathBuf {
    std::env::temp_dir().join("qol-window-actions-screens-v1")
}

fn read_cached_screens() -> Option<Vec<Rect>> {
    let path = screen_cache_path();
    let metadata = fs::metadata(&path).ok()?;
    let modified = metadata.modified().ok()?;
    if SystemTime::now().duration_since(modified).ok()? > SCREEN_CACHE_TTL {
        return None;
    }

    let contents = fs::read_to_string(path).ok()?;
    parse_cached_screens(&contents)
}

fn parse_cached_screens(contents: &str) -> Option<Vec<Rect>> {
    let mut screens = Vec::new();
    for line in contents.lines() {
        let mut fields = line.split(',');
        let x: f64 = fields.next()?.parse().ok()?;
        let y: f64 = fields.next()?.parse().ok()?;
        let w: f64 = fields.next()?.parse().ok()?;
        let h: f64 = fields.next()?.parse().ok()?;
        if fields.next().is_some()
            || ![x, y, w, h].iter().all(|value| value.is_finite())
            || w <= 0.0
            || h <= 0.0
        {
            return None;
        }
        screens.push(Rect { x, y, w, h });
    }

    if screens.is_empty() {
        return None;
    }
    Some(screens)
}

fn write_cached_screens(screens: &[Rect]) {
    if screens.is_empty() {
        return;
    }

    let path = screen_cache_path();
    let contents = screens
        .iter()
        .map(|s| format!("{},{},{},{}\n", s.x, s.y, s.w, s.h))
        .collect::<String>();
    let _ = qol_fs::atomic_write(&path, contents.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn parse_cached_screens_accepts_valid_rows() {
        let screens = parse_cached_screens("0,25,1440,875\n1440,0,1920,1080\n").unwrap();
        assert_eq!(screens.len(), 2);
        assert_eq!(screens[0].x, 0.0);
        assert_eq!(screens[0].y, 25.0);
        assert_eq!(screens[0].w, 1440.0);
        assert_eq!(screens[0].h, 875.0);
    }

    #[test]
    fn parse_cached_screens_rejects_invalid_rows() {
        assert!(parse_cached_screens("").is_none());
        assert!(parse_cached_screens("NaN,0,100,100\n").is_none());
        assert!(parse_cached_screens("0,0,0,100\n").is_none());
        assert!(parse_cached_screens("0,0,100\n").is_none());
        assert!(parse_cached_screens("0,0,100,100,extra\n").is_none());
    }

    #[test]
    fn screen_containing_point_matches_visible_rects() {
        let screens = [rect(0.0, 0.0, 100.0, 100.0), rect(120.0, 0.0, 100.0, 100.0)];
        assert_eq!(
            screen_containing_point(&screens, 50.0, 50.0).map(|screen| screen.x),
            Some(0.0)
        );
        assert_eq!(
            screen_containing_point(&screens, 150.0, 50.0).map(|screen| screen.x),
            Some(120.0)
        );
        assert!(screen_containing_point(&screens, 110.0, 50.0).is_none());
    }
}
