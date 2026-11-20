use std::collections::HashMap;
use std::path::PathBuf;

const APP: &str = "kitty";
const CACHE_HOME: &str = "XDG_CACHE_HOME";
const HOME: &str = "HOME";
const HOME_CACHE: &str = ".cache";

pub(super) fn default_cache_dir(env: &HashMap<String, String>) -> Option<PathBuf> {
    let absolute = |name: &str| {
        env.get(name)
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
    };
    absolute(CACHE_HOME)
        .or_else(|| absolute(HOME).map(|home| home.join(HOME_CACHE)))
        .map(|cache| cache.join(APP))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn the_cache_follows_the_xdg_cache_home_then_the_home_folder() {
        let cases = [
            (
                vec![("XDG_CACHE_HOME", "/x"), ("HOME", "/home/me")],
                Some("/x/kitty"),
            ),
            (vec![("HOME", "/home/me")], Some("/home/me/.cache/kitty")),
            (
                vec![("XDG_CACHE_HOME", ""), ("HOME", "/home/me")],
                Some("/home/me/.cache/kitty"),
            ),
            (vec![("HOME", "me")], None),
            (vec![], None),
        ];
        for (pairs, expected) in cases {
            assert_eq!(
                default_cache_dir(&env(&pairs)),
                expected.map(PathBuf::from),
                "{pairs:?}"
            );
        }
    }
}
