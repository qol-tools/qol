use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XdgDirs {
    pub config_home: PathBuf,
    pub config_dirs: Vec<PathBuf>,
    pub data_home: PathBuf,
    pub data_dirs: Vec<PathBuf>,
}

impl XdgDirs {
    pub fn from_env() -> Option<Self> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let home = lookup("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let home_dir = |tail: &str| home.as_ref().map(|home| home.join(tail));
        let config_home = absolute(lookup("XDG_CONFIG_HOME")).or_else(|| home_dir(".config"))?;
        let data_home = absolute(lookup("XDG_DATA_HOME")).or_else(|| home_dir(".local/share"))?;
        Some(Self {
            config_home,
            config_dirs: dir_list(lookup("XDG_CONFIG_DIRS"), &["/etc/xdg"]),
            data_home,
            data_dirs: dir_list(lookup("XDG_DATA_DIRS"), &["/usr/local/share", "/usr/share"]),
        })
    }
}

pub fn desktops_from(raw: &str) -> Vec<String> {
    raw.split(':')
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| !part.is_empty())
        .collect()
}

fn absolute(value: Option<String>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

fn dir_list(value: Option<String>, defaults: &[&str]) -> Vec<PathBuf> {
    let listed = value
        .iter()
        .flat_map(|value| value.split(':'))
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .collect::<Vec<_>>();
    if !listed.is_empty() {
        return listed;
    }
    defaults.iter().map(PathBuf::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map = pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect::<HashMap<_, _>>();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn defaults_come_from_home() {
        let dirs = XdgDirs::from_lookup(lookup(&[("HOME", "/home/user")])).unwrap();
        assert_eq!(dirs.config_home, PathBuf::from("/home/user/.config"));
        assert_eq!(dirs.data_home, PathBuf::from("/home/user/.local/share"));
        assert_eq!(dirs.config_dirs, vec![PathBuf::from("/etc/xdg")]);
        assert_eq!(
            dirs.data_dirs,
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share")
            ]
        );
    }

    #[test]
    fn explicit_absolute_values_win_and_relative_ones_are_ignored() {
        let cases = [
            (
                "absolute",
                vec![
                    ("HOME", "/home/user"),
                    ("XDG_CONFIG_HOME", "/cfg"),
                    ("XDG_DATA_HOME", "/data"),
                    ("XDG_CONFIG_DIRS", "/a:/b"),
                    ("XDG_DATA_DIRS", "/c:rel:/d"),
                ],
                "/cfg",
                "/data",
                vec!["/a", "/b"],
                vec!["/c", "/d"],
            ),
            (
                "relative homes",
                vec![
                    ("HOME", "/home/user"),
                    ("XDG_CONFIG_HOME", "cfg"),
                    ("XDG_DATA_HOME", ""),
                    ("XDG_CONFIG_DIRS", ""),
                    ("XDG_DATA_DIRS", "rel"),
                ],
                "/home/user/.config",
                "/home/user/.local/share",
                vec!["/etc/xdg"],
                vec!["/usr/local/share", "/usr/share"],
            ),
        ];
        for (label, pairs, config_home, data_home, config_dirs, data_dirs) in cases {
            let dirs = XdgDirs::from_lookup(lookup(&pairs)).unwrap();
            assert_eq!(
                dirs.config_home,
                PathBuf::from(config_home),
                "case: {label}"
            );
            assert_eq!(dirs.data_home, PathBuf::from(data_home), "case: {label}");
            assert_eq!(
                dirs.config_dirs,
                config_dirs
                    .into_iter()
                    .map(PathBuf::from)
                    .collect::<Vec<_>>(),
                "case: {label}"
            );
            assert_eq!(
                dirs.data_dirs,
                data_dirs.into_iter().map(PathBuf::from).collect::<Vec<_>>(),
                "case: {label}"
            );
        }
    }

    #[test]
    fn no_home_and_no_explicit_homes_is_none() {
        assert_eq!(XdgDirs::from_lookup(lookup(&[])), None);
        assert_eq!(XdgDirs::from_lookup(lookup(&[("HOME", "")])), None);
    }

    #[test]
    fn desktops_are_lowercased_components() {
        let cases = [
            ("X-Cinnamon", vec!["x-cinnamon"]),
            ("ubuntu:GNOME", vec!["ubuntu", "gnome"]),
            ("", vec![]),
            (":KDE:", vec!["kde"]),
        ];
        for (raw, expected) in cases {
            assert_eq!(desktops_from(raw), expected, "raw: {raw:?}");
        }
    }
}
