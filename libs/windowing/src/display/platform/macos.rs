use sha2::{Digest, Sha256};

use super::DisplayEnumerator;
use crate::display::{DisplayError, DisplayHandle};

const MAX_DISPLAYS: u32 = 16;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetOnlineDisplayList(
        max_displays: u32,
        online_displays: *mut u32,
        display_count: *mut u32,
    ) -> i32;
    fn CGDisplayVendorNumber(display: u32) -> u32;
    fn CGDisplayModelNumber(display: u32) -> u32;
    fn CGDisplaySerialNumber(display: u32) -> u32;
    fn CGDisplayIsBuiltin(display: u32) -> bool;
}

pub struct Platform;

impl DisplayEnumerator for Platform {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, DisplayError> {
        let mut display_ids = [0u32; MAX_DISPLAYS as usize];
        let mut display_count = 0u32;
        let error = unsafe {
            CGGetOnlineDisplayList(MAX_DISPLAYS, display_ids.as_mut_ptr(), &mut display_count)
        };
        if error != 0 {
            return Err(DisplayError::Io(std::io::Error::other(format!(
                "CGGetOnlineDisplayList failed with {error}"
            ))));
        }
        let mut handles = Vec::with_capacity(display_count as usize);
        for &display_id in &display_ids[..display_count as usize] {
            let vendor = unsafe { CGDisplayVendorNumber(display_id) };
            let model = unsafe { CGDisplayModelNumber(display_id) };
            let serial = unsafe { CGDisplaySerialNumber(display_id) };
            let builtin = unsafe { CGDisplayIsBuiltin(display_id) };
            handles.push(derive_handle(vendor, model, serial, display_id, builtin));
        }
        Ok(handles)
    }
}

fn derive_handle(
    vendor: u32,
    model: u32,
    serial: u32,
    display_id: u32,
    builtin: bool,
) -> DisplayHandle {
    let digest: [u8; 32] = Sha256::digest(format!("{vendor}:{model}:{serial}").as_bytes()).into();
    let mut hash_bytes = [0u8; 8];
    hash_bytes.copy_from_slice(&digest[..8]);
    let hash = u64::from_be_bytes(hash_bytes);
    let connector = if builtin {
        format!("cg-{display_id}-builtin")
    } else {
        format!("cg-{display_id}")
    };
    DisplayHandle::new(format!("mac-{hash:016x}"), connector, None, serial == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_handle_pins_stable_id() {
        let handle = derive_handle(123, 456, 789, 1, false);
        assert_eq!(handle.id(), "mac-a879cdb88eb95acd");
    }

    #[test]
    fn derive_handle_serial_zero_marks_identity_unstable() {
        let handle = derive_handle(123, 456, 0, 1, false);
        assert!(handle.identity_unstable());
    }

    #[test]
    fn derive_handle_builtin_changes_connector_suffix() {
        let handle = derive_handle(123, 456, 789, 1, true);
        assert_eq!(handle.connector(), "cg-1-builtin");
    }

    #[test]
    fn derive_handle_same_tuple_is_equal() {
        let first = derive_handle(123, 456, 789, 1, false);
        let second = derive_handle(123, 456, 789, 1, false);
        assert_eq!(first, second);
    }

    #[test]
    fn derive_handle_serial_change_changes_id() {
        let first = derive_handle(123, 456, 789, 1, false);
        let second = derive_handle(123, 456, 790, 1, false);
        assert_ne!(first.id(), second.id());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn enumerate_returns_ok_on_this_host() {
        assert!(Platform.enumerate().is_ok());
    }
}
