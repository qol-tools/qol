use std::fmt;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use qol_process::{run_owned_with_output_timeout, BoundedCommandOutput};

use super::objc::{
    cls, msg_ptr, msg_ptr_usize, msg_rect, msg_usize, sel, CGDisplayBounds, CGGetActiveDisplayList,
    CGMainDisplayID, CGRect,
};
use super::trace::{timed_result, trace_screen_snapshot};

pub(super) const SCREENS_ACTION: &str = "print-work-areas";
const SCREEN_HELPER_ENV: &str = "QOL_WINDOW_ACTIONS_SCREEN_HELPER";
const MAX_DISPLAYS: usize = 16;
const HELPER_TIMEOUT: Duration = Duration::from_secs(3);
const HELPER_OUTPUT_LIMIT: usize = 64 * 1024;
static SCREEN_CACHE: SnapshotCache = SnapshotCache::new();
static APPKIT_SCREENS: AppKitScreens = AppKitScreens::new();

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct DisplayKey {
    displays: Vec<Rect>,
    preferences: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct ScreenSnapshot {
    key: DisplayKey,
    visible: Vec<Rect>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkAreaSource {
    AppKit,
    Helper,
}

impl WorkAreaSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::AppKit => "appkit",
            Self::Helper => "helper",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Relay {
    Allowed,
    Forbidden,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum DisplayError {
    DisplayListUnreadable(i32),
    NoActiveDisplay,
    StaleAppKit,
    HelperFailed(String),
    Uncovered {
        source: WorkAreaSource,
        displays: usize,
        work_areas: usize,
    },
}

impl DisplayError {
    fn trace_name(&self) -> &'static str {
        match self {
            Self::DisplayListUnreadable(_) => "unreadable",
            Self::NoActiveDisplay => "unavailable",
            Self::StaleAppKit => "stale",
            Self::HelperFailed(_) => "helper-failed",
            Self::Uncovered { .. } => "incomplete",
        }
    }
}

impl fmt::Display for DisplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DisplayListUnreadable(code) => write!(
                f,
                "the window server could not list the active displays (CGError {code})"
            ),
            Self::NoActiveDisplay => f.write_str("the window server reports no active display"),
            Self::StaleAppKit => {
                f.write_str("AppKit's screen list predates the current display layout")
            }
            Self::HelperFailed(reason) => write!(
                f,
                "a fresh process could not read the display work areas: {reason}"
            ),
            Self::Uncovered {
                source,
                displays,
                work_areas,
            } => write!(
                f,
                "the {} work areas ({work_areas}) do not match the {displays} active display(s)",
                source.as_str()
            ),
        }
    }
}

/// AppKit freezes `NSScreen.screens` at a process's first read and never refreshes it,
/// not even with a run loop, so a daemon that outlives a docking change keeps the old
/// screens. Its list is trusted only under the display layout seen at that first read;
/// every other layout is read by a newly started copy of this binary.
struct AppKitScreens {
    frozen_under: OnceLock<DisplayKey>,
}

impl AppKitScreens {
    const fn new() -> Self {
        Self {
            frozen_under: OnceLock::new(),
        }
    }

    fn read(&self, key: &DisplayKey, system: impl FnOnce() -> Vec<Rect>) -> Option<Vec<Rect>> {
        let frozen_under = self.frozen_under.get_or_init(|| key.clone());
        (frozen_under == key).then(system)
    }
}

struct SnapshotCache {
    latest: Mutex<Option<ScreenSnapshot>>,
}

impl SnapshotCache {
    const fn new() -> Self {
        Self {
            latest: Mutex::new(None),
        }
    }

    fn get(&self, key: &DisplayKey) -> Option<ScreenSnapshot> {
        let latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        latest
            .as_ref()
            .filter(|snapshot| snapshot.key == *key)
            .cloned()
    }

    fn store(&self, snapshot: ScreenSnapshot) {
        *self.latest.lock().unwrap_or_else(PoisonError::into_inner) = Some(snapshot);
    }
}

pub(super) fn screen_for_point(cx: f64, cy: f64) -> Result<Rect, DisplayError> {
    timed_result("screen_for_point", 0, || {
        let snapshot = screen_snapshot()?;
        screen_containing_point(&snapshot.visible, cx, cy)
            .or_else(|| snapshot.visible.first().copied())
            .ok_or(DisplayError::NoActiveDisplay)
    })
}

