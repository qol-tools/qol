use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFRelease, CFRetain, CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::runloop::{
    kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopRef, CFRunLoopTimer, CFRunLoopTimerContext,
    CFRunLoopTimerRef,
};
use core_foundation::string::{CFString, CFStringRef};

use super::modifier_mapping::ModifierMapping;
use super::protocol::{
    PAGE_APPLE_VENDOR_KEYBOARD, PAGE_APPLE_VENDOR_TOP_CASE, PAGE_CONSUMER, PAGE_KEYBOARD,
};
use super::report::PressedKeys;
use super::watchdog::Action;
use super::Shared;
use crate::platform::macos::virtual_hid::client::request::{
    VIRTUAL_KEYBOARD_PRODUCT_ID, VIRTUAL_KEYBOARD_VENDOR_ID,
};

type IOHIDManagerRef = *mut c_void;
type IOHIDDeviceRef = *mut c_void;
type IOHIDValueRef = *mut c_void;
type IOHIDElementRef = *mut c_void;
type IOReturn = i32;
type DeviceCallback = extern "C" fn(*mut c_void, IOReturn, *mut c_void, IOHIDDeviceRef);
type ValueCallback = extern "C" fn(*mut c_void, IOReturn, *mut c_void, IOHIDValueRef);

const OPTIONS_NONE: u32 = 0;
const OPTIONS_SEIZE: u32 = 1;
const RETURN_EXCLUSIVE_ACCESS: IOReturn = 0xE000_02C5_u32 as i32;
const REQUEST_LISTEN_EVENT: u32 = 1;
const ACCESS_GRANTED: u32 = 0;
const APPLE_VENDOR_ID: i64 = 0x05AC;
const REGISTRY_ITERATE_RECURSIVELY: u32 = 1;
const TICK_SECONDS: f64 = 0.025;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOHIDManagerCreate(allocator: *const c_void, options: u32) -> IOHIDManagerRef;
    fn IOHIDManagerSetDeviceMatching(manager: IOHIDManagerRef, matching: CFDictionaryRef);
    fn IOHIDManagerRegisterDeviceMatchingCallback(
        manager: IOHIDManagerRef,
        callback: DeviceCallback,
        context: *mut c_void,
    );
    fn IOHIDManagerRegisterDeviceRemovalCallback(
        manager: IOHIDManagerRef,
        callback: DeviceCallback,
        context: *mut c_void,
    );
    fn IOHIDManagerScheduleWithRunLoop(
        manager: IOHIDManagerRef,
        run_loop: CFRunLoopRef,
        mode: CFStringRef,
    );
    fn IOHIDDeviceOpen(device: IOHIDDeviceRef, options: u32) -> IOReturn;
    fn IOHIDDeviceClose(device: IOHIDDeviceRef, options: u32) -> IOReturn;
    fn IOHIDDeviceGetProperty(device: IOHIDDeviceRef, key: CFStringRef) -> CFTypeRef;
    fn IOHIDDeviceRegisterInputValueCallback(
        device: IOHIDDeviceRef,
        callback: Option<ValueCallback>,
        context: *mut c_void,
    );
    fn IOHIDDeviceScheduleWithRunLoop(
        device: IOHIDDeviceRef,
        run_loop: CFRunLoopRef,
        mode: CFStringRef,
    );
    fn IOHIDDeviceUnscheduleFromRunLoop(
        device: IOHIDDeviceRef,
        run_loop: CFRunLoopRef,
        mode: CFStringRef,
    );
    fn IOHIDDeviceCopyMatchingElements(
        device: IOHIDDeviceRef,
        matching: CFDictionaryRef,
        options: u32,
    ) -> CFArrayRef;
    fn IOHIDDeviceSetValue(
        device: IOHIDDeviceRef,
        element: IOHIDElementRef,
        value: IOHIDValueRef,
    ) -> IOReturn;
    fn IOHIDValueCreateWithIntegerValue(
        allocator: *const c_void,
        element: IOHIDElementRef,
        timestamp: u64,
        value: isize,
    ) -> IOHIDValueRef;
    fn IOHIDValueGetElement(value: IOHIDValueRef) -> IOHIDElementRef;
    fn IOHIDValueGetIntegerValue(value: IOHIDValueRef) -> isize;
    fn IOHIDElementGetUsagePage(element: IOHIDElementRef) -> u32;
    fn IOHIDElementGetUsage(element: IOHIDElementRef) -> u32;
    fn IOHIDDeviceGetService(device: IOHIDDeviceRef) -> u32;
    fn IORegistryEntrySearchCFProperty(
        entry: u32,
        plane: *const std::ffi::c_char,
        key: CFStringRef,
        allocator: *const c_void,
        options: u32,
    ) -> CFTypeRef;
    fn IOHIDCheckAccess(request: u32) -> u32;
    fn IOHIDRequestAccess(request: u32) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFAbsoluteTimeGetCurrent() -> f64;
}

