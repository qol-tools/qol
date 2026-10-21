use std::path::{Component, Path, PathBuf};

pub const APP_CRUMB: &str = "explorer";
const HOME_CRUMB: &str = "Home";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub path: PathBuf,
}

pub fn crumbs<'a>(
    folder: &Path,
    home: &'a Path,
    devices: impl IntoIterator<Item = (&'a str, &'a Path)>,
) -> Vec<Crumb> {
    let mut anchors = vec![(HOME_CRUMB, home)];
    anchors.extend(devices);
    let (label, anchor) = deepest(folder, anchors.iter().map(|(_, path)| *path))
        .map(|index| (anchors[index].0.to_string(), anchors[index].1.to_path_buf()))
        .unwrap_or_else(|| root_of(folder));
    let mut trail = vec![
        Crumb {
            label: APP_CRUMB.to_string(),
            path: home.to_path_buf(),
        },
        Crumb {
            label,
            path: anchor.clone(),
        },
    ];
    let mut path = anchor.clone();
    for part in folder
        .strip_prefix(&anchor)
        .into_iter()
        .flat_map(Path::iter)
    {
        path.push(part);
        trail.push(Crumb {
            label: crate::listing::display_name(part),
            path: path.clone(),
        });
    }
    trail
}

pub fn deepest<'a>(folder: &Path, paths: impl IntoIterator<Item = &'a Path>) -> Option<usize> {
    paths
        .into_iter()
        .enumerate()
        .filter(|(_, path)| folder.starts_with(path))
        .max_by_key(|(_, path)| path.components().count())
        .map(|(index, _)| index)
}

fn root_of(folder: &Path) -> (String, PathBuf) {
    let root: PathBuf = folder
        .components()
        .take_while(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
        .collect();
    (root.display().to_string(), root)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{crumbs, deepest, Crumb, APP_CRUMB};

    fn labels(trail: &[Crumb]) -> Vec<&str> {
        trail.iter().map(|crumb| crumb.label.as_str()).collect()
    }

    #[test]
    fn a_folder_under_home_reads_from_home_and_keeps_its_case() {
        let home = Path::new("/home/kim");
        let folder = Path::new("/home/kim/Git/qol-monorepo/docs/plans");
        let trail = crumbs(folder, home, [("This computer", Path::new("/"))]);
        assert_eq!(
            labels(&trail),
            [APP_CRUMB, "Home", "Git", "qol-monorepo", "docs", "plans"]
        );
        assert_eq!(trail[0].path, home);
        assert_eq!(trail[1].path, home);
        assert_eq!(trail[3].path, PathBuf::from("/home/kim/Git/qol-monorepo"));
        assert_eq!(trail.last().unwrap().path, folder);
    }

    #[test]
    fn a_folder_on_a_drive_reads_from_the_drive_label() {
        let home = Path::new("/home/kim");
        let drives = [
            ("This computer", Path::new("/")),
            ("WD_SN850X", Path::new("/media/kim/WD_SN850X")),
        ];
        let trail = crumbs(Path::new("/media/kim/WD_SN850X/Git"), home, drives);
        assert_eq!(labels(&trail), [APP_CRUMB, "WD_SN850X", "Git"]);
        let trail = crumbs(Path::new("/etc/X11"), home, drives);
        assert_eq!(labels(&trail), [APP_CRUMB, "This computer", "etc", "X11"]);
        assert_eq!(trail[1].path, PathBuf::from("/"));
    }

    #[test]
    fn home_itself_is_one_crumb_after_the_app() {
        let home = Path::new("/home/kim");
        let trail = crumbs(home, home, [("This computer", Path::new("/"))]);
        assert_eq!(labels(&trail), [APP_CRUMB, "Home"]);
        let trail = crumbs(
            Path::new("/home"),
            home,
            [("This computer", Path::new("/"))],
        );
        assert_eq!(labels(&trail), [APP_CRUMB, "This computer", "home"]);
    }

    #[test]
    fn without_an_anchor_the_trail_starts_at_the_root() {
        let none: [(&str, &Path); 0] = [];
        let trail = crumbs(Path::new("/srv/data"), Path::new("/home/kim"), none);
        assert_eq!(labels(&trail)[2..], ["srv", "data"]);
        assert_eq!(trail[1].path, PathBuf::from("/"));
    }

    #[test]
    fn the_deepest_containing_path_wins() {
        let paths = [
            Path::new("/"),
            Path::new("/home/kim"),
            Path::new("/home/kim/Git"),
        ];
        let cases = [
            ("/home/kim/Git/qol", Some(2)),
            ("/home/kim/Music", Some(1)),
            ("/etc", Some(0)),
            ("/home/kimberly", Some(0)),
        ];
        for (folder, expected) in cases {
            assert_eq!(deepest(Path::new(folder), paths), expected, "{folder}");
        }
        assert_eq!(deepest(Path::new("/etc"), [Path::new("/home")]), None);
    }
}
