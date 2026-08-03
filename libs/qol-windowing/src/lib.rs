//! Shared window model for qol-tools window consumers (alt-tab, window-actions).
//!
//! Owns the platform-neutral window identity, geometry, and the window
//! operation contract. Platform backends live in the consuming plugins and
//! implement [`WindowOps`]; this crate holds only pure types and the trait.

mod geometry;
mod ops;
mod window_id;

pub use geometry::WindowRect;
pub use ops::WindowOps;
pub use window_id::WindowId;
