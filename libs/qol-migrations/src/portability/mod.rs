//! Cross-OS portability helpers used by migrations.
//!
//! These exist because qol-tray profiles are synced between Linux, macOS,
//! and Windows machines via git, and each OS has its own quirks: macOS
//! prefers NFD filenames, Windows defaults to MAX_PATH=260, and git on
//! Windows rewrites line endings unless `.gitattributes` overrides it.

pub mod gitattributes;
pub mod paths;
pub mod unicode;

pub use gitattributes::{ensure_gitattributes, GITATTRIBUTES_CONTENT};
pub use paths::{ensure_path_within_platform_limit, validate_profile_name};
pub use unicode::normalize_to_nfc;
