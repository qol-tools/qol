use super::ffi::{
    CFArrayGetCount, CFArrayGetValueAtIndex, CFDictionaryGetValue, CFDictionaryRef,
    CFNumberGetValue, CFRelease,
};
use super::{parse_cg_window_list, CgWindow};
use std::collections::HashSet;
use std::ffi::c_void;
use std::sync::OnceLock;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFTypeArrayCallBacks: c_void;
    fn CFNumberCreate(
        allocator: *const c_void,
        number_type: isize,
        value_ptr: *const c_void,
    ) -> *const c_void;
    fn CFArrayCreate(
        allocator: *const c_void,
        values: *const *const c_void,
        num_values: isize,
        callbacks: *const c_void,
    ) -> *const c_void;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGWindowListCreateDescriptionFromArray(window_ids: *const c_void) -> *const c_void;
}

extern "C" {
    fn dlopen(path: *const std::ffi::c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const std::ffi::c_char) -> *mut c_void;
}
const RTLD_NOW: i32 = 2;

const K_CF_NUMBER_SINT64_TYPE: isize = 4;
const COPY_WINDOWS_INVISIBLE1: u64 = 1 << 0;
const COPY_WINDOWS_SCREEN_SAVER_LEVEL: u64 = 1 << 1;
const COPY_WINDOWS_INVISIBLE2: u64 = 1 << 2;
const COPY_WINDOWS_OPTIONS: u64 =
    COPY_WINDOWS_INVISIBLE1 | COPY_WINDOWS_SCREEN_SAVER_LEVEL | COPY_WINDOWS_INVISIBLE2;

type MainConnectionFn = unsafe extern "C" fn() -> u32;
type CopyManagedDisplaySpacesFn = unsafe extern "C" fn(u32) -> *const c_void;
type CopyWindowsFn =
    unsafe extern "C" fn(u32, u64, *const c_void, u64, *mut u64, *mut u64) -> *const c_void;

struct SpaceApi {
    main_connection: MainConnectionFn,
    copy_managed_display_spaces: CopyManagedDisplaySpacesFn,
    copy_windows: CopyWindowsFn,
}

fn space_api() -> Option<&'static SpaceApi> {
    static API: OnceLock<Option<SpaceApi>> = OnceLock::new();
    API.get_or_init(|| unsafe {
        let path = c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight";
        let handle = dlopen(path.as_ptr(), RTLD_NOW);
        if handle.is_null() {
            return None;
        }
        let main_connection = dlsym(handle, c"SLSMainConnectionID".as_ptr());
        let copy_spaces = dlsym(handle, c"SLSCopyManagedDisplaySpaces".as_ptr());
        let copy_windows = dlsym(handle, c"SLSCopyWindowsWithOptionsAndTags".as_ptr());
        if main_connection.is_null() || copy_spaces.is_null() || copy_windows.is_null() {
            return None;
        }
        Some(SpaceApi {
            main_connection: std::mem::transmute::<*mut c_void, MainConnectionFn>(main_connection),
            copy_managed_display_spaces: std::mem::transmute::<
                *mut c_void,
                CopyManagedDisplaySpacesFn,
            >(copy_spaces),
            copy_windows: std::mem::transmute::<*mut c_void, CopyWindowsFn>(copy_windows),
        })
    })
    .as_ref()
}

pub(super) struct CrossSpaceWindows {
    pub ids: HashSet<u32>,
    pub hydrated: Vec<CgWindow>,
}

impl CrossSpaceWindows {
    fn empty() -> Self {
        Self {
            ids: HashSet::new(),
            hydrated: Vec::new(),
        }
    }
}

pub(super) fn cross_space_windows(own_pid: i32, known_ids: &HashSet<u32>) -> CrossSpaceWindows {
    let Some(api) = space_api() else {
        return CrossSpaceWindows::empty();
    };
    let cid = unsafe { (api.main_connection)() };
    let layout = space_layout(api, cid);
    let inactive: Vec<i64> = layout
        .all
        .iter()
        .copied()
        .filter(|id| !layout.current.contains(id))
        .collect();
    if inactive.is_empty() {
        return CrossSpaceWindows::empty();
    }
    let on_inactive = windows_in_spaces(api, cid, &inactive);
    if on_inactive.is_empty() {
        return CrossSpaceWindows::empty();
    }
    let on_current: HashSet<u32> = windows_in_spaces(api, cid, &layout.current)
        .into_iter()
        .collect();
    let ids: HashSet<u32> = on_inactive
        .into_iter()
        .filter(|wid| !on_current.contains(wid))
        .collect();
    let unknown: Vec<u32> = ids
        .iter()
        .copied()
        .filter(|wid| !known_ids.contains(wid))
        .collect();
    let hydrated = if unknown.is_empty() {
        Vec::new()
    } else {
        describe_windows(&unknown, own_pid)
    };
    CrossSpaceWindows { ids, hydrated }
}

struct SpaceLayout {
    all: Vec<i64>,
    current: Vec<i64>,
}

