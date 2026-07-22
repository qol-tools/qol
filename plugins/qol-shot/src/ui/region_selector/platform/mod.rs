#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::{
    hide_cached_guide, identity_rect_mapper, open_cached, pre_create_cached, show_cached_guide,
    SelectorCache,
};
#[cfg(target_os = "macos")]
pub(crate) use macos::{
    hide_cached_guide, identity_rect_mapper, open_cached, pre_create_cached, show_cached_guide,
    SelectorCache,
};
#[cfg(target_os = "windows")]
pub(crate) use windows::{
    hide_cached_guide, identity_rect_mapper, open_cached, pre_create_cached, show_cached_guide,
    SelectorCache,
};