pub(super) fn all_screens_sorted() -> Result<Vec<Rect>, DisplayError> {
    let mut screens = screen_snapshot()?.visible;
    screens.sort_by(|a, b| a.x.total_cmp(&b.x));
    Ok(screens)
}

fn screen_snapshot() -> Result<ScreenSnapshot, DisplayError> {
    let start = Instant::now();
    match current_snapshot() {
        Ok((source, snapshot)) => {
            trace_screen_snapshot(source, snapshot.visible.len(), start);
            Ok(snapshot)
        }
        Err(error) => {
            trace_screen_snapshot(error.trace_name(), 0, start);
            Err(error)
        }
    }
}

fn current_snapshot() -> Result<(&'static str, ScreenSnapshot), DisplayError> {
    let key = DisplayKey {
        displays: active_displays()?,
        preferences: screen_preferences_generation(),
    };
    if let Some(snapshot) = SCREEN_CACHE.get(&key) {
        return Ok(("memory", snapshot));
    }
    if let Some(snapshot) = read_cached_snapshot().filter(|cached| cached.key == key) {
        SCREEN_CACHE.store(snapshot.clone());
        return Ok(("disk", snapshot));
    }
    let (source, snapshot) = build_snapshot(
        key,
        relay(),
        |key| APPKIT_SCREENS.read(key, system_screens),
        helper_screens,
    )?;
    write_cached_snapshot(&snapshot);
    SCREEN_CACHE.store(snapshot.clone());
    Ok((source.as_str(), snapshot))
}

fn build_snapshot(
    key: DisplayKey,
    relay: Relay,
    appkit: impl FnOnce(&DisplayKey) -> Option<Vec<Rect>>,
    helper: impl FnOnce() -> Result<Vec<Rect>, String>,
) -> Result<(WorkAreaSource, ScreenSnapshot), DisplayError> {
    let rejected = match appkit_snapshot(&key, appkit) {
        Ok(snapshot) => return Ok((WorkAreaSource::AppKit, snapshot)),
        Err(rejected) => rejected,
    };
    if relay == Relay::Forbidden {
        return Err(rejected);
    }
    let visible = helper().map_err(DisplayError::HelperFailed)?;
    covering(WorkAreaSource::Helper, key, visible)
        .map(|snapshot| (WorkAreaSource::Helper, snapshot))
}

fn appkit_snapshot(
    key: &DisplayKey,
    appkit: impl FnOnce(&DisplayKey) -> Option<Vec<Rect>>,
) -> Result<ScreenSnapshot, DisplayError> {
    let visible = appkit(key).ok_or(DisplayError::StaleAppKit)?;
    covering(WorkAreaSource::AppKit, key.clone(), visible)
}

fn covering(
    source: WorkAreaSource,
    key: DisplayKey,
    visible: Vec<Rect>,
) -> Result<ScreenSnapshot, DisplayError> {
    let snapshot = ScreenSnapshot { key, visible };
    if snapshot_covers_every_display(&snapshot) {
        return Ok(snapshot);
    }
    Err(DisplayError::Uncovered {
        source,
        displays: snapshot.key.displays.len(),
        work_areas: snapshot.visible.len(),
    })
}

fn active_displays() -> Result<Vec<Rect>, DisplayError> {
    let mut ids = [0u32; MAX_DISPLAYS];
    let mut count = 0u32;
    let result =
        unsafe { CGGetActiveDisplayList(MAX_DISPLAYS as u32, ids.as_mut_ptr(), &mut count) };
    if result != 0 {
        return Err(DisplayError::DisplayListUnreadable(result));
    }
    let mut displays = ids[..count.min(MAX_DISPLAYS as u32) as usize]
        .iter()
        .map(|id| rect_from(unsafe { CGDisplayBounds(*id) }))
        .filter(valid_rect)
        .collect::<Vec<_>>();
    if displays.is_empty() {
        return Err(DisplayError::NoActiveDisplay);
    }
    displays.sort_by(rect_order);
    Ok(displays)
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

fn relay() -> Relay {
    if std::env::var_os(SCREEN_HELPER_ENV).is_some() {
        Relay::Forbidden
    } else {
        Relay::Allowed
    }
}

fn helper_screens() -> Result<Vec<Rect>, String> {
    let exe =
        std::env::current_exe().map_err(|error| format!("cannot locate this binary: {error}"))?;
    let mut command = Command::new(exe);
    command.arg(SCREENS_ACTION).env(SCREEN_HELPER_ENV, "1");
    let output = run_owned_with_output_timeout(command, HELPER_TIMEOUT, HELPER_OUTPUT_LIMIT)
        .map_err(|error| format!("cannot run it: {error}"))?;
    let completed = match output {
        BoundedCommandOutput::Completed(completed) => completed,
        BoundedCommandOutput::TimedOut { .. } => {
            return Err(format!("no answer within {}s", HELPER_TIMEOUT.as_secs()));
        }
    };
    if !completed.status.success() {
        return Err(format!(
            "{}: {}",
            completed.status,
            last_line(completed.stderr.as_bytes())
        ));
    }
    Ok(parse_work_area_rows(&String::from_utf8_lossy(
        completed.stdout.as_bytes(),
    )))
}

fn last_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("no stderr")
        .trim()
        .to_string()
}

