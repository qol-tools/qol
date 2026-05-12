use objc2_app_kit::NSRunningApplication;

pub fn should_poll_process_focus() -> bool {
    true
}

pub fn has_process_focus() -> bool {
    let app = NSRunningApplication::currentApplication();
    app.isActive()
}
