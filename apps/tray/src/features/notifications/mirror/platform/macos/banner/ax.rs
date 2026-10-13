use std::ffi::c_void;

use anyhow::{bail, Result};
use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::runloop::{
    kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopSource, CFRunLoopSourceRef,
};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::geometry::CGPoint;

type ObserverCallback = extern "C" fn(CFTypeRef, CFTypeRef, CFStringRef, *mut c_void);

const AX_SUCCESS: i32 = 0;
const AX_VALUE_CG_POINT: u32 = 1;
const CREATED: &str = "AXCreated";
const WINDOW_CREATED: &str = "AXWindowCreated";

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        attribute: *const c_void,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: CFTypeRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> i32;
    fn AXValueCreate(kind: u32, value: *const c_void) -> CFTypeRef;
    fn AXObserverCreate(pid: i32, callback: ObserverCallback, out: *mut CFTypeRef) -> i32;
    fn AXObserverAddNotification(
        observer: CFTypeRef,
        element: CFTypeRef,
        notification: CFStringRef,
        refcon: *mut c_void,
    ) -> i32;
    fn AXObserverGetRunLoopSource(observer: CFTypeRef) -> CFRunLoopSourceRef;
}

#[derive(Clone)]
pub(super) struct Element(CFType);

impl Element {
    fn attribute(&self, name: &str) -> Option<CFType> {
        let name = CFString::new(name);
        let mut value: CFTypeRef = std::ptr::null();
        let status = unsafe {
            AXUIElementCopyAttributeValue(self.0.as_CFTypeRef(), name.as_CFTypeRef(), &mut value)
        };
        (status == AX_SUCCESS && !value.is_null())
            .then(|| unsafe { CFType::wrap_under_create_rule(value) })
    }

    pub(super) fn text(&self, name: &str) -> Option<String> {
        self.attribute(name)?
            .downcast_into::<CFString>()
            .map(|text| text.to_string())
    }

    pub(super) fn children(&self) -> Vec<Element> {
        let Some(children) = self.attribute("AXChildren") else {
            return Vec::new();
        };
        if children.type_of() != CFArray::<CFType>::type_id() {
            return Vec::new();
        }
        let array: CFArray<CFType> =
            unsafe { CFArray::wrap_under_get_rule(children.as_CFTypeRef() as CFArrayRef) };
        array.iter().map(|child| Element(child.clone())).collect()
    }

    pub(super) fn window(&self) -> Option<Element> {
        if self.text("AXRole").as_deref() == Some("AXWindow") {
            return Some(self.clone());
        }
        self.attribute("AXWindow").map(Element)
    }

    pub(super) fn move_to(&self, x: f64, y: f64) -> Result<()> {
        let point = CGPoint::new(x, y);
        let value =
            unsafe { AXValueCreate(AX_VALUE_CG_POINT, &point as *const CGPoint as *const c_void) };
        if value.is_null() {
            bail!("AXValueCreate failed");
        }
        let value = unsafe { CFType::wrap_under_create_rule(value) };
        let name = CFString::new("AXPosition");
        let status = unsafe {
            AXUIElementSetAttributeValue(
                self.0.as_CFTypeRef(),
                name.as_concrete_TypeRef(),
                value.as_CFTypeRef(),
            )
        };
        if status != AX_SUCCESS {
            bail!("setting AXPosition failed with AX error {status}");
        }
        Ok(())
    }
}

pub(super) struct Observer {
    pub(super) pid: i32,
    _observer: CFType,
    source: CFRunLoopSource,
}

impl Observer {
    pub(super) fn attach(pid: i32, callback: ObserverCallback) -> Result<Self> {
        let mut raw: CFTypeRef = std::ptr::null();
        let status = unsafe { AXObserverCreate(pid, callback, &mut raw) };
        if status != AX_SUCCESS || raw.is_null() {
            bail!("AXObserverCreate failed with AX error {status}");
        }
        let observer = unsafe { CFType::wrap_under_create_rule(raw) };
        let app = unsafe { CFType::wrap_under_create_rule(AXUIElementCreateApplication(pid)) };
        for notification in [CREATED, WINDOW_CREATED] {
            let name = CFString::new(notification);
            let status = unsafe {
                AXObserverAddNotification(
                    observer.as_CFTypeRef(),
                    app.as_CFTypeRef(),
                    name.as_concrete_TypeRef(),
                    std::ptr::null_mut(),
                )
            };
            if status != AX_SUCCESS {
                bail!("watching {notification} failed with AX error {status}");
            }
        }
        let source = unsafe {
            CFRunLoopSource::wrap_under_get_rule(AXObserverGetRunLoopSource(
                observer.as_CFTypeRef(),
            ))
        };
        CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopDefaultMode });
        Ok(Self {
            pid,
            _observer: observer,
            source,
        })
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        CFRunLoop::get_current().remove_source(&self.source, unsafe { kCFRunLoopDefaultMode });
    }
}

pub(super) fn callback_element(element: CFTypeRef) -> Option<Element> {
    (!element.is_null()).then(|| Element(unsafe { CFType::wrap_under_get_rule(element) }))
}