fn parse_work_area_rows(stdout: &str) -> Vec<Rect> {
    stdout.lines().filter_map(parse_work_area_row).collect()
}

fn parse_work_area_row(line: &str) -> Option<Rect> {
    let mut fields = line.trim().split(',');
    let rect = Rect {
        x: fields.next()?.trim().parse().ok()?,
        y: fields.next()?.trim().parse().ok()?,
        w: fields.next()?.trim().parse().ok()?,
        h: fields.next()?.trim().parse().ok()?,
    };
    (fields.next().is_none() && valid_rect(&rect)).then_some(rect)
}

pub(super) fn system_screens() -> Vec<Rect> {
    let frames = appkit_visible_frames();
    let Some(main_display_height) = main_display_height() else {
        return Vec::new();
    };
    frames
        .into_iter()
        .map(|frame| cocoa_to_global(frame, main_display_height))
        .collect()
}

fn appkit_visible_frames() -> Vec<Rect> {
    unsafe {
        let screens = msg_ptr(cls("NSScreen"), sel("screens"));
        if screens.is_null() {
            return Vec::new();
        }
        (0..msg_usize(screens, sel("count")))
            .map(|index| msg_ptr_usize(screens, sel("objectAtIndex:"), index))
            .map(|screen| rect_from(msg_rect(screen, sel("visibleFrame"))))
            .collect()
    }
}

fn main_display_height() -> Option<f64> {
    let height = unsafe { CGDisplayBounds(CGMainDisplayID()) }.size.height;
    (height.is_finite() && height > 0.0).then_some(height)
}

fn cocoa_to_global(frame: Rect, main_display_height: f64) -> Rect {
    Rect {
        y: main_display_height - frame.y - frame.h,
        ..frame
    }
}

fn rect_from(frame: CGRect) -> Rect {
    Rect {
        x: frame.origin.x,
        y: frame.origin.y,
        w: frame.size.width,
        h: frame.size.height,
    }
}

fn screen_containing_point(screens: &[Rect], cx: f64, cy: f64) -> Option<Rect> {
    screens
        .iter()
        .copied()
        .find(|s| cx >= s.x && cx < s.x + s.w && cy >= s.y && cy < s.y + s.h)
}

fn screen_cache_path() -> PathBuf {
    std::env::temp_dir().join("qol-window-actions-screens-v3")
}

fn read_cached_snapshot() -> Option<ScreenSnapshot> {
    let contents = fs::read_to_string(screen_cache_path()).ok()?;
    parse_cached_snapshot(&contents)
}

fn parse_cached_snapshot(contents: &str) -> Option<ScreenSnapshot> {
    let mut displays = Vec::new();
    let mut visible = Vec::new();
    let mut preferences = None;
    for line in contents.lines() {
        if let Some(value) = line.strip_prefix("preferences,") {
            if preferences.is_some() || value.contains(',') {
                return None;
            }
            preferences = Some(value.parse::<u64>().ok()?);
            continue;
        }
        let (kind, rect) = parse_cache_row(line)?;
        match kind {
            "layout" => displays.push(rect),
            "visible" => visible.push(rect),
            _ => return None,
        }
    }
    displays.sort_by(rect_order);
    let snapshot = ScreenSnapshot {
        key: DisplayKey {
            displays,
            preferences: preferences?,
        },
        visible,
    };
    snapshot_covers_every_display(&snapshot).then_some(snapshot)
}

