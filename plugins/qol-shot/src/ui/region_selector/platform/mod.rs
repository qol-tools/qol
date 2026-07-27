#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub(crate) use linux::{identity_rect_mapper, open_cached, pre_create_cached, SelectorCache};