struct Keyboard {
    device: IOHIDDeviceRef,
    name: String,
    apple: bool,
    modifiers: ModifierMapping,
    seized: bool,
    pressed: PressedKeys,
}

struct Devices {
    shared: Arc<Shared>,
    keyboards: RefCell<Vec<Keyboard>>,
    run_loop: CFRunLoop,
}

pub(super) fn input_monitoring_granted() -> bool {
    unsafe { IOHIDCheckAccess(REQUEST_LISTEN_EVENT) == ACCESS_GRANTED }
}

pub(super) fn request_input_monitoring() {
    if !input_monitoring_granted() && !unsafe { IOHIDRequestAccess(REQUEST_LISTEN_EVENT) } {
        log::warn!(
            "Input Monitoring is off for the keyboard helper; turn on com.qol-tools.keyremap.hid-helper in System Settings > Privacy & Security > Input Monitoring"
        );
    }
}

pub(super) fn run(shared: Arc<Shared>) -> ! {
    let devices: &'static Devices = Box::leak(Box::new(Devices {
        shared,
        keyboards: RefCell::new(Vec::new()),
        run_loop: CFRunLoop::get_current(),
    }));
    let context = ptr::from_ref(devices).cast_mut().cast::<c_void>();
    let matching = number_dictionary(&[("DeviceUsagePage", 0x01), ("DeviceUsage", 0x06)]);
    unsafe {
        let manager = IOHIDManagerCreate(ptr::null(), OPTIONS_NONE);
        IOHIDManagerSetDeviceMatching(manager, matching.as_concrete_TypeRef());
        IOHIDManagerRegisterDeviceMatchingCallback(manager, device_matched, context);
        IOHIDManagerRegisterDeviceRemovalCallback(manager, device_removed, context);
        IOHIDManagerScheduleWithRunLoop(
            manager,
            devices.run_loop.as_concrete_TypeRef(),
            kCFRunLoopDefaultMode,
        );
    }
    let mut timer_context = CFRunLoopTimerContext {
        version: 0,
        info: context,
        retain: None,
        release: None,
        copyDescription: None,
    };
    let timer = CFRunLoopTimer::new(
        unsafe { CFAbsoluteTimeGetCurrent() } + TICK_SECONDS,
        TICK_SECONDS,
        0,
        0,
        tick,
        &mut timer_context,
    );
    devices
        .run_loop
        .add_timer(&timer, unsafe { kCFRunLoopDefaultMode });
    loop {
        CFRunLoop::run_current();
    }
}

extern "C" fn tick(_timer: CFRunLoopTimerRef, info: *mut c_void) {
    let devices = unsafe { &*info.cast::<Devices>() };
    let (action, caps_light) = devices.shared.tick();
    match action {
        Action::Seize => {
            let any = devices.seize_all();
            devices.shared.announce_seized(any);
        }
        Action::Release => {
            devices.release_all();
            devices.shared.announce_seized(false);
        }
        Action::Hold => {}
    }
    if let Some(on) = caps_light {
        devices.set_caps_lock_light(on);
    }
}

