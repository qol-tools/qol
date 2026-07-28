pub(crate) fn hypervisor() -> &'static str {
    "hvf"
}

pub(crate) fn hypervisor_available() -> bool {
    let mut supported = 0i32;
    let mut size = std::mem::size_of_val(&supported);
    let status = unsafe {
        libc::sysctlbyname(
            c"kern.hv_support".as_ptr(),
            (&mut supported as *mut i32).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    status == 0 && size == std::mem::size_of_val(&supported) && supported == 1
}

pub(crate) fn display() -> &'static str {
    "cocoa,zoom-to-fit=on"
}

pub(crate) fn libvirt_uris() -> &'static [&'static str] {
    &[]
}

pub(crate) fn path_is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
