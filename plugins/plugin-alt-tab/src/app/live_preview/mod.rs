mod cg;

use crate::delegate::WindowDelegate;
use gpui::{Entity, Task};

const LIVE_PREVIEW_INTERVAL_MS: u64 = 500;

pub(crate) fn spawn(
    delegate: Entity<WindowDelegate>,
    cx: &mut gpui::Context<super::AltTabApp>,
) -> Task<()> {
    cg::spawn(delegate, cx)
}
