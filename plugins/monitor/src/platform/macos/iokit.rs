use std::ffi::{c_char, c_void};

use crate::monitor::backends::avservice::{
    pnp_to_vendor_id, AvError, AvService, AvServiceInfo, AvSymbolResolver, AvTransport,
    DisplayIdentity, ResolvedSymbols, TransportClass,
};

const I2C_BUS_ID: u32 = 0x37;
const I2C_DATA_ADDRESS: u32 = 0x51;

const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const K_CF_COMPARE_EQUAL: isize = 0;
const K_IOREGISTRY_ITERATE_RECURSIVELY: u32 = 1;
const K_CF_NUMBER_SINT64: isize = 4;

type CreateWithServiceFn = unsafe extern "C" fn(u32, u32) -> *mut c_void;
type WriteI2cFn = unsafe extern "C" fn(*mut c_void, u32, u32, *const u8, u32) -> i32;
type ReadI2cFn = unsafe extern "C" fn(*mut c_void, u32, u32, *mut u8, u32) -> i32;

extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOIteratorNext(iterator: u32) -> u32;
    fn IORegistryEntryCreateCFProperty(
        entry: u32,
        key: *const c_void,
        allocator: *const c_void,
        options: u32,
    ) -> *const c_void;
    fn IORegistryEntryGetParentIterator(
        entry: u32,
        plane: *const c_char,
        iterator: *mut u32,
    ) -> i32;
    fn IORegistryEntryGetName(entry: u32, name: *mut c_char) -> i32;
    fn IORegistryGetRootEntry(main_port: u32) -> u32;
    fn IORegistryEntryCreateIterator(
        entry: u32,
        plane: *const c_char,
        options: u32,
        iterator: *mut u32,
    ) -> i32;
    fn IOObjectRelease(object: u32) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        c_string: *const c_char,
        encoding: u32,
    ) -> *const c_void;
    fn CFStringCompare(string1: *const c_void, string2: *const c_void, options: u64) -> isize;
    fn CFStringGetCString(
        the_string: *const c_void,
        buffer: *mut c_char,
        buffer_size: isize,
        encoding: u32,
    ) -> u8;
    fn CFDictionaryGetValue(the_dict: *const c_void, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(number: *const c_void, the_type: isize, value_ptr: *mut c_void) -> u8;
    fn CFRelease(value: *const c_void);
    fn CFGetTypeID(value: *const c_void) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFNumberGetTypeID() -> usize;
}

fn cf_string(text: *const c_char) -> *const c_void {
    unsafe { CFStringCreateWithCString(std::ptr::null(), text, K_CF_STRING_ENCODING_UTF8) }
}

fn cf_string_contents(value: *const c_void) -> Option<String> {
    if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { CFStringGetTypeID() } {
        return None;
    }
    let mut buffer = [0i8; 128];
    let ok = unsafe {
        CFStringGetCString(
            value,
            buffer.as_mut_ptr(),
            buffer.len() as isize,
            K_CF_STRING_ENCODING_UTF8,
        )
    };
    if ok == 0 {
        return None;
    }
    unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }
        .to_str()
        .ok()
        .map(str::to_string)
}

fn registry_property(entry: u32, key: *const c_void) -> Option<*const c_void> {
    let property = unsafe { IORegistryEntryCreateCFProperty(entry, key, std::ptr::null(), 0) };
    if property.is_null() {
        None
    } else {
        Some(property)
    }
}

fn entry_name(entry: u32) -> Option<String> {
    let mut buffer = [0i8; 128];
    let result = unsafe { IORegistryEntryGetName(entry, buffer.as_mut_ptr()) };
    if result != 0 {
        return None;
    }
    unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }
        .to_str()
        .ok()
        .map(str::to_string)
}

