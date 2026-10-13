use std::io;

use windows_sys::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows_sys::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    COINIT_MULTITHREADED,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Apartment {
    SingleThreaded,
    MultiThreaded,
}

pub struct ComApartment {
    initialized: bool,
}

impl ComApartment {
    pub fn enter(apartment: Apartment) -> io::Result<ComApartment> {
        let mode = match apartment {
            Apartment::SingleThreaded => COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE,
            Apartment::MultiThreaded => COINIT_MULTITHREADED,
        };
        let result = unsafe { CoInitializeEx(std::ptr::null(), mode as u32) };
        if result == RPC_E_CHANGED_MODE {
            return Ok(ComApartment { initialized: false });
        }
        if result < 0 {
            return Err(io::Error::from_raw_os_error(result));
        }
        Ok(ComApartment { initialized: true })
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_and_mismatched_apartments_enter() {
        for (outer, inner) in [
            (Apartment::SingleThreaded, Apartment::SingleThreaded),
            (Apartment::MultiThreaded, Apartment::MultiThreaded),
            (Apartment::SingleThreaded, Apartment::MultiThreaded),
            (Apartment::MultiThreaded, Apartment::SingleThreaded),
        ] {
            std::thread::spawn(move || {
                let first = ComApartment::enter(outer).unwrap();
                let second = ComApartment::enter(inner).unwrap();
                assert!(first.initialized, "{outer:?}");
                assert_eq!(
                    second.initialized,
                    outer == inner,
                    "{outer:?} then {inner:?}"
                );
            })
            .join()
            .unwrap();
        }
    }
}