fn space_layout(api: &SpaceApi, cid: u32) -> SpaceLayout {
    let mut layout = SpaceLayout {
        all: Vec::new(),
        current: Vec::new(),
    };
    let displays = unsafe { (api.copy_managed_display_spaces)(cid) };
    if displays.is_null() {
        return layout;
    }
    let key_spaces = super::ffi::cfstr(b"Spaces");
    let key_current = super::ffi::cfstr(b"Current Space");
    let key_id = super::ffi::cfstr(b"id64");

    let display_count = unsafe { CFArrayGetCount(displays) };
    for i in 0..display_count {
        let display = unsafe { CFArrayGetValueAtIndex(displays, i) } as CFDictionaryRef;
        if display.is_null() {
            continue;
        }
        let current = unsafe { CFDictionaryGetValue(display, key_current) } as CFDictionaryRef;
        if let Some(id) = space_id_of(current, key_id) {
            layout.current.push(id);
        }
        let spaces = unsafe { CFDictionaryGetValue(display, key_spaces) };
        if spaces.is_null() {
            continue;
        }
        let space_count = unsafe { CFArrayGetCount(spaces) };
        for j in 0..space_count {
            let space = unsafe { CFArrayGetValueAtIndex(spaces, j) } as CFDictionaryRef;
            if let Some(id) = space_id_of(space, key_id) {
                layout.all.push(id);
            }
        }
    }
    unsafe {
        CFRelease(key_spaces);
        CFRelease(key_current);
        CFRelease(key_id);
        CFRelease(displays);
    }
    layout
}

fn space_id_of(space: CFDictionaryRef, key_id: *const c_void) -> Option<i64> {
    if space.is_null() {
        return None;
    }
    let id_num = unsafe { CFDictionaryGetValue(space, key_id) };
    if id_num.is_null() {
        return None;
    }
    let mut id: i64 = 0;
    let ok = unsafe {
        CFNumberGetValue(
            id_num,
            K_CF_NUMBER_SINT64_TYPE,
            &mut id as *mut i64 as *mut c_void,
        )
    };
    ok.then_some(id)
}

fn windows_in_spaces(api: &SpaceApi, cid: u32, space_ids: &[i64]) -> Vec<u32> {
    let Some(spaces_array) = cf_number_array(space_ids) else {
        return Vec::new();
    };
    let mut set_tags: u64 = 0;
    let mut clear_tags: u64 = 0;
    let wid_list = unsafe {
        (api.copy_windows)(
            cid,
            0,
            spaces_array,
            COPY_WINDOWS_OPTIONS,
            &mut set_tags,
            &mut clear_tags,
        )
    };
    unsafe { CFRelease(spaces_array) };
    if wid_list.is_null() {
        return Vec::new();
    }
    let mut wids = Vec::new();
    let count = unsafe { CFArrayGetCount(wid_list) };
    for i in 0..count {
        let num = unsafe { CFArrayGetValueAtIndex(wid_list, i) };
        if num.is_null() {
            continue;
        }
        let mut wid: i64 = 0;
        let ok = unsafe {
            CFNumberGetValue(
                num,
                K_CF_NUMBER_SINT64_TYPE,
                &mut wid as *mut i64 as *mut c_void,
            )
        };
        if ok && wid > 0 {
            wids.push(wid as u32);
        }
    }
    unsafe { CFRelease(wid_list) };
    wids
}

fn describe_windows(wids: &[u32], own_pid: i32) -> Vec<CgWindow> {
    let raw: Vec<*const c_void> = wids
        .iter()
        .map(|wid| *wid as usize as *const c_void)
        .collect();
    let wid_array = unsafe {
        CFArrayCreate(
            std::ptr::null(),
            raw.as_ptr(),
            raw.len() as isize,
            std::ptr::null(),
        )
    };
    if wid_array.is_null() {
        return Vec::new();
    }
    let descriptions = unsafe { CGWindowListCreateDescriptionFromArray(wid_array) };
    unsafe { CFRelease(wid_array) };
    if descriptions.is_null() {
        return Vec::new();
    }
    let mut result = parse_cg_window_list(descriptions, own_pid);
    unsafe { CFRelease(descriptions) };
    for window in &mut result {
        window.is_cross_space = true;
    }
    result
}

fn cf_number_array(values: &[i64]) -> Option<*const c_void> {
    let numbers: Vec<*const c_void> = values
        .iter()
        .map(|value| unsafe {
            CFNumberCreate(
                std::ptr::null(),
                K_CF_NUMBER_SINT64_TYPE,
                value as *const i64 as *const c_void,
            )
        })
        .collect();
    if numbers.iter().any(|number| number.is_null()) {
        for number in numbers.iter().filter(|number| !number.is_null()) {
            unsafe { CFRelease(*number) };
        }
        return None;
    }
    let array = unsafe {
        CFArrayCreate(
            std::ptr::null(),
            numbers.as_ptr(),
            numbers.len() as isize,
            &kCFTypeArrayCallBacks as *const c_void,
        )
    };
    for number in &numbers {
        unsafe { CFRelease(*number) };
    }
    if array.is_null() {
        return None;
    }
    Some(array)
}
