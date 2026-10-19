use std::ffi::c_void;

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};

use crate::devices::Direction;
use crate::AudioError;

pub(super) type ObjectId = u32;

pub(super) const SYSTEM: ObjectId = 1;
pub(super) const NO_OBJECT: ObjectId = 0;

pub(super) const DEVICES: [u8; 4] = *b"dev#";
pub(super) const DEFAULT_OUTPUT: [u8; 4] = *b"dOut";
pub(super) const DEFAULT_INPUT: [u8; 4] = *b"dIn ";
pub(super) const DEVICE_UID: [u8; 4] = *b"uid ";
pub(super) const NAME: [u8; 4] = *b"lnam";
pub(super) const STREAMS: [u8; 4] = *b"stm#";
pub(super) const TRANSPORT: [u8; 4] = *b"tran";
pub(super) const CAN_BE_DEFAULT: [u8; 4] = *b"dflt";
pub(super) const IS_ALIVE: [u8; 4] = *b"livn";
pub(super) const IS_HIDDEN: [u8; 4] = *b"hidn";
pub(super) const IS_RUNNING_SOMEWHERE: [u8; 4] = *b"gone";
pub(super) const RELATED_DEVICES: [u8; 4] = *b"akin";
pub(super) const VIRTUAL_MAIN_VOLUME: [u8; 4] = *b"vmvc";
pub(super) const MUTE: [u8; 4] = *b"mute";
pub(super) const NOMINAL_SAMPLE_RATE: [u8; 4] = *b"nsrt";
pub(super) const VIRTUAL_FORMAT: [u8; 4] = *b"sfmt";
pub(super) const PROCESSES: [u8; 4] = *b"prs#";
pub(super) const PROCESS_DEVICES: [u8; 4] = *b"pdv#";
pub(super) const PROCESS_PID: [u8; 4] = *b"ppid";
pub(super) const PROCESS_IS_RUNNING_INPUT: [u8; 4] = *b"piri";

const GLOBAL_SCOPE: [u8; 4] = *b"glob";
const INPUT_SCOPE: [u8; 4] = *b"inpt";
const OUTPUT_SCOPE: [u8; 4] = *b"outp";
const MAIN_ELEMENT: u32 = 0;
const ID_SIZE: usize = std::mem::size_of::<ObjectId>();

#[repr(C)]
struct Address {
    selector: u32,
    scope: u32,
    element: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectHasProperty(object: ObjectId, address: *const Address) -> u8;
    fn AudioObjectGetPropertyDataSize(
        object: ObjectId,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
    ) -> i32;
    fn AudioObjectGetPropertyData(
        object: ObjectId,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> i32;
    fn AudioObjectSetPropertyData(
        object: ObjectId,
        address: *const Address,
        qualifier_size: u32,
        qualifier: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> i32;
}

#[derive(Clone, Copy)]
pub(super) struct Property {
    selector: [u8; 4],
    scope: [u8; 4],
}

impl Property {
    pub(super) const fn global(selector: [u8; 4]) -> Self {
        Self {
            selector,
            scope: GLOBAL_SCOPE,
        }
    }

    pub(super) const fn scoped(selector: [u8; 4], direction: Direction) -> Self {
        Self {
            selector,
            scope: match direction {
                Direction::Output => OUTPUT_SCOPE,
                Direction::Input => INPUT_SCOPE,
            },
        }
    }

    pub(super) const fn default_device(direction: Direction) -> Self {
        Self::global(match direction {
            Direction::Output => DEFAULT_OUTPUT,
            Direction::Input => DEFAULT_INPUT,
        })
    }

    fn address(self) -> Address {
        Address {
            selector: u32::from_be_bytes(self.selector),
            scope: u32::from_be_bytes(self.scope),
            element: MAIN_ELEMENT,
        }
    }

    fn refused(self, object: ObjectId, status: i32) -> AudioError {
        AudioError::Operation(format!(
            "Core Audio refused '{}' on object {object} with {}",
            String::from_utf8_lossy(&self.selector),
            status_text(status)
        ))
    }
}

pub(super) fn status_text(status: i32) -> String {
    let tag = status.to_be_bytes();
    if tag
        .iter()
        .all(|byte| byte.is_ascii_graphic() || *byte == b' ')
    {
        format!("'{}'", String::from_utf8_lossy(&tag))
    } else {
        format!("status {status}")
    }
}

pub(super) fn has(object: ObjectId, property: Property) -> bool {
    let address = property.address();
    unsafe { AudioObjectHasProperty(object, &address) != 0 }
}

pub(super) fn read_into<T: Copy>(
    object: ObjectId,
    property: Property,
    value: T,
) -> Result<T, AudioError> {
    let address = property.address();
    let mut value = value;
    let mut size = u32::try_from(std::mem::size_of::<T>()).unwrap_or(u32::MAX);
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            (&raw mut value).cast(),
        )
    };
    if status != 0 {
        return Err(property.refused(object, status));
    }
    Ok(value)
}

