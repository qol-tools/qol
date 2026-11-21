use std::path::Path;

use crate::platform::Refusal;
use crate::updates::{AutoKey, Cleaner, Cleanup};

mod mint;
mod unattended;

const UNATTENDED: &str = "/usr/bin/unattended-upgrade";
const NONE: &str = "Nothing here removes old kernels on its own";

fn cleaner() -> Option<Cleaner> {
    if super::is_mint() {
        Some(Cleaner::Mint)
    } else if Path::new(UNATTENDED).exists() {
        Some(Cleaner::Unattended)
    } else {
        None
    }
}

pub(super) fn read(running: Option<&str>) -> Option<Cleanup> {
    match cleaner()? {
        Cleaner::Mint => mint::read(running),
        Cleaner::Unattended => Some(unattended::read()),
    }
}

pub(super) fn set(key: AutoKey, on: bool) -> Result<(), Refusal> {
    match (cleaner(), key) {
        (Some(Cleaner::Mint), AutoKey::Kernels) => mint::set_removals(on),
        (Some(Cleaner::Mint), AutoKey::ByHand) => mint::set_by_hand(on),
        (Some(Cleaner::Unattended), AutoKey::Kernels) => unattended::set(on),
        _ => Err(Refusal::Failed(NONE.into())),
    }
}
