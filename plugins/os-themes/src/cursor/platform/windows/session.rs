use std::collections::HashMap;
use std::ptr::null_mut;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    CopyImage, DestroyCursor, GetCursorInfo, LoadCursorW, SetSystemCursor, SystemParametersInfoW,
    CURSORINFO, CURSOR_SHOWING, HCURSOR, IMAGE_CURSOR, LR_COPYFROMRESOURCE, OCR_APPSTARTING,
    OCR_CROSS, OCR_HAND, OCR_HELP, OCR_IBEAM, OCR_NO, OCR_NORMAL, OCR_SIZEALL, OCR_SIZENESW,
    OCR_SIZENS, OCR_SIZENWSE, OCR_SIZEWE, OCR_UP, OCR_WAIT, SPI_SETCURSORS, SYSTEM_CURSOR_ID,
};

use crate::cursor::platform::shake;

use super::raster::{self, Raster};

const SYSTEM_CURSORS: [SYSTEM_CURSOR_ID; 14] = [
    OCR_NORMAL,
    OCR_IBEAM,
    OCR_WAIT,
    OCR_CROSS,
    OCR_UP,
    OCR_SIZENWSE,
    OCR_SIZENESW,
    OCR_SIZEWE,
    OCR_SIZENS,
    OCR_SIZEALL,
    OCR_NO,
    OCR_HAND,
    OCR_APPSTARTING,
    OCR_HELP,
];

struct Source {
    base_width: u32,
    base_height: u32,
    raster: Raster,
}

pub(super) struct CursorSession {
    source_factor: f32,
    shared: Vec<(SYSTEM_CURSOR_ID, HCURSOR)>,
    sources: HashMap<SYSTEM_CURSOR_ID, Source>,
    applied: HashMap<SYSTEM_CURSOR_ID, f32>,
    scale: f32,
}

impl CursorSession {
    pub fn open(scale_factor: u32) -> Self {
        Self {
            source_factor: scale_factor.max(1) as f32,
            shared: shared_cursors(),
            sources: HashMap::new(),
            applied: HashMap::new(),
            scale: 1.0,
        }
    }

    fn live_id(&mut self) -> Option<SYSTEM_CURSOR_ID> {
        let info = cursor_info()?;
        if is_hidden(&info) {
            return None;
        }
        if let Some(id) = id_for(&self.shared, info.hCursor) {
            return Some(id);
        }
        self.shared = shared_cursors();
        id_for(&self.shared, info.hCursor)
    }

    fn apply(&mut self, id: SYSTEM_CURSOR_ID, scale: f32) -> bool {
        if !self.sources.contains_key(&id) {
            let Some(source) = self.capture(id) else {
                log::warn!("could not capture system cursor {id}");
                return false;
            };
            self.sources.insert(id, source);
        }
        let Some(source) = self.sources.get(&id) else {
            return false;
        };
        let Some((width, height)) =
            raster::target_size(source.base_width, source.base_height, scale)
        else {
            return false;
        };
        let Some(cursor) = source
            .raster
            .scaled_to(width, height)
            .as_ref()
            .and_then(raster::create_cursor)
        else {
            return false;
        };
        if unsafe { SetSystemCursor(cursor, id) } == 0 {
            log::warn!(
                "could not replace system cursor {id}: {}",
                std::io::Error::last_os_error()
            );
            unsafe { DestroyCursor(cursor) };
            return false;
        }
        self.applied.insert(id, scale);
        true
    }

    fn capture(&self, id: SYSTEM_CURSOR_ID) -> Option<Source> {
        let shared = self
            .shared
            .iter()
            .find(|(cursor_id, _)| *cursor_id == id)
            .map(|(_, handle)| *handle)?;
        let base = raster::capture(shared)?;
        let detailed = detailed_raster(shared, &base, self.source_factor);
        log::debug!(
            "captured cursor {id}: base={}x{} source={}x{}",
            base.width,
            base.height,
            detailed.width,
            detailed.height
        );
        Some(Source {
            base_width: base.width,
            base_height: base.height,
            raster: detailed,
        })
    }
}

impl shake::CursorSession for CursorSession {
    fn set_scale(&mut self, scale: f32) -> bool {
        if scale <= 1.0 + f32::EPSILON {
            self.restore();
            return true;
        }
        self.scale = scale;
        let id = self.live_id().unwrap_or(OCR_NORMAL);
        self.apply(id, scale)
    }

    fn refresh(&mut self) -> bool {
        if self.applied.is_empty() {
            return false;
        }
        let Some(id) = self.live_id() else {
            return false;
        };
        if self.applied.get(&id) == Some(&self.scale) {
            return false;
        }
        self.apply(id, self.scale)
    }

    fn restore(&mut self) {
        if self.applied.is_empty() {
            return;
        }
        reload_system_cursors();
        self.applied.clear();
        self.sources.clear();
        self.scale = 1.0;
        self.shared = shared_cursors();
    }

    fn live_cursor_hidden(&mut self) -> bool {
        cursor_info().is_some_and(|info| is_hidden(&info))
    }
}

impl Drop for CursorSession {
    fn drop(&mut self) {
        shake::CursorSession::restore(self);
    }
}

pub(super) fn reload_system_cursors() {
    if unsafe { SystemParametersInfoW(SPI_SETCURSORS, 0, null_mut(), 0) } == 0 {
        log::warn!(
            "could not reload the system cursor scheme: {}",
            std::io::Error::last_os_error()
        );
    }
}

fn detailed_raster(shared: HCURSOR, base: &Raster, factor: f32) -> Raster {
    let Some((width, height)) = raster::target_size(base.width, base.height, factor) else {
        return base.clone();
    };
    let copy = unsafe {
        CopyImage(
            shared,
            IMAGE_CURSOR,
            width as i32,
            height as i32,
            LR_COPYFROMRESOURCE,
        )
    };
    if copy.is_null() {
        return base.clone();
    }
    let detailed = raster::capture(copy);
    unsafe { DestroyCursor(copy) };
    detailed
        .filter(|detailed| detailed.longest_side() > base.longest_side())
        .unwrap_or_else(|| base.clone())
}

fn shared_cursors() -> Vec<(SYSTEM_CURSOR_ID, HCURSOR)> {
    SYSTEM_CURSORS
        .iter()
        .filter_map(|id| {
            let handle = unsafe { LoadCursorW(null_mut(), *id as usize as *const u16) };
            (!handle.is_null()).then_some((*id, handle))
        })
        .collect()
}

fn id_for(shared: &[(SYSTEM_CURSOR_ID, HCURSOR)], live: HCURSOR) -> Option<SYSTEM_CURSOR_ID> {
    shared
        .iter()
        .find(|(_, handle)| *handle == live)
        .map(|(id, _)| *id)
}

fn cursor_info() -> Option<CURSORINFO> {
    let mut info: CURSORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<CURSORINFO>() as u32;
    (unsafe { GetCursorInfo(&mut info) } != 0).then_some(info)
}

fn is_hidden(info: &CURSORINFO) -> bool {
    info.flags & CURSOR_SHOWING == 0 || info.hCursor.is_null()
}
