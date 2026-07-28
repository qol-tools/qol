use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use super::objc::{
    cls, msg_ptr, msg_ptr_usize, msg_rect, msg_usize, sel, CGDisplayBounds, CGGetActiveDisplayList,
    CGRect,
};
use super::trace::{timed_opt, trace_screen_snapshot};

const MAX_DISPLAYS: usize = 16;
static SCREEN_CACHE: OnceLock<Mutex<Option<ScreenSnapshot>>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct ScreenSnapshot {
    layout: Vec<Rect>,
    preferences: u64,
    visible: Vec<Rect>,
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
        let snapshot = screen_snapshot()?;
        screen_containing_point(&snapshot.visible, cx, cy)
            .or_else(|| snapshot.visible.first().copied())
    })
}

pub(super) fn all_screens_sorted() -> Vec<Rect> {
    let mut result = screen_snapshot()
        .map(|snapshot| snapshot.visible)
        .unwrap_or_default();
    result.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));
    result
}

fn screen_snapshot() -> Option<ScreenSnapshot> {
    let start = Instant::now();
    let layout = physical_screens();
    if layout.is_empty() {
        trace_screen_snapshot("unavailable", 0, start);
        return None;
    }
    let preferences = screen_preferences_generation();
    if let Some(snapshot) = memory_snapshot(&layout, preferences) {
        trace_screen_snapshot("memory", snapshot.visible.len(), start);
        return Some(snapshot);
    }
    if let Some(snapshot) = read_cached_snapshot()
        .filter(|cached| cached.layout == layout && cached.preferences == preferences)
    {
        store_memory_snapshot(snapshot.clone());
        trace_screen_snapshot("disk", snapshot.visible.len(), start);
        return Some(snapshot);
    }

    let visible = system_screens();
    let snapshot = ScreenSnapshot {
        layout,
        preferences,
        visible,
    };
    if !snapshot_covers_every_display(&snapshot) {
        trace_screen_snapshot("incomplete", snapshot.visible.len(), start);
        return None;
    }
    write_cached_snapshot(&snapshot);
    store_memory_snapshot(snapshot.clone());
    trace_screen_snapshot("nsscreen", snapshot.visible.len(), start);
    Some(snapshot)
}

fn physical_screens() -> Vec<Rect> {
    let mut ids = [0u32; MAX_DISPLAYS];
    let mut count = 0u32;
    let result =
        unsafe { CGGetActiveDisplayList(MAX_DISPLAYS as u32, ids.as_mut_ptr(), &mut count) };
    if result != 0 {
        return Vec::new();
    }
    let mut screens = ids[..count.min(MAX_DISPLAYS as u32) as usize]
        .iter()
        .map(|id| unsafe { CGDisplayBounds(*id) })
        .map(|bounds| Rect {
            x: bounds.origin.x,
            y: bounds.origin.y,
            w: bounds.size.width,
            h: bounds.size.height,
        })
        .filter(valid_rect)
        .collect::<Vec<_>>();
    screens.sort_by(rect_order);
    screens
}

fn screen_preferences_generation() -> u64 {
    let Some(home) = std::env::var_os("HOME") else {
        return 0;
    };
    let preferences = PathBuf::from(home).join("Library/Preferences");
    let mut hasher = DefaultHasher::new();
    for name in [
        "com.apple.dock.plist",
        ".GlobalPreferences.plist",
        "com.apple.controlcenter.plist",
    ] {
        name.hash(&mut hasher);
        let Ok(metadata) = fs::metadata(preferences.join(name)) else {
            continue;
        };
        metadata.len().hash(&mut hasher);
        metadata.modified().ok().hash(&mut hasher);
    }
    hasher.finish()
}

fn rect_order(a: &Rect, b: &Rect) -> std::cmp::Ordering {
    a.x.total_cmp(&b.x)
        .then_with(|| a.y.total_cmp(&b.y))
        .then_with(|| a.w.total_cmp(&b.w))
        .then_with(|| a.h.total_cmp(&b.h))
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
    std::env::temp_dir().join("qol-window-actions-screens-v2")
}

fn memory_snapshot(layout: &[Rect], preferences: u64) -> Option<ScreenSnapshot> {
    let cached = SCREEN_CACHE.get_or_init(|| Mutex::new(None));
    let snapshot = cached.lock().ok()?.as_ref()?.clone();
    (snapshot.layout == layout && snapshot.preferences == preferences).then_some(snapshot)
}

fn store_memory_snapshot(snapshot: ScreenSnapshot) {
    let cached = SCREEN_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(mut cached) = cached.lock() {
        *cached = Some(snapshot);
    }
}

fn read_cached_snapshot() -> Option<ScreenSnapshot> {
    let contents = fs::read_to_string(screen_cache_path()).ok()?;
    parse_cached_snapshot(&contents)
}

