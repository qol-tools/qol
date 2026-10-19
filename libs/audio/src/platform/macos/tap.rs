use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicU32, Ordering};

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::CFString;
use objc2::msg_send;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool};

use crate::AudioError;

use super::hal::{self, ObjectId, Property};

const TAP_UID: [u8; 4] = *b"tuid";
const AGGREGATE_NAME: &str = "qol sound level";
const AGGREGATE_UID_PREFIX: &str = "com.qol-tools.qol-audio.level";
const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

static NEXT_AGGREGATE: AtomicU32 = AtomicU32::new(0);

type CreateProcessTap =
    unsafe extern "C" fn(description: *mut AnyObject, tap: *mut ObjectId) -> i32;
type DestroyProcessTap = unsafe extern "C" fn(tap: ObjectId) -> i32;

extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioHardwareCreateAggregateDevice(
        description: CFDictionaryRef,
        device: *mut ObjectId,
    ) -> i32;
    fn AudioHardwareDestroyAggregateDevice(device: ObjectId) -> i32;
}

pub(super) struct SystemTap {
    tap: ObjectId,
    aggregate: ObjectId,
    destroy_tap: DestroyProcessTap,
}

unsafe impl Send for SystemTap {}

impl SystemTap {
    pub(super) fn create() -> Result<Self, AudioError> {
        let (create_tap, destroy_tap) = tapping().ok_or(AudioError::Unsupported)?;
        let description = global_description()?;
        let mut tap = hal::NO_OBJECT;
        let status = unsafe { create_tap(Retained::as_ptr(&description).cast_mut(), &mut tap) };
        if status != 0 {
            return Err(AudioError::Operation(format!(
                "cannot tap the system output: Core Audio answered {}",
                hal::status_text(status)
            )));
        }
        let mut system_tap = Self {
            tap,
            aggregate: hal::NO_OBJECT,
            destroy_tap,
        };
        let tap_uid = hal::read_text(tap, Property::global(TAP_UID))?;
        system_tap.aggregate = aggregate_over(&tap_uid)?;
        Ok(system_tap)
    }

    pub(super) fn device(&self) -> ObjectId {
        self.aggregate
    }
}

impl Drop for SystemTap {
    fn drop(&mut self) {
        unsafe {
            if self.aggregate != hal::NO_OBJECT {
                AudioHardwareDestroyAggregateDevice(self.aggregate);
            }
            (self.destroy_tap)(self.tap);
        }
    }
}

fn tapping() -> Option<(CreateProcessTap, DestroyProcessTap)> {
    let create = symbol(c"AudioHardwareCreateProcessTap")?;
    let destroy = symbol(c"AudioHardwareDestroyProcessTap")?;
    Some(unsafe {
        (
            std::mem::transmute::<*mut c_void, CreateProcessTap>(create),
            std::mem::transmute::<*mut c_void, DestroyProcessTap>(destroy),
        )
    })
}

fn symbol(name: &CStr) -> Option<*mut c_void> {
    let found = unsafe { dlsym(RTLD_DEFAULT, name.as_ptr()) };
    (!found.is_null()).then_some(found)
}

fn global_description() -> Result<Retained<AnyObject>, AudioError> {
    let description_class = AnyClass::get(c"CATapDescription").ok_or(AudioError::Unsupported)?;
    let array_class = AnyClass::get(c"NSArray").ok_or(AudioError::Unsupported)?;
    let excluded: Retained<AnyObject> = unsafe { msg_send![array_class, array] };
    let allocated: Allocated<AnyObject> = unsafe { msg_send![description_class, alloc] };
    let description: Option<Retained<AnyObject>> =
        unsafe { msg_send![allocated, initStereoGlobalTapButExcludeProcesses: &*excluded] };
    let description = description.ok_or_else(|| {
        AudioError::Operation("Core Audio refused the system output tap description".to_owned())
    })?;
    let _: () = unsafe { msg_send![&*description, setPrivate: Bool::YES] };
    Ok(description)
}

fn aggregate_over(tap_uid: &str) -> Result<ObjectId, AudioError> {
    let description = aggregate_description(&aggregate_uid(), tap_uid);
    let mut device = hal::NO_OBJECT;
    let status = unsafe {
        AudioHardwareCreateAggregateDevice(description.as_concrete_TypeRef(), &mut device)
    };
    if status != 0 {
        return Err(AudioError::Operation(format!(
            "cannot open the system output tap: Core Audio answered {}",
            hal::status_text(status)
        )));
    }
    Ok(device)
}

fn aggregate_uid() -> String {
    format!(
        "{AGGREGATE_UID_PREFIX}.{}.{}",
        std::process::id(),
        NEXT_AGGREGATE.fetch_add(1, Ordering::Relaxed)
    )
}

fn entry(key: &str, value: CFType) -> (CFType, CFType) {
    (CFString::new(key).as_CFType(), value)
}

fn aggregate_description(uid: &str, tap_uid: &str) -> CFDictionary<CFType, CFType> {
    let tap = CFDictionary::from_CFType_pairs(&[
        entry("uid", CFString::new(tap_uid).as_CFType()),
        entry("drift", CFBoolean::true_value().as_CFType()),
    ]);
    CFDictionary::from_CFType_pairs(&[
        entry("uid", CFString::new(uid).as_CFType()),
        entry("name", CFString::new(AGGREGATE_NAME).as_CFType()),
        entry("private", CFBoolean::true_value().as_CFType()),
        entry("stacked", CFBoolean::false_value().as_CFType()),
        entry("tapautostart", CFBoolean::true_value().as_CFType()),
        entry(
            "taps",
            CFArray::from_CFTypes(&[tap.as_CFType()]).as_CFType(),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_aggregate_gets_its_own_uid() {
        let first = aggregate_uid();
        let second = aggregate_uid();
        assert_ne!(first, second);
        assert!(first.starts_with(AGGREGATE_UID_PREFIX), "{first}");
    }

    #[test]
    fn the_aggregate_is_private_and_carries_only_the_tap() {
        let description = aggregate_description("aggregate-uid", "tap-uid");
        let text = |key: &str| {
            description
                .find(CFString::new(key).as_CFType())
                .and_then(|value| value.downcast::<CFString>())
                .map(|value| value.to_string())
        };
        assert_eq!(text("uid").as_deref(), Some("aggregate-uid"));
        assert_eq!(text("name").as_deref(), Some(AGGREGATE_NAME));
        let private = description
            .find(CFString::new("private").as_CFType())
            .and_then(|value| value.downcast::<CFBoolean>());
        assert_eq!(private, Some(CFBoolean::true_value()));
        assert!(description
            .find(CFString::new("subdevices").as_CFType())
            .is_none());
        let taps = description
            .find(CFString::new("taps").as_CFType())
            .and_then(|value| value.downcast::<CFArray>())
            .expect("a tap list");
        assert_eq!(taps.len(), 1);
    }
}