fn product_identity(
    entry: u32,
    display_attributes_key: *const c_void,
    product_attributes_key: *const c_void,
    manufacturer_key: *const c_void,
    product_key: *const c_void,
    serial_key: *const c_void,
) -> Option<DisplayIdentity> {
    let attributes = registry_property(entry, display_attributes_key)?;
    let product = unsafe { CFDictionaryGetValue(attributes, product_attributes_key) };
    if product.is_null() {
        unsafe { CFRelease(attributes) };
        return None;
    }
    let manufacturer = unsafe { CFDictionaryGetValue(product, manufacturer_key) };
    let vendor = cf_string_contents(manufacturer).map(|pnp| pnp_to_vendor_id(&pnp));
    let product_id = cf_u32_contents(unsafe { CFDictionaryGetValue(product, product_key) });
    let serial = cf_u32_contents(unsafe { CFDictionaryGetValue(product, serial_key) });
    unsafe { CFRelease(attributes) };
    Some(DisplayIdentity {
        vendor: vendor?,
        model: product_id?,
        serial: serial?,
    })
}

fn cf_u32_contents(value: *const c_void) -> Option<u32> {
    if value.is_null() {
        return None;
    }
    if unsafe { CFGetTypeID(value) } == unsafe { CFNumberGetTypeID() } {
        let mut number: i64 = 0;
        let ok = unsafe {
            CFNumberGetValue(
                value,
                K_CF_NUMBER_SINT64,
                &mut number as *mut i64 as *mut c_void,
            )
        };
        if ok == 0 {
            return None;
        }
        return u32::try_from(number).ok();
    }
    parse_serial(&cf_string_contents(value)?)
}

fn transport_class(proxy: u32, epic_key: *const c_void) -> TransportClass {
    let mut converter_routed = false;
    if let Some(own) = registry_property(proxy, epic_key) {
        converter_routed = is_converter_class(&cf_string_contents(own));
        unsafe { CFRelease(own) };
    }
    let mut iterator = 0u32;
    let result =
        unsafe { IORegistryEntryGetParentIterator(proxy, c"IOService".as_ptr(), &mut iterator) };
    if result == 0 && iterator != 0 {
        loop {
            let parent = unsafe { IOIteratorNext(iterator) };
            if parent == 0 {
                break;
            }
            if let Some(class) = registry_property(parent, epic_key) {
                if is_converter_class(&cf_string_contents(class)) {
                    converter_routed = true;
                }
                unsafe { CFRelease(class) };
            }
            unsafe { IOObjectRelease(parent) };
        }
        unsafe { IOObjectRelease(iterator) };
    }
    if converter_routed {
        TransportClass::ConverterRouted
    } else {
        TransportClass::DirectDp
    }
}

fn is_converter_class(class: &Option<String>) -> bool {
    matches!(
        class.as_deref(),
        Some("AppleDCPMCDP29XX") | Some("AppleDCPPS190")
    )
}

pub struct DlsymResolver;

impl AvSymbolResolver for DlsymResolver {
    fn resolve(&self, name: &str) -> Result<*mut c_void, AvError> {
        let c_name = std::ffi::CString::new(name).map_err(|_| AvError::MissingSymbol {
            name: name.to_string(),
        })?;
        let symbol = unsafe { dlsym(RTLD_DEFAULT, c_name.as_ptr()) };
        if symbol.is_null() {
            return Err(AvError::MissingSymbol {
                name: name.to_string(),
            });
        }
        Ok(symbol)
    }
}

pub struct IokitAvTransport {
    symbols: ResolvedSymbols<DlsymResolver>,
}

impl IokitAvTransport {
    pub fn new() -> Self {
        Self {
            symbols: ResolvedSymbols::new(DlsymResolver),
        }
    }
}