extern "C" fn device_matched(
    context: *mut c_void,
    _result: IOReturn,
    _sender: *mut c_void,
    device: IOHIDDeviceRef,
) {
    let devices = unsafe { &*context.cast::<Devices>() };
    let vendor = number_property(device, "VendorID").unwrap_or(0);
    let product = number_property(device, "ProductID").unwrap_or(0);
    let name = string_property(device, "Product")
        .unwrap_or_else(|| format!("keyboard {vendor:04x}:{product:04x}"));
    if is_virtual_keyboard(vendor, product, &name) {
        return;
    }
    unsafe { CFRetain(device as CFTypeRef) };
    log::info!("keyboard arrived: {name}");
    devices.keyboards.borrow_mut().push(Keyboard {
        device,
        name,
        apple: is_apple_keyboard(vendor, bool_property(device, "Built-In").unwrap_or(false)),
        modifiers: modifier_mapping(device),
        seized: false,
        pressed: PressedKeys::default(),
    });
    let country_code = number_property(device, "CountryCode").unwrap_or(0);
    devices
        .shared
        .device_arrived(u64::try_from(country_code).unwrap_or(0));
}

extern "C" fn device_removed(
    context: *mut c_void,
    _result: IOReturn,
    _sender: *mut c_void,
    device: IOHIDDeviceRef,
) {
    let devices = unsafe { &*context.cast::<Devices>() };
    let mut removed = {
        let mut keyboards = devices.keyboards.borrow_mut();
        let Some(index) = keyboards
            .iter()
            .position(|keyboard| keyboard.device == device)
        else {
            return;
        };
        keyboards.remove(index)
    };
    for (page, usage) in removed.pressed.drain() {
        devices.shared.forward(page, usage, false, removed.apple);
    }
    if removed.seized {
        devices.close(&removed);
    }
    log::info!("keyboard removed: {}", removed.name);
    unsafe { CFRelease(removed.device as CFTypeRef) };
    devices.publish(Vec::new());
}

extern "C" fn value_changed(
    context: *mut c_void,
    _result: IOReturn,
    sender: *mut c_void,
    value: IOHIDValueRef,
) {
    let devices = unsafe { &*context.cast::<Devices>() };
    let element = unsafe { IOHIDValueGetElement(value) };
    let (Ok(page), Ok(usage)) = (
        u16::try_from(unsafe { IOHIDElementGetUsagePage(element) }),
        u16::try_from(unsafe { IOHIDElementGetUsage(element) }),
    ) else {
        return;
    };
    let pressed = unsafe { IOHIDValueGetIntegerValue(value) } != 0;
    let (page, usage, apple) = {
        let mut keyboards = devices.keyboards.borrow_mut();
        let Some(keyboard) = keyboards
            .iter_mut()
            .find(|keyboard| keyboard.device == sender)
        else {
            return;
        };
        let (page, usage) = keyboard.modifiers.apply(page, usage);
        if !forwarded(page, usage) {
            return;
        }
        keyboard.pressed.record(page, usage, pressed);
        (page, usage, keyboard.apple)
    };
    devices.shared.forward(page, usage, pressed, apple);
}

impl Devices {
    fn seize_all(&self) -> bool {
        let mut conflicts = Vec::new();
        let context = ptr::from_ref(self).cast_mut().cast::<c_void>();
        for keyboard in self
            .keyboards
            .borrow_mut()
            .iter_mut()
            .filter(|keyboard| !keyboard.seized)
        {
            match unsafe { IOHIDDeviceOpen(keyboard.device, OPTIONS_SEIZE) } {
                0 => unsafe {
                    IOHIDDeviceRegisterInputValueCallback(
                        keyboard.device,
                        Some(value_changed),
                        context,
                    );
                    IOHIDDeviceScheduleWithRunLoop(
                        keyboard.device,
                        self.run_loop.as_concrete_TypeRef(),
                        kCFRunLoopDefaultMode,
                    );
                    keyboard.seized = true;
                    log::info!("seized {}", keyboard.name);
                },
                RETURN_EXCLUSIVE_ACCESS => {
                    log::warn!("{} is held by another app; leaving it alone", keyboard.name);
                    conflicts.push(keyboard.name.clone());
                }
                code => log::warn!("could not seize {}: IOReturn {code:#x}", keyboard.name),
            }
        }
        self.publish(conflicts)
    }

