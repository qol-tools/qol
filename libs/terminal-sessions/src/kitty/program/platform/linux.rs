use std::path::{Path, PathBuf};

pub(super) use super::unix::is_executable;

pub(super) fn install_candidates(home: Option<&Path>) -> Vec<PathBuf> {
    home.map(|home| home.join(".local/kitty.app/bin/kitten"))
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_install_lives_under_local_kitty_app() {
        assert_eq!(
            install_candidates(Some(Path::new("/home/me"))),
            [PathBuf::from("/home/me/.local/kitty.app/bin/kitten")]
        );
    }

    #[test]
    fn missing_home_has_no_candidates() {
        assert!(install_candidates(None).is_empty());
    }
}
