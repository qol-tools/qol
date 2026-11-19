use std::collections::HashMap;
use std::path::PathBuf;

const HOME: &str = "HOME";
const HOME_CACHE: &str = "Library/Caches/kitty";

pub(super) fn default_cache_dir(env: &HashMap<String, String>) -> Option<PathBuf> {
    env.get(HOME)
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .map(|home| home.join(HOME_CACHE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_lives_in_the_library_of_the_home_folder() {
        let home = |home: &str| HashMap::from([("HOME".to_owned(), home.to_owned())]);
        assert_eq!(
            default_cache_dir(&home("/Users/me")),
            Some(PathBuf::from("/Users/me/Library/Caches/kitty"))
        );
        assert_eq!(default_cache_dir(&home("me")), None);
        assert_eq!(default_cache_dir(&HashMap::new()), None);
    }
}