    fn release_all(&self) {
        for keyboard in self
            .keyboards
            .borrow_mut()
            .iter_mut()
            .filter(|keyboard| keyboard.seized)
        {
            self.close(keyboard);
            keyboard.seized = false;
            keyboard.pressed.drain();
            log::info!("released {}", keyboard.name);
        }
        self.publish(Vec::new());
    }

    fn close(&self, keyboard: &Keyboard) {
        unsafe {
            IOHIDDeviceRegisterInputValueCallback(keyboard.device, None, ptr::null_mut());
            IOHIDDeviceUnscheduleFromRunLoop(
                keyboard.device,
                self.run_loop.as_concrete_TypeRef(),
                kCFRunLoopDefaultMode,
            );
            IOHIDDeviceClose(keyboard.device, OPTIONS_NONE);
        }
    }

    fn publish(&self, conflicts: Vec<String>) -> bool {
        let seized: Vec<String> = self
            .keyboards
            .borrow()
            .iter()
            .filter(|keyboard| keyboard.seized)
            .map(|keyboard| keyboard.name.clone())
            .collect();
        let any = !seized.is_empty();
        self.shared.publish_devices(seized, conflicts);
        any
    }

    fn set_caps_lock_light(&self, on: bool) {
        let matching = number_dictionary(&[("UsagePage", 0x08), ("Usage", 0x02)]);
        for keyboard in self
            .keyboards
            .borrow()
            .iter()
            .filter(|keyboard| keyboard.seized)
        {
            let elements = unsafe {
                IOHIDDeviceCopyMatchingElements(
                    keyboard.device,
                    matching.as_concrete_TypeRef(),
                    OPTIONS_NONE,
                )
            };
            if elements.is_null() {
                continue;
            }
            let elements: CFArray<CFType> = unsafe { CFArray::wrap_under_create_rule(elements) };
            for element in elements.iter() {
                let element = element.as_CFTypeRef() as IOHIDElementRef;
                let value = unsafe {
                    IOHIDValueCreateWithIntegerValue(ptr::null(), element, 0, isize::from(on))
                };
                if value.is_null() {
                    continue;
                }
                unsafe {
                    IOHIDDeviceSetValue(keyboard.device, element, value);
                    CFRelease(value as CFTypeRef);
                }
            }
        }
    }
}

fn forwarded(page: u16, usage: u16) -> bool {
    match page {
        PAGE_KEYBOARD => (0x04..=0xE7).contains(&usage),
        PAGE_CONSUMER | PAGE_APPLE_VENDOR_KEYBOARD | PAGE_APPLE_VENDOR_TOP_CASE => usage != 0,
        _ => false,
    }
}

fn is_apple_keyboard(vendor: i64, built_in: bool) -> bool {
    built_in || vendor == APPLE_VENDOR_ID
}

fn is_virtual_keyboard(vendor: i64, product: i64, name: &str) -> bool {
    let pqrs = u64::try_from(vendor) == Ok(VIRTUAL_KEYBOARD_VENDOR_ID)
        && u64::try_from(product) == Ok(VIRTUAL_KEYBOARD_PRODUCT_ID);
    pqrs || name.contains("Karabiner")
}

fn number_dictionary(pairs: &[(&str, i64)]) -> CFDictionary<CFType, CFType> {
    let pairs: Vec<(CFType, CFType)> = pairs
        .iter()
        .map(|(key, value)| {
            (
                CFString::new(key).as_CFType(),
                CFNumber::from(*value).as_CFType(),
            )
        })
        .collect();
    CFDictionary::from_CFType_pairs(&pairs)
}

