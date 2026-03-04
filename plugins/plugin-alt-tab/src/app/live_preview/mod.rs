mod cg;

use crate::picker::state::PickerState;
use gpui::{Entity, Task};

const LIVE_PREVIEW_INTERVAL_MS: u64 = 500;

pub(crate) fn spawn(
    delegate: Entity<PickerState>,
    cx: &mut gpui::Context<super::AltTabApp>,
) -> Task<()> {
    cg::spawn(delegate, cx)
}
