use anyhow::{anyhow, Result};

use crate::{Monitor, Rect};

const MAX_DISPLAYS: u32 = 16;

type CGDirectDisplayID = u32;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetActiveDisplayList(
        max_displays: u32,
        active_displays: *mut CGDirectDisplayID,
        display_count: *mut u32,
    ) -> i32;
    fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    fn CGMainDisplayID() -> CGDirectDisplayID;
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DisplayInfo {
    pub(super) display_index: u32,
    pub(super) bounds: Monitor,
}

pub fn get_monitors() -> Result<Vec<Monitor>> {
    active_display_bounds()
}

pub fn full_screen_bounds() -> Result<Monitor> {
    let monitors = active_display_bounds()?;
    union_bounds(&monitors)
}

pub(super) fn active_display_bounds() -> Result<Vec<Monitor>> {
    Ok(active_displays()?
        .into_iter()
        .map(|display| display.bounds)
        .collect())
}

pub(super) fn active_displays() -> Result<Vec<DisplayInfo>> {
    let mut displays = [0u32; MAX_DISPLAYS as usize];
    let mut count = 0u32;
    let result = unsafe { CGGetActiveDisplayList(MAX_DISPLAYS, displays.as_mut_ptr(), &mut count) };

    if result != 0 {
        return Err(anyhow!("CGGetActiveDisplayList failed: {}", result));
    }

    if count == 0 {
        return Err(anyhow!("no active displays found"));
    }

    let display_ids = screencapture_display_order(&displays[..count as usize]);
    Ok(display_ids
        .into_iter()
        .enumerate()
        .map(|(index, display_id)| {
            let bounds = unsafe { CGDisplayBounds(display_id) };
            DisplayInfo {
                display_index: index as u32 + 1,
                bounds: monitor_from_cg_bounds(bounds),
            }
        })
        .collect())
}

fn screencapture_display_order(display_ids: &[CGDirectDisplayID]) -> Vec<CGDirectDisplayID> {
    let main = unsafe { CGMainDisplayID() };
    let mut ordered = Vec::with_capacity(display_ids.len());

    if display_ids.contains(&main) {
        ordered.push(main);
    }

    ordered.extend(
        display_ids
            .iter()
            .copied()
            .filter(|display| *display != main),
    );
    ordered
}

fn union_bounds(monitors: &[Monitor]) -> Result<Monitor> {
    crate::geometry::union_bounds(monitors).ok_or_else(|| anyhow!("no active displays found"))
}

fn monitor_from_cg_bounds(bounds: CGRect) -> Monitor {
    Monitor {
        x: round_i32(bounds.origin.x),
        y: round_i32(bounds.origin.y),
        w: round_i32(bounds.size.width),
        h: round_i32(bounds.size.height),
    }
}

fn round_i32(value: f64) -> i32 {
    value.round() as i32
}

pub(super) fn rect_intersection(left: Rect, right: Monitor) -> Option<Rect> {
    crate::geometry::rect_intersection(left, right)
}
