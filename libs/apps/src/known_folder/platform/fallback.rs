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

pub fn programs_dir() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_windows_has_a_start_menu_programs_folder() {
        assert_eq!(programs_dir(), None);
    }
}
