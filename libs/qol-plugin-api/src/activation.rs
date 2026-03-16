#[cfg(target_os = "macos")]
pub fn set_accessory_policy() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    unsafe { app.setActivationPolicy(NSApplicationActivationPolicy::Accessory) };
}

#[cfg(not(target_os = "macos"))]
pub fn set_accessory_policy() {}
