use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::ptr;

use anyhow::{ensure, Result};
use x11::{xcursor, xfixes, xlib};

use crate::cursor::platform::linux::scale::scale_bilinear;

const MAX_CURSOR_DIMENSION: u32 = 512;
const XFIXES_CURSOR_NOTIFY: i32 = 1;
const XFIXES_DISPLAY_CURSOR_NOTIFY: i32 = 0;
const XFIXES_DISPLAY_CURSOR_NOTIFY_MASK: libc::c_ulong = 1;
const FRAME_TABLE_CAP: usize = 4;
const MAX_SOURCE_FRAMES: usize = 60;

unsafe extern "C" {
    #[link_name = "XFixesSelectCursorInput"]
    fn xfixes_select_cursor_input_raw(
        display: *mut xlib::Display,
        window: xlib::Window,
        event_mask: libc::c_ulong,
    );
}

struct BaseCursor {
    width: u32,
    height: u32,
    xhot: u32,
    yhot: u32,
    pixels: Vec<u32>,
    default_size: u32,
    source: Option<CursorRaster>,
}

#[derive(Clone)]
struct CursorRaster {
    width: u32,
    height: u32,
    xhot: u32,
    yhot: u32,
    delay_ms: u32,
    pixels: Vec<u32>,
}

#[derive(Clone)]
struct CursorImage {
    width: u32,
    height: u32,
    xhot: u32,
    yhot: u32,
    pixels: Vec<u32>,
    default_size: u32,
    name: Option<String>,
    source: Vec<CursorRaster>,
}

struct ScaledCursor {
    frames: Vec<CursorRaster>,
    applied: CursorImage,
}

use animation::*;
use sampling::*;
use source::*;

mod animation;
mod sampling;
mod session;
mod source;

pub(crate) use session::{recover_scale, CursorSession};
