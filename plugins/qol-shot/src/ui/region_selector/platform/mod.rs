#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub(crate) use linux::{
    hide_cached_guide, identity_rect_mapper, open_cached, pre_create_cached, show_cached_guide,
    SelectorCache,
};
