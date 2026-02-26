use std::sync::{Arc, RwLock};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub struct AppTracker {
    bundle_id: Arc<RwLock<String>>,
}

impl AppTracker {
    pub fn start() -> Arc<Self> {
        let bundle_id = Arc::new(RwLock::new(
            frontmost_bundle_id().unwrap_or_default(),
        ));

        let poll_ref = Arc::clone(&bundle_id);
        std::thread::spawn(move || loop {
            if let Some(id) = frontmost_bundle_id() {
                if let Ok(mut guard) = poll_ref.write() {
                    *guard = id;
                }
            }
            std::thread::sleep(POLL_INTERVAL);
        });

        Arc::new(Self { bundle_id })
    }

    pub fn bundle_id(&self) -> String {
        self.bundle_id.read().map(|g| g.clone()).unwrap_or_default()
    }
}

fn frontmost_bundle_id() -> Option<String> {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSRunningApplication, NSWorkspace};

    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let app: objc2::rc::Retained<NSRunningApplication> =
            workspace.frontmostApplication()?;
        let bundle_id = app.bundleIdentifier()?;
        Some(bundle_id.to_string())
    })
}