impl Default for IokitAvTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl AvTransport for IokitAvTransport {
    fn list_external_services(&self) -> Result<Vec<AvServiceInfo>, AvError> {
        let create_with_service = self.symbols.get("IOAVServiceCreateWithService")?;
        let create_with_service: CreateWithServiceFn =
            unsafe { std::mem::transmute(create_with_service) };
        let location_key = cf_string(c"Location".as_ptr());
        let external = cf_string(c"External".as_ptr());
        let display_attributes_key = cf_string(c"DisplayAttributes".as_ptr());
        let product_attributes_key = cf_string(c"ProductAttributes".as_ptr());
        let manufacturer_key = cf_string(c"ManufacturerID".as_ptr());
        let product_key = cf_string(c"ProductID".as_ptr());
        let serial_key = cf_string(c"SerialNumber".as_ptr());
        let epic_key = cf_string(c"EPICProviderClass".as_ptr());
        let mut services = Vec::new();
        let root = unsafe { IORegistryGetRootEntry(0) };
        let mut iterator = 0u32;
        let result = unsafe {
            IORegistryEntryCreateIterator(
                root,
                c"IOService".as_ptr(),
                K_IOREGISTRY_ITERATE_RECURSIVELY,
                &mut iterator,
            )
        };
        if result != 0 || iterator == 0 {
            unsafe { IOObjectRelease(root) };
            return Err(AvError::Unsupported {
                detail: format!("io registry iteration failed with code {result}"),
            });
        }
        let mut pending_identity: Option<DisplayIdentity> = None;
        loop {
            let entry = unsafe { IOIteratorNext(iterator) };
            if entry == 0 {
                break;
            }
            match entry_name(entry).as_deref() {
                Some("AppleCLCD2") | Some("IOMobileFramebufferShim") => {
                    pending_identity = product_identity(
                        entry,
                        display_attributes_key,
                        product_attributes_key,
                        manufacturer_key,
                        product_key,
                        serial_key,
                    );
                }
                Some("DCPAVServiceProxy") => {
                    let location = registry_property(entry, location_key);
                    let is_external = location.is_some_and(|property| unsafe {
                        CFStringCompare(property, external, 0) == K_CF_COMPARE_EQUAL
                    });
                    if let Some(property) = location {
                        unsafe { CFRelease(property) };
                    }
                    if is_external {
                        if let Some(identity) = pending_identity {
                            let service = unsafe { create_with_service(0, entry) };
                            if !service.is_null() {
                                services.push(AvServiceInfo {
                                    service: AvService::new(service),
                                    identity,
                                    transport_class: transport_class(entry, epic_key),
                                });
                            }
                        }
                    }
                    pending_identity = None;
                }
                _ => {}
            }
            unsafe { IOObjectRelease(entry) };
        }
        unsafe {
            IOObjectRelease(iterator);
            IOObjectRelease(root);
        }
        unsafe {
            CFRelease(location_key);
            CFRelease(external);
            CFRelease(display_attributes_key);
            CFRelease(product_attributes_key);
            CFRelease(manufacturer_key);
            CFRelease(product_key);
            CFRelease(serial_key);
            CFRelease(epic_key);
        }
        Ok(services)
    }

    fn write(&self, service: &AvService, payload: &[u8]) -> Result<(), AvError> {
        let write_i2c = self.symbols.get("IOAVServiceWriteI2C")?;
        let write_i2c: WriteI2cFn = unsafe { std::mem::transmute(write_i2c) };
        let result = unsafe {
            write_i2c(
                service.raw(),
                I2C_BUS_ID,
                I2C_DATA_ADDRESS,
                payload.as_ptr(),
                payload.len() as u32,
            )
        };
        if result != 0 {
            return Err(AvError::Unsupported {
                detail: format!("IOAVServiceWriteI2C failed with code {result}"),
            });
        }
        Ok(())
    }

    fn read(&self, service: &AvService, buffer: &mut [u8]) -> Result<(), AvError> {
        let read_i2c = self.symbols.get("IOAVServiceReadI2C")?;
        let read_i2c: ReadI2cFn = unsafe { std::mem::transmute(read_i2c) };
        let result = unsafe {
            read_i2c(
                service.raw(),
                I2C_BUS_ID,
                0,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
            )
        };
        if result != 0 {
            return Err(AvError::Unsupported {
                detail: format!("IOAVServiceReadI2C failed with code {result}"),
            });
        }
        Ok(())
    }
}

fn parse_serial(text: &str) -> Option<u32> {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).ok();
    }
    text.parse().ok()
}