fn modifier_mapping(device: IOHIDDeviceRef) -> ModifierMapping {
    let key = CFString::from_static_string("HIDEventServiceProperties");
    let found = unsafe {
        IORegistryEntrySearchCFProperty(
            IOHIDDeviceGetService(device),
            c"IOService".as_ptr(),
            key.as_concrete_TypeRef(),
            ptr::null(),
            REGISTRY_ITERATE_RECURSIVELY,
        )
    };
    if found.is_null() {
        return ModifierMapping::default();
    }
    let properties = unsafe { CFType::wrap_under_create_rule(found) };
    let pairs = properties
        .downcast::<CFDictionary>()
        .and_then(|properties| {
            let key = CFString::from_static_string("HIDKeyboardModifierMappingPairs");
            let pairs = properties.find(key.as_concrete_TypeRef().cast::<c_void>())?;
            unsafe { CFType::wrap_under_get_rule(pairs.cast()) }.downcast::<CFArray>()
        });
    let Some(pairs) = pairs else {
        return ModifierMapping::default();
    };
    let number = |pair: &CFDictionary, key: &'static str| {
        let key = CFString::from_static_string(key);
        let value = pair.find(key.as_concrete_TypeRef().cast::<c_void>())?;
        unsafe { CFType::wrap_under_get_rule(value.cast()) }
            .downcast::<CFNumber>()?
            .to_i64()
    };
    ModifierMapping::from_pairs(pairs.iter().filter_map(|pair| {
        let pair =
            unsafe { CFType::wrap_under_get_rule(pair.cast()) }.downcast::<CFDictionary>()?;
        Some((
            number(&pair, "HIDKeyboardModifierMappingSrc")?,
            number(&pair, "HIDKeyboardModifierMappingDst")?,
        ))
    }))
}

fn property(device: IOHIDDeviceRef, key: &str) -> Option<CFType> {
    let key = CFString::new(key);
    let value = unsafe { IOHIDDeviceGetProperty(device, key.as_concrete_TypeRef()) };
    (!value.is_null()).then(|| unsafe { CFType::wrap_under_get_rule(value) })
}

fn number_property(device: IOHIDDeviceRef, key: &str) -> Option<i64> {
    property(device, key)?.downcast::<CFNumber>()?.to_i64()
}

fn bool_property(device: IOHIDDeviceRef, key: &str) -> Option<bool> {
    property(device, key)?
        .downcast::<CFBoolean>()
        .map(bool::from)
}

fn string_property(device: IOHIDDeviceRef, key: &str) -> Option<String> {
    property(device, key)?
        .downcast::<CFString>()
        .map(|name| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_built_in_keyboard_is_an_apple_keyboard_whatever_its_vendor_id() {
        assert!(is_apple_keyboard(0, true));
        assert!(is_apple_keyboard(APPLE_VENDOR_ID, false));
        assert!(!is_apple_keyboard(0x046D, false));
    }

    #[test]
    fn only_key_pages_are_forwarded() {
        assert!(forwarded(PAGE_KEYBOARD, 0x04));
        assert!(forwarded(PAGE_KEYBOARD, 0xE7));
        assert!(!forwarded(PAGE_KEYBOARD, 0x01));
        assert!(!forwarded(PAGE_KEYBOARD, 0xFFFF));
        assert!(forwarded(PAGE_CONSUMER, 0xCD));
        assert!(forwarded(PAGE_APPLE_VENDOR_TOP_CASE, 0x03));
        assert!(forwarded(PAGE_APPLE_VENDOR_KEYBOARD, 0x01));
        assert!(!forwarded(PAGE_CONSUMER, 0));
        assert!(!forwarded(0x01, 0x30));
    }

    #[test]
    fn the_virtual_keyboard_itself_is_never_seized() {
        assert!(is_virtual_keyboard(0x16c0, 0x27db, "anything"));
        assert!(is_virtual_keyboard(
            0x05ac,
            0x0341,
            "Karabiner DriverKit VirtualHIDKeyboard"
        ));
        assert!(!is_virtual_keyboard(
            0x05ac,
            0x0341,
            "Apple Internal Keyboard / Trackpad"
        ));
    }
}