fn write_from<T: Copy>(object: ObjectId, property: Property, value: T) -> Result<(), AudioError> {
    let address = property.address();
    let size = u32::try_from(std::mem::size_of::<T>()).unwrap_or(u32::MAX);
    let status = unsafe {
        AudioObjectSetPropertyData(
            object,
            &address,
            0,
            std::ptr::null(),
            size,
            (&raw const value).cast(),
        )
    };
    if status != 0 {
        return Err(property.refused(object, status));
    }
    Ok(())
}

pub(super) fn read_u32(object: ObjectId, property: Property) -> Result<u32, AudioError> {
    read_into(object, property, 0u32)
}

pub(super) fn read_f32(object: ObjectId, property: Property) -> Result<f32, AudioError> {
    read_into(object, property, 0f32)
}

pub(super) fn write_u32(
    object: ObjectId,
    property: Property,
    value: u32,
) -> Result<(), AudioError> {
    write_from(object, property, value)
}

pub(super) fn write_f32(
    object: ObjectId,
    property: Property,
    value: f32,
) -> Result<(), AudioError> {
    write_from(object, property, value)
}

pub(super) fn read_ids(object: ObjectId, property: Property) -> Result<Vec<ObjectId>, AudioError> {
    let address = property.address();
    let mut size = 0u32;
    let status =
        unsafe { AudioObjectGetPropertyDataSize(object, &address, 0, std::ptr::null(), &mut size) };
    if status != 0 {
        return Err(property.refused(object, status));
    }
    let mut ids = vec![NO_OBJECT; size as usize / ID_SIZE];
    let mut size = u32::try_from(ids.len() * ID_SIZE).unwrap_or(u32::MAX);
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            &address,
            0,
            std::ptr::null(),
            &mut size,
            ids.as_mut_ptr().cast(),
        )
    };
    if status != 0 {
        return Err(property.refused(object, status));
    }
    ids.truncate(size as usize / ID_SIZE);
    Ok(ids)
}

pub(super) fn read_text(object: ObjectId, property: Property) -> Result<String, AudioError> {
    let text: CFStringRef = read_into(object, property, std::ptr::null())?;
    if text.is_null() {
        return Err(AudioError::Operation(format!(
            "Core Audio returned no text for '{}' on object {object}",
            String::from_utf8_lossy(&property.selector)
        )));
    }
    Ok(unsafe { CFString::wrap_under_create_rule(text) }.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_four_character_status_reads_as_its_code_and_any_other_as_a_number() {
        assert_eq!(status_text(i32::from_be_bytes(*b"who?")), "'who?'");
        assert_eq!(status_text(i32::from_be_bytes(*b"nope")), "'nope'");
        assert_eq!(status_text(-50), "status -50");
    }

    #[test]
    fn a_property_address_is_the_big_endian_code_of_each_tag() {
        let address = Property::scoped(VIRTUAL_MAIN_VOLUME, Direction::Output).address();
        assert_eq!(address.selector, 0x766d_7663);
        assert_eq!(address.scope, 0x6f75_7470);
        assert_eq!(address.element, MAIN_ELEMENT);
        let address = Property::default_device(Direction::Input).address();
        assert_eq!(address.selector, 0x6449_6e20);
        assert_eq!(address.scope, 0x676c_6f62);
    }
}