/// A stale AppKit screen list describes displays that no longer exist, which makes
/// windows maximize onto a work area belonging to nothing. Counting screens is not
/// enough, because changing a display's scaling keeps the count and only changes the
/// sizes, so pair each work area with a distinct display that actually contains it.
fn snapshot_covers_every_display(snapshot: &ScreenSnapshot) -> bool {
    let displays = &snapshot.key.displays;
    if displays.is_empty() || snapshot.visible.len() != displays.len() {
        return false;
    }
    let mut claimed = vec![false; displays.len()];
    for work_area in &snapshot.visible {
        let paired = displays
            .iter()
            .enumerate()
            .find(|(index, display)| !claimed[*index] && display_contains(display, work_area));
        let Some((index, _)) = paired else {
            return false;
        };
        claimed[index] = true;
    }
    true
}

fn display_contains(display: &Rect, work_area: &Rect) -> bool {
    const EDGE_TOLERANCE: f64 = 1.0;
    work_area.x >= display.x - EDGE_TOLERANCE
        && work_area.y >= display.y - EDGE_TOLERANCE
        && work_area.x + work_area.w <= display.x + display.w + EDGE_TOLERANCE
        && work_area.y + work_area.h <= display.y + display.h + EDGE_TOLERANCE
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
    let _ = qol_fs::atomic_write(
        &screen_cache_path(),
        cached_snapshot_contents(snapshot).as_bytes(),
    );
}

