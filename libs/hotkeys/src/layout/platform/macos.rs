use std::ffi::c_void;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, TCFType};
use core_foundation::data::{CFData, CFDataRef};
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};

use crate::keycode::macos_keycode::PhysicalLayout;
use crate::layout::LayoutError;

const KEY_ACTION_DOWN: u16 = 0;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    static kTISPropertyUnicodeKeyLayoutData: CFStringRef;
    static kTISPropertyInputSourceID: CFStringRef;
    fn TISCopyCurrentKeyboardLayoutInputSource() -> *const c_void;
    fn TISCreateInputSourceList(properties: *const c_void, include_all: u8) -> CFArrayRef;
    fn TISGetInputSourceProperty(source: *const c_void, key: CFStringRef) -> *const c_void;
    fn LMGetKbdType() -> u8;
    fn KBGetLayoutType(keyboard_type: i16) -> u32;
    #[allow(clippy::too_many_arguments)]
    fn UCKeyTranslate(
        layout: *const c_void,
        code: u16,
        action: u16,
        modifier_state: u32,
        keyboard_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        max_length: usize,
        actual_length: *mut usize,
        chars: *mut u16,
    ) -> i32;
}

pub struct KeyLayout {
    id: String,
    data: CFData,
    keyboard_type: u32,
}

impl KeyLayout {
    pub fn current() -> Result<Self, LayoutError> {
        let source = unsafe { TISCopyCurrentKeyboardLayoutInputSource() };
        if source.is_null() {
            return Err(LayoutError::NoInputSource);
        }
        Self::from_source(unsafe { CFType::wrap_under_create_rule(source) })
    }

    pub fn by_id(id: &str) -> Result<Self, LayoutError> {
        let key = unsafe { CFString::wrap_under_get_rule(kTISPropertyInputSourceID) };
        let filter =
            CFDictionary::from_CFType_pairs(&[(key.as_CFType(), CFString::new(id).as_CFType())]);
        let list = unsafe { TISCreateInputSourceList(filter.as_CFTypeRef(), 1) };
        if list.is_null() {
            return Err(LayoutError::NotFound(id.to_string()));
        }
        let list: CFArray<CFType> = unsafe { CFArray::wrap_under_create_rule(list) };
        let source = list
            .get(0)
            .map(|source| source.clone())
            .ok_or_else(|| LayoutError::NotFound(id.to_string()))?;
        Self::from_source(source)
    }

    fn from_source(source: CFType) -> Result<Self, LayoutError> {
        let id = source_id(&source);
        let data = unsafe {
            TISGetInputSourceProperty(source.as_CFTypeRef(), kTISPropertyUnicodeKeyLayoutData)
        };
        if data.is_null() {
            return Err(LayoutError::NoKeyMap(id));
        }
        Ok(Self {
            id,
            data: unsafe { CFData::wrap_under_get_rule(data as CFDataRef) },
            keyboard_type: u32::from(unsafe { LMGetKbdType() }),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn translate(
        &self,
        code: u16,
        modifier_state: u32,
        dead_key_state: &mut u32,
    ) -> Option<String> {
        let mut length = 0;
        let mut chars = [0u16; 4];
        let status = unsafe {
            UCKeyTranslate(
                self.data.bytes().as_ptr().cast(),
                code,
                KEY_ACTION_DOWN,
                modifier_state,
                self.keyboard_type,
                0,
                dead_key_state,
                chars.len(),
                &mut length,
                chars.as_mut_ptr(),
            )
        };
        (status == 0).then(|| String::from_utf16_lossy(&chars[..length]))
    }
}

pub fn current_layout_id() -> Result<String, LayoutError> {
    let source = unsafe { TISCopyCurrentKeyboardLayoutInputSource() };
    if source.is_null() {
        return Err(LayoutError::NoInputSource);
    }
    Ok(source_id(&unsafe {
        CFType::wrap_under_create_rule(source)
    }))
}

pub fn physical_layout() -> PhysicalLayout {
    let keyboard_type = unsafe { LMGetKbdType() };
    PhysicalLayout::from_layout_type(unsafe { KBGetLayoutType(i16::from(keyboard_type)) })
}

fn source_id(source: &CFType) -> String {
    let id = unsafe { TISGetInputSourceProperty(source.as_CFTypeRef(), kTISPropertyInputSourceID) };
    if id.is_null() {
        return String::new();
    }
    unsafe { CFString::wrap_under_get_rule(id as CFStringRef) }.to_string()
}
