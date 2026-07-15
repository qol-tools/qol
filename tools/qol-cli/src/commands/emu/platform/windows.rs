use std::ffi::c_void;

type ModuleHandle = isize;
type WhvGetCapability = unsafe extern "system" fn(i32, *mut c_void, u32, *mut u32) -> i32;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> ModuleHandle;
    fn GetProcAddress(module: ModuleHandle, name: *const u8) -> *mut c_void;
    fn FreeLibrary(module: ModuleHandle) -> i32;
}

pub(crate) fn hypervisor() -> &'static str {
    "whpx"
}

pub(crate) fn hypervisor_available() -> bool {
    whpx_available()
}

fn whpx_available() -> bool {
    let library = "WinHvPlatform.dll"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let module = unsafe { LoadLibraryW(library.as_ptr()) };
    if module == 0 {
        return false;
    }
    let procedure = unsafe { GetProcAddress(module, c"WHvGetCapability".as_ptr().cast()) };
    let available = if procedure.is_null() {
        false
    } else {
        let get_capability =
            unsafe { std::mem::transmute::<*mut c_void, WhvGetCapability>(procedure) };
        let mut present = 0i32;
        let mut written = 0u32;
        let size = u32::try_from(std::mem::size_of_val(&present)).unwrap_or_default();
        (unsafe { get_capability(0, (&mut present as *mut i32).cast(), size, &mut written) }) == 0
            && written == size
            && present != 0
    };
    unsafe { FreeLibrary(module) };
    available
}

pub(crate) fn display() -> &'static str {
    "sdl"
}

pub(crate) fn libvirt_uris() -> &'static [&'static str] {
    &[]
}
