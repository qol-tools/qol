use std::path::PathBuf;

pub fn user_content_folders() -> Vec<PathBuf> {
    Vec::new()
}

pub fn pictures_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join("Pictures"))
}

pub fn videos_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join("Videos"))
}
