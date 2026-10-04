#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(target_os = "macos"))]
pub use fallback::{current_layout_id, physical_layout, KeyLayout};
#[cfg(target_os = "macos")]
pub use macos::{current_layout_id, physical_layout, KeyLayout};
