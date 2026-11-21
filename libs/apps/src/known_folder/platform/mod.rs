#[cfg(not(target_os = "windows"))]
mod fallback;
#[cfg(target_os = "windows")]
pub(crate) mod windows;

#[cfg(not(target_os = "windows"))]
pub use fallback::{
    common_programs_dir, pictures_dir, programs_dir, user_content_folders, videos_dir,
};
#[cfg(target_os = "windows")]
pub use windows::{
    common_programs_dir, pictures_dir, programs_dir, user_content_folders, videos_dir,
};