fn cached_snapshot_contents(snapshot: &ScreenSnapshot) -> String {
    let mut contents = format!("preferences,{}\n", snapshot.key.preferences);
    append_cache_rows(&mut contents, "layout", &snapshot.key.displays);
    append_cache_rows(&mut contents, "visible", &snapshot.visible);
    contents
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
    use std::cell::Cell;

    use proptest::prelude::*;

    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    fn key(displays: &[Rect], preferences: u64) -> DisplayKey {
        DisplayKey {
            displays: displays.to_vec(),
            preferences,
        }
    }

    struct Layout {
        displays: Vec<Rect>,
        work_areas: Vec<Rect>,
    }

    fn laptop_alone() -> Layout {
        Layout {
            displays: vec![rect(0.0, 0.0, 1512.0, 982.0)],
            work_areas: vec![rect(0.0, 33.0, 1512.0, 949.0)],
        }
    }

    fn docked_left() -> Layout {
        Layout {
            displays: vec![
                rect(-1512.0, 458.0, 1512.0, 982.0),
                rect(0.0, 0.0, 2560.0, 1440.0),
            ],
            work_areas: vec![
                rect(0.0, 25.0, 2560.0, 1415.0),
                rect(-1512.0, 491.0, 1512.0, 949.0),
            ],
        }
    }

    fn docked_right() -> Layout {
        Layout {
            displays: vec![
                rect(0.0, 0.0, 2560.0, 1440.0),
                rect(2560.0, 458.0, 1512.0, 982.0),
            ],
            work_areas: vec![
                rect(0.0, 25.0, 2560.0, 1415.0),
                rect(2560.0, 491.0, 1512.0, 949.0),
            ],
        }
    }

    struct Case {
        name: &'static str,
        relay: Relay,
        appkit: Option<Vec<Rect>>,
        helper: Result<Vec<Rect>, String>,
        expected: Result<WorkAreaSource, DisplayError>,
        helper_runs: usize,
    }

    #[test]
    fn work_areas_come_from_appkit_only_while_they_cover_the_live_displays() {
        let layout = docked_right();
        let key = key(&layout.displays, 7);
        let old_arrangement = docked_left().work_areas;
        let cases = [
            Case {
                name: "current appkit list",
                relay: Relay::Allowed,
                appkit: Some(layout.work_areas.clone()),
                helper: Err("unused".into()),
                expected: Ok(WorkAreaSource::AppKit),
                helper_runs: 0,
            },
            Case {
                name: "appkit frozen under another layout",
                relay: Relay::Allowed,
                appkit: None,
                helper: Ok(layout.work_areas.clone()),
                expected: Ok(WorkAreaSource::Helper),
                helper_runs: 1,
            },
            Case {
                name: "same count in the old arrangement",
                relay: Relay::Allowed,
                appkit: Some(old_arrangement.clone()),
                helper: Ok(layout.work_areas.clone()),
                expected: Ok(WorkAreaSource::Helper),
                helper_runs: 1,
            },
            Case {
                name: "helper fails",
                relay: Relay::Allowed,
                appkit: None,
                helper: Err("no answer within 3s".into()),
                expected: Err(DisplayError::HelperFailed("no answer within 3s".into())),
                helper_runs: 1,
            },
            Case {
                name: "helper misses a display",
                relay: Relay::Allowed,
                appkit: Some(old_arrangement.clone()),
                helper: Ok(laptop_alone().work_areas),
                expected: Err(DisplayError::Uncovered {
                    source: WorkAreaSource::Helper,
                    displays: 2,
                    work_areas: 1,
                }),
                helper_runs: 1,
            },
            Case {
                name: "the helper itself never relays an uncovered list",
                relay: Relay::Forbidden,
                appkit: Some(old_arrangement),
                helper: Ok(layout.work_areas.clone()),
                expected: Err(DisplayError::Uncovered {
                    source: WorkAreaSource::AppKit,
                    displays: 2,
                    work_areas: 2,
                }),
                helper_runs: 0,
            },
            Case {
                name: "the helper itself never relays a stale list",
                relay: Relay::Forbidden,
                appkit: None,
                helper: Ok(layout.work_areas.clone()),
                expected: Err(DisplayError::StaleAppKit),
                helper_runs: 0,
            },
        ];
        for case in cases {
            let runs = Cell::new(0);
            let built = build_snapshot(
                key.clone(),
                case.relay,
                |_| case.appkit,
                || {
                    runs.set(runs.get() + 1);
                    case.helper
                },
            );
            let expected = case.expected.map(|source| {
                let snapshot = ScreenSnapshot {
                    key: key.clone(),
                    visible: layout.work_areas.clone(),
                };
                (source, snapshot)
            });
            assert_eq!(built, expected, "{}", case.name);
            assert_eq!(runs.get(), case.helper_runs, "{}", case.name);
        }
    }

    #[test]
    fn a_daemon_follows_every_layout_after_the_one_it_started_under() {
        let appkit = AppKitScreens::new();
        let frozen_at_startup = docked_left().work_areas;
        let sequence = [
            (
                "startup at the first desk",
                docked_left(),
                WorkAreaSource::AppKit,
            ),
            ("undocked", laptop_alone(), WorkAreaSource::Helper),
            (
                "docked at another desk",
                docked_right(),
                WorkAreaSource::Helper,
            ),
            (
                "back at the first desk",
                docked_left(),
                WorkAreaSource::AppKit,
            ),
        ];
        for (name, layout, source) in sequence {
            let key = key(&layout.displays, 7);
            let built = build_snapshot(
                key.clone(),
                Relay::Allowed,
                |key| appkit.read(key, || frozen_at_startup.clone()),
                || Ok(layout.work_areas.clone()),
            );
            let snapshot = ScreenSnapshot {
                key,
                visible: layout.work_areas,
            };
            assert_eq!(built, Ok((source, snapshot)), "{name}");
        }
    }

    #[test]
    fn appkit_is_read_only_under_the_layout_of_its_first_read() {
        let appkit = AppKitScreens::new();
        let reads = Cell::new(0);
        let startup = key(&laptop_alone().displays, 7);
        let cases = [
            ("first read", startup.clone(), true),
            ("same layout", startup.clone(), true),
            ("docked", key(&docked_left().displays, 7), false),
            (
                "dock preferences changed",
                key(&laptop_alone().displays, 8),
                false,
            ),
            ("undocked again", startup, true),
        ];
        for (name, key, trusted) in cases {
            let read = appkit.read(&key, || {
                reads.set(reads.get() + 1);
                laptop_alone().work_areas
            });
            assert_eq!(read.is_some(), trusted, "{name}");
        }
        assert_eq!(reads.get(), 3);
    }

    #[test]
    fn the_cached_snapshot_serves_only_its_own_layout_and_preferences() {
        let cache = SnapshotCache::new();
        let docked = ScreenSnapshot {
            key: key(&docked_left().displays, 7),
            visible: docked_left().work_areas,
        };
        assert_eq!(cache.get(&docked.key), None, "empty");
        cache.store(docked.clone());
        let cases = [
            ("same layout", key(&docked_left().displays, 7), Some(docked)),
            ("rearranged", key(&docked_right().displays, 7), None),
            ("undocked", key(&laptop_alone().displays, 7), None),
            (
                "dock preferences changed",
                key(&docked_left().displays, 8),
                None,
            ),
        ];
        for (name, key, expected) in cases {
            assert_eq!(cache.get(&key), expected, "{name}");
        }
    }

    #[test]
    fn cocoa_frames_flip_against_the_main_display_height() {
        let cases = [
            (
                "main display below its menu bar",
                rect(0.0, 0.0, 1512.0, 949.0),
                982.0,
                rect(0.0, 33.0, 1512.0, 949.0),
            ),
            (
                "laptop left of a taller main display",
                rect(-1512.0, 0.0, 1512.0, 949.0),
                1440.0,
                rect(-1512.0, 491.0, 1512.0, 949.0),
            ),
            (
                "display above the main display",
                rect(0.0, 1440.0, 1920.0, 1080.0),
                1440.0,
                rect(0.0, -1080.0, 1920.0, 1080.0),
            ),
        ];
        for (name, cocoa, main_display_height, global) in cases {
            assert_eq!(
                cocoa_to_global(cocoa, main_display_height),
                global,
                "{name}"
            );
        }
    }

    proptest! {
        #[test]
        fn prop_flipping_twice_restores_the_frame(
            x in -8000i32..8000,
            y in -8000i32..8000,
            w in 1i32..8000,
            h in 1i32..8000,
            main_display_height in 1i32..8000,
        ) {
            let frame = rect(f64::from(x), f64::from(y), f64::from(w), f64::from(h));
            let main_display_height = f64::from(main_display_height);
            let flipped = cocoa_to_global(frame, main_display_height);
            prop_assert_eq!(cocoa_to_global(flipped, main_display_height), frame);
        }
    }

    #[test]
    fn cached_snapshot_round_trips_layout_and_visible_frames() {
        let snapshot = ScreenSnapshot {
            key: key(
                &[
                    rect(1440.0, 0.0, 1920.0, 1080.0),
                    rect(0.0, 0.0, 1440.0, 900.0),
                ],
                42,
            ),
            visible: vec![
                rect(0.0, 25.0, 1440.0, 875.0),
                rect(1440.0, 0.0, 1920.0, 1080.0),
            ],
        };
        let parsed = parse_cached_snapshot(&cached_snapshot_contents(&snapshot)).unwrap();
        assert_eq!(
            parsed.key.displays,
            vec![
                rect(0.0, 0.0, 1440.0, 900.0),
                rect(1440.0, 0.0, 1920.0, 1080.0)
            ]
        );
        assert_eq!(parsed.key.preferences, 42);
        assert_eq!(parsed.visible, snapshot.visible);
    }

    #[test]
    fn cached_snapshot_rejects_invalid_contracts() {
        let cases = [
            "",
            "preferences,42\nlayout,0,0,100,100\n",
            "preferences,42\nvisible,0,0,100,100\n",
            "preferences,nope\nlayout,0,0,100,100\nvisible,0,0,100,100\n",
            "preferences,nope\npreferences,43\nlayout,0,0,100,100\nvisible,0,0,100,100\n",
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
            "preferences,42\nlayout,0,0,100,100\nvisible,0,0,200,200\n",
            "preferences,42\nlayout,0,0,100,100\nlayout,200,0,100,100\nvisible,0,0,50,50\nvisible,10,10,50,50\n",
        ];
        for contents in cases {
            assert!(parse_cached_snapshot(contents).is_none(), "{contents:?}");
        }
    }

    #[test]
    fn work_area_rows_survive_a_round_trip_and_reject_noise() {
        let rect = rect(-1512.0, 778.0, 1512.0, 950.0);
        let row = format!("{},{},{},{}", rect.x, rect.y, rect.w, rect.h);
        assert_eq!(parse_work_area_row(&row), Some(rect), "round trip");
        assert_eq!(
            parse_work_area_rows(&format!("[config] loaded from /a/b/c\n{row}\n\n")),
            vec![rect],
            "helper stdout"
        );

        let rejected = [
            "[config] loaded from /a/b/c",
            "",
            "0,0,100",
            "0,0,100,100,extra",
            "0,0,0,100",
            "0,0,NaN,100",
        ];
        for line in rejected {
            assert!(parse_work_area_row(line).is_none(), "{line:?}");
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
