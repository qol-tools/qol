mod cg;
#[cfg(target_os = "macos")]
mod perf;
#[cfg(target_os = "macos")]
mod sc;

use crate::delegate::WindowDelegate;
use crate::platform;
use gpui::{Entity, Task};

const LIVE_PREVIEW_INTERVAL_MS: u64 = 500;
const SC_POLL_INTERVAL_MS: u64 = 33; // ~30fps visual refresh

pub(crate) fn spawn(
    delegate: Entity<WindowDelegate>,
    cx: &mut gpui::Context<super::AltTabApp>,
) -> Task<()> {
    #[cfg(target_os = "macos")]
    if platform::sc_available() {
        return sc::spawn(delegate, cx);
    }
    cg::spawn(delegate, cx)
}