fn parse_cached_snapshot(contents: &str) -> Option<ScreenSnapshot> {
    let mut layout = Vec::new();
    let mut visible = Vec::new();
    let mut preferences = None;
    for line in contents.lines() {
        if let Some(value) = line.strip_prefix("preferences,") {
            if preferences.is_some() || value.contains(',') {
                return None;
            }
            preferences = value.parse().ok();
            continue;
        }
        let (kind, rect) = parse_cache_row(line)?;
        match kind {
            "layout" => layout.push(rect),
            "visible" => visible.push(rect),
            _ => return None,
        }
    }
    layout.sort_by(rect_order);
    let snapshot = ScreenSnapshot {
        layout,
        preferences: preferences?,
        visible,
    };
    snapshot_covers_every_display(&snapshot).then_some(snapshot)
}

/// AppKit can report fewer screens than the window server while a display is
/// waking or the daemon's cached screen list is stale. Such a snapshot maximizes
/// onto a work area that belongs to no real display and makes monitor moves a
/// silent no-op, so it must never be trusted or persisted.
fn snapshot_covers_every_display(snapshot: &ScreenSnapshot) -> bool {
    !snapshot.layout.is_empty() && snapshot.visible.len() == snapshot.layout.len()
}

fn parse_cache_row(line: &str) -> Option<(&str, Rect)> {
    let mut fields = line.split(',');
    let kind = fields.next()?;
    let rect = Rect {
        x: fields.next()?.parse().ok()?,
        y: fields.next()?.parse().ok()?,
        w: fields.next()?.parse().ok()?,
        h: fields.next()?.parse().ok()?,
    };
    if fields.next().is_some() || !valid_rect(&rect) {
        return None;
    }
    Some((kind, rect))
}

fn valid_rect(rect: &Rect) -> bool {
    [rect.x, rect.y, rect.w, rect.h]
        .iter()
        .all(|value| value.is_finite())
        && rect.w > 0.0
        && rect.h > 0.0
}

fn write_cached_snapshot(snapshot: &ScreenSnapshot) {
    let mut contents = format!("preferences,{}\n", snapshot.preferences);
    append_cache_rows(&mut contents, "layout", &snapshot.layout);
    append_cache_rows(&mut contents, "visible", &snapshot.visible);
    let _ = qol_fs::atomic_write(&screen_cache_path(), contents.as_bytes());
}

fn append_cache_rows(contents: &mut String, kind: &str, screens: &[Rect]) {
    for screen in screens {
        contents.push_str(&format!(
            "{kind},{},{},{},{}\n",
            screen.x, screen.y, screen.w, screen.h
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn cached_snapshot_round_trips_layout_and_visible_frames() {
        let snapshot = ScreenSnapshot {
            layout: vec![
                rect(1440.0, 0.0, 1920.0, 1080.0),
                rect(0.0, 0.0, 1440.0, 900.0),
            ],
            preferences: 42,
            visible: vec![
                rect(0.0, 25.0, 1440.0, 875.0),
                rect(1440.0, 0.0, 1920.0, 1080.0),
            ],
        };
        let mut contents = format!("preferences,{}\n", snapshot.preferences);
        append_cache_rows(&mut contents, "layout", &snapshot.layout);
        append_cache_rows(&mut contents, "visible", &snapshot.visible);
        let parsed = parse_cached_snapshot(&contents).unwrap();
        assert_eq!(
            parsed.layout,
            vec![
                rect(0.0, 0.0, 1440.0, 900.0),
                rect(1440.0, 0.0, 1920.0, 1080.0)
            ]
        );
        assert_eq!(parsed.visible, snapshot.visible);
    }

    #[test]
    fn cached_snapshot_rejects_invalid_contracts() {
        let cases = [
            "",
            "preferences,42\nlayout,0,0,100,100\n",
            "preferences,42\nvisible,0,0,100,100\n",
            "preferences,nope\nlayout,0,0,100,100\nvisible,0,0,100,100\n",
            "preferences,42,extra\nlayout,0,0,100,100\nvisible,0,0,100,100\n",
            "preferences,42\npreferences,43\nlayout,0,0,100,100\nvisible,0,0,100,100\n",
            "layout,0,0,100,100\n",
            "visible,0,0,100,100\n",
            "other,0,0,100,100\nvisible,0,0,100,100\n",
            "layout,NaN,0,100,100\nvisible,0,0,100,100\n",
            "layout,0,0,0,100\nvisible,0,0,100,100\n",
            "layout,0,0,100\nvisible,0,0,100,100\n",
            "layout,0,0,100,100,extra\nvisible,0,0,100,100\n",
            "preferences,42\nlayout,0,0,100,100\nlayout,100,0,100,100\nvisible,0,0,100,100\n",
            "preferences,42\nlayout,0,0,100,100\nvisible,0,0,100,100\nvisible,100,0,100,100\n",
        ];
        for contents in cases {
            assert!(parse_cached_snapshot(contents).is_none(), "{contents:?}");
        }
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
