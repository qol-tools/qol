pub(crate) mod warning;

use std::sync::Arc;
use std::time::Duration;

use qol_runtime::protocol::NotificationLevel;

use super::input::InputState;
use warning::SecureInputWatch;

const POLL_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) fn watch(input: Arc<InputState>) {
    std::thread::Builder::new()
        .name("keyremap-secure-input".into())
        .spawn(move || {
            let mut watch = SecureInputWatch::default();
            let client = qol_runtime::PlatformStateClient::from_env();
            loop {
                let holder = qol_platform::secure_input_holder();
                if let Some(toast) = watch.observe(holder.as_ref(), input.strategy.get()) {
                    log::warn!("{}", toast.title);
                    if !client.send_notification(&toast.title, &toast.body, NotificationLevel::Warn)
                    {
                        log::warn!("could not show the Secure Input warning in qol-tray");
                    }
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        })
        .expect("failed to spawn the Secure Input watch thread");
}
