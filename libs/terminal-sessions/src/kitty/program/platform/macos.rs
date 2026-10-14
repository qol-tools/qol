use std::path::{Path, PathBuf};

pub(super) use super::unix::is_executable;

const BUNDLE_KITTEN: &str = "kitty.app/Contents/MacOS/kitten";

pub(super) fn install_candidates(home: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = vec![Path::new("/Applications").join(BUNDLE_KITTEN)];
    candidates.extend(home.map(|home| home.join("Applications").join(BUNDLE_KITTEN)));
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_bundle_precedes_user_bundle() {
        assert_eq!(
            install_candidates(Some(Path::new("/Users/me"))),
            [
                PathBuf::from("/Applications/kitty.app/Contents/MacOS/kitten"),
                PathBuf::from("/Users/me/Applications/kitty.app/Contents/MacOS/kitten"),
            ]
        );
    }

    #[test]
    fn missing_home_keeps_the_system_bundle() {
        assert_eq!(
            install_candidates(None),
            [PathBuf::from(
                "/Applications/kitty.app/Contents/MacOS/kitten"
            )]
        );
    }
}
