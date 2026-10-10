use std::path::{Path, PathBuf};

use qol_apps::shell_link::LinkTarget;

use crate::core::LeftoverKind;

const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";
const VERBATIM_PREFIX: &str = r"\\?\";
const PROGRAMS_DIR: &str = "Programs";
const APP_DATA_DIR: &str = "AppData";
const LOCAL_LOW_DIR: &str = "LocalLow";
const SYSTEM_DIR: &str = "System32";
const EXE_EXTENSION: &str = ".exe";
const PARENT_DIR: &str = "..";
const SEPARATORS: [char; 2] = ['\\', '/'];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Launch {
    pub(super) program: String,
    pub(super) arguments: String,
}

pub(super) trait PathProbe {
    fn canonical(&self, path: &Path) -> Option<PathBuf>;
    fn is_dir(&self, path: &Path) -> bool;
    fn is_file(&self, path: &Path) -> bool;
}

#[derive(Debug, Clone)]
pub(super) struct Roots {
    pub(super) roaming: Option<PathBuf>,
    pub(super) local: Option<PathBuf>,
    pub(super) program_data: Option<PathBuf>,
    pub(super) windows: Option<PathBuf>,
    pub(super) profile: Option<PathBuf>,
    pub(super) protected: Vec<PathBuf>,
}

impl Roots {
    pub(super) fn data_roots(&self) -> Vec<(LeftoverKind, PathBuf)> {
        let mut roots = Vec::new();
        if let Some(roaming) = &self.roaming {
            roots.push((LeftoverKind::Config, roaming.clone()));
        }
        if let Some(local) = &self.local {
            roots.push((LeftoverKind::Data, local.clone()));
            roots.push((LeftoverKind::Data, join(local, &[PROGRAMS_DIR])));
        }
        if let Some(program_data) = &self.program_data {
            roots.push((LeftoverKind::Data, program_data.clone()));
        }
        roots
    }

    pub(super) fn system32(&self) -> Option<PathBuf> {
        self.windows
            .as_ref()
            .map(|windows| join(windows, &[SYSTEM_DIR]))
    }

    fn app_data_roots(&self) -> Vec<PathBuf> {
        let local_low = self
            .profile
            .as_ref()
            .map(|profile| join(profile, &[APP_DATA_DIR, LOCAL_LOW_DIR]));
        self.roaming
            .iter()
            .chain(self.local.iter())
            .cloned()
            .chain(local_low)
            .collect()
    }

    fn exact_roots(&self) -> Vec<PathBuf> {
        let app_data = self
            .profile
            .as_ref()
            .map(|profile| join(profile, &[APP_DATA_DIR]));
        let programs = self
            .local
            .as_ref()
            .map(|local| join(local, &[PROGRAMS_DIR]));
        self.app_data_roots()
            .into_iter()
            .chain(self.program_data.iter().cloned())
            .chain(self.windows.iter().cloned())
            .chain(self.profile.iter().cloned())
            .chain(app_data)
            .chain(programs)
            .chain(self.protected.iter().cloned())
            .collect()
    }

    fn users_root(&self) -> Option<PathBuf> {
        let profile = self.profile.as_ref()?;
        Some(parent(profile).unwrap_or_else(|| profile.clone()))
    }

    fn in_protected_subtree(&self, path: &Path) -> bool {
        let under_windows = self
            .windows
            .as_ref()
            .is_some_and(|windows| within(path, windows));
        let under_profiles = self.users_root().is_some_and(|users| within(path, &users))
            && !self
                .app_data_roots()
                .iter()
                .any(|root| strictly_within(path, root));
        under_windows || under_profiles
    }

    pub(super) fn is_safe_dir(&self, path: &Path) -> bool {
        is_drive_path(path)
            && !has_parent_dir(path)
            && segments(path).len() >= 2
            && !self.in_protected_subtree(path)
            && !self.exact_roots().iter().any(|root| same_path(path, root))
    }

    pub(super) fn is_system_target(&self, target: &LinkTarget) -> bool {
        match target {
            LinkTarget::ShellFolder => true,
            LinkTarget::Path(path) => self
                .windows
                .as_ref()
                .is_some_and(|windows| within(path, windows)),
            LinkTarget::Advertised | LinkTarget::Unknown => false,
        }
    }
}

pub(super) fn same_path(left: &Path, right: &Path) -> bool {
    path_key(left) == path_key(right)
}

pub(super) fn within(child: &Path, parent: &Path) -> bool {
    let parent = path_key(parent);
    let child = path_key(child);
    !parent.is_empty() && (child == parent || child.starts_with(&format!("{parent}\\")))
}

fn strictly_within(child: &Path, parent: &Path) -> bool {
    within(child, parent) && !same_path(child, parent)
}

pub(super) fn path_key(path: &Path) -> String {
    strip_verbatim(path.to_path_buf())
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

pub(super) fn strip_verbatim(path: PathBuf) -> PathBuf {
    let raw = path.to_string_lossy();
    let unc = raw
        .get(..VERBATIM_UNC_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(VERBATIM_UNC_PREFIX));
    if unc {
        return PathBuf::from(format!(r"\\{}", &raw[VERBATIM_UNC_PREFIX.len()..]));
    }
    match raw.strip_prefix(VERBATIM_PREFIX) {
        Some(rest) => PathBuf::from(rest),
        None => path.clone(),
    }
}

pub(super) fn has_parent_dir(path: &Path) -> bool {
    path.to_string_lossy()
        .split(SEPARATORS)
        .any(|segment| segment == PARENT_DIR)
}

fn is_drive_path(path: &Path) -> bool {
    let key = path_key(path);
    let bytes = key.as_bytes();
    bytes.len() >= 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes.get(2).is_none_or(|separator| *separator == b'\\')
}

fn segments(path: &Path) -> Vec<String> {
    let key = path_key(path);
    let rest = if is_drive_path(path) { &key[2..] } else { &key };
    rest.split('\\')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect()
}

fn parent(path: &Path) -> Option<PathBuf> {
    let raw = path.to_string_lossy();
    let (head, _) = raw.trim_end_matches(SEPARATORS).rsplit_once(SEPARATORS)?;
    (!head.is_empty() && !head.ends_with(':')).then(|| PathBuf::from(head))
}

fn join(base: &Path, parts: &[&str]) -> PathBuf {
    let base = base.to_string_lossy();
    PathBuf::from(format!(
        r"{}\{}",
        base.trim_end_matches(SEPARATORS),
        parts.join(r"\")
    ))
}

pub(super) fn split_command(
    raw: &str,
    system: Option<&Path>,
    probe: &impl PathProbe,
) -> Option<Launch> {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix('"') {
        let (program, arguments) = rest.split_once('"')?;
        return Some(Launch {
            program: resolve_program(program.trim(), system, probe)?,
            arguments: arguments.trim().to_string(),
        });
    }
    let ends: Vec<usize> = raw
        .char_indices()
        .filter(|(_, c)| c.is_whitespace())
        .map(|(index, _)| index)
        .chain(std::iter::once(raw.len()))
        .filter(|end| raw[..*end].ends_with(|c: char| !c.is_whitespace()))
        .collect();
    ends.into_iter().rev().find_map(|end| {
        Some(Launch {
            program: resolve_program(&raw[..end], system, probe)?,
            arguments: raw[end..].trim().to_string(),
        })
    })
}

pub(super) fn resolve_program(
    program: &str,
    system: Option<&Path>,
    probe: &impl PathProbe,
) -> Option<String> {
    if program.is_empty() || has_parent_dir(Path::new(program)) {
        return None;
    }
    let base = if program.contains(SEPARATORS) {
        is_drive_path(Path::new(program)).then(|| program.to_string())?
    } else {
        join(system?, &[program]).to_string_lossy().into_owned()
    };
    let name = base.rsplit(SEPARATORS).next().unwrap_or(&base);
    let with_exe = (!name.contains('.')).then(|| format!("{base}{EXE_EXTENSION}"));
    std::iter::once(base.clone())
        .chain(with_exe)
        .find(|candidate| probe.is_file(Path::new(candidate)))
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(super) struct FakeDisk {
    pub(super) dirs: Vec<PathBuf>,
    pub(super) files: Vec<PathBuf>,
}

#[cfg(test)]
impl FakeDisk {
    pub(super) fn new(dirs: &[&str], files: &[&str]) -> Self {
        Self {
            dirs: dirs.iter().map(PathBuf::from).collect(),
            files: files.iter().map(PathBuf::from).collect(),
        }
    }
}

#[cfg(test)]
impl PathProbe for FakeDisk {
    fn canonical(&self, path: &Path) -> Option<PathBuf> {
        (!has_parent_dir(path)).then(|| strip_verbatim(path.to_path_buf()))
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.iter().any(|dir| same_path(dir, path))
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.iter().any(|file| same_path(file, path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Roots {
        Roots {
            roaming: Some(PathBuf::from(r"C:\Users\me\AppData\Roaming")),
            local: Some(PathBuf::from(r"C:\Users\me\AppData\Local")),
            program_data: Some(PathBuf::from(r"C:\ProgramData")),
            windows: Some(PathBuf::from(r"C:\Windows")),
            profile: Some(PathBuf::from(r"C:\Users\me")),
            protected: vec![
                PathBuf::from(r"C:\Program Files"),
                PathBuf::from(r"C:\Program Files (x86)"),
            ],
        }
    }

    #[test]
    fn path_keys_fold_case_separators_and_verbatim_prefixes() {
        let cases = [
            (r"C:\Program Files\Foo", r"c:\program files\foo"),
            (r"c:/program files/foo/", r"c:\program files\foo"),
            (r"\\?\C:\Program Files\Foo", r"c:\program files\foo"),
            (r"\\?\UNC\server\share\Foo", r"\\server\share\foo"),
            (r"\\?\unc\server\share", r"\\server\share"),
        ];
        for (path, expected) in cases {
            assert_eq!(path_key(Path::new(path)), expected, "{path}");
        }
        assert!(within(
            Path::new(r"\\?\C:\Program Files\Foo\bin"),
            Path::new(r"C:\PROGRAM FILES\foo")
        ));
        assert!(!within(
            Path::new(r"C:\Program Files\Foobar"),
            Path::new(r"C:\Program Files\Foo")
        ));
    }

    #[test]
    fn strip_verbatim_keeps_case_and_rewrites_unc() {
        let cases = [
            (r"\\?\C:\Program Files\Foo", r"C:\Program Files\Foo"),
            (r"\\?\UNC\Server\Share\Foo", r"\\Server\Share\Foo"),
            (r"C:\Plain", r"C:\Plain"),
        ];
        for (path, expected) in cases {
            assert_eq!(
                strip_verbatim(PathBuf::from(path)),
                PathBuf::from(expected),
                "{path}"
            );
        }
    }

    #[test]
    fn safe_dirs_exclude_roots_profile_folders_and_hostile_forms() {
        let roots = roots();
        let cases = [
            (r"C:\Program Files\Foo", true),
            (r"C:\Users\me\AppData\Local\Programs\Foo", true),
            (r"C:\Users\me\AppData\Local\Foo", true),
            (r"C:\Users\me\AppData\Roaming\Foo", true),
            (r"C:\Users\me\AppData\LocalLow\Foo", true),
            (r"C:\ProgramData\Foo", true),
            (r"\\?\C:\Program Files\Foo", true),
            (r"C:\Program Files", false),
            (r"c:\program files\", false),
            (r"\\?\C:\Program Files", false),
            (r"C:\Users\me\AppData\Local\Programs", false),
            (r"C:\Users\me\AppData\Local", false),
            (r"C:\Users\me\AppData\LocalLow", false),
            (r"C:\Users\me\AppData", false),
            (r"C:\Windows\System32\Foo", false),
            (r"C:\Users\me", false),
            (r"C:\Users\me\Documents", false),
            (r"C:\Users\me\Documents\Foo", false),
            (r"C:\Users\me\Desktop\Tool", false),
            (r"C:\Users\me\Downloads\Foo", false),
            (r"C:\Users\me\Pictures", false),
            (r"C:\Users\me\Music", false),
            (r"C:\Users\me\Videos\Foo", false),
            (r"C:\Users\other\Foo", false),
            (r"C:\Users\Public", false),
            (r"C:\Foo", false),
            (r"C:\Program Files\Foo\..\..\Users\me", false),
            (r"C:\Program Files\..\Windows", false),
            (r"\\server\share\Foo", false),
            (r"\\?\UNC\server\share\Foo", false),
            (r"Program Files\Foo", false),
        ];
        for (path, expected) in cases {
            assert_eq!(roots.is_safe_dir(Path::new(path)), expected, "{path}");
        }
    }

    #[test]
    fn data_roots_and_system_dir_come_from_the_roots() {
        let roots = roots();
        let data: Vec<(LeftoverKind, String)> = roots
            .data_roots()
            .into_iter()
            .map(|(kind, path)| (kind, path_key(&path)))
            .collect();
        assert_eq!(
            data,
            [
                (
                    LeftoverKind::Config,
                    r"c:\users\me\appdata\roaming".to_string()
                ),
                (LeftoverKind::Data, r"c:\users\me\appdata\local".to_string()),
                (
                    LeftoverKind::Data,
                    r"c:\users\me\appdata\local\programs".to_string()
                ),
                (LeftoverKind::Data, r"c:\programdata".to_string()),
            ]
        );
        assert_eq!(
            roots.system32(),
            Some(PathBuf::from(r"C:\Windows\System32"))
        );
        let cases = [
            (LinkTarget::ShellFolder, true),
            (LinkTarget::Path(r"C:\WINDOWS\system32\x.exe".into()), true),
            (LinkTarget::Path(r"C:\Program Files\x.exe".into()), false),
            (LinkTarget::Advertised, false),
            (LinkTarget::Unknown, false),
        ];
        for (target, expected) in cases {
            assert_eq!(roots.is_system_target(&target), expected, "{target:?}");
        }
    }

    #[test]
    fn split_command_takes_the_longest_existing_prefix() {
        let system = PathBuf::from(r"C:\Windows\System32");
        let disk = FakeDisk::new(
            &[],
            &[
                r"C:\Program Files\Foo\uninst.exe",
                r"C:\Program Files\Foo Bar\unins000.exe",
                r"C:\Users\me\AppData\Local\Discord\Update.exe",
                r"C:\Tools\remove.exe",
                r"C:\Windows\System32\RunDll32.exe",
                r"C:\Apps\Plain Tool\remove.exe",
                r"C:\Apps\Plain.exe",
            ],
        );
        let cases = [
            (
                r#""C:\Program Files\Foo\uninst.exe" /S"#,
                Some((r"C:\Program Files\Foo\uninst.exe", "/S")),
            ),
            (
                r"C:\Program Files\Foo Bar\unins000.exe /SILENT",
                Some((r"C:\Program Files\Foo Bar\unins000.exe", "/SILENT")),
            ),
            (
                r"C:\Users\me\AppData\Local\Discord\Update.exe --uninstall",
                Some((
                    r"C:\Users\me\AppData\Local\Discord\Update.exe",
                    "--uninstall",
                )),
            ),
            (r"C:\Tools\remove.exe", Some((r"C:\Tools\remove.exe", ""))),
            (r"C:\Tools\remove  /x", Some((r"C:\Tools\remove.exe", "/x"))),
            (
                r"RunDll32 C:\PROGRA~1\Foo\setup.dll,Uninstall",
                Some((
                    r"C:\Windows\System32\RunDll32.exe",
                    r"C:\PROGRA~1\Foo\setup.dll,Uninstall",
                )),
            ),
            (
                r"C:\Apps\Plain Tool\remove.exe /S",
                Some((r"C:\Apps\Plain Tool\remove.exe", "/S")),
            ),
            (r"C:\Program Files\Missing\remove.exe /S", None),
            (r#""C:\Program Files\Missing\remove.exe" /S"#, None),
            (r"Tools\remove.exe", None),
            (r"C:\Program Files\Foo\..\Foo\uninst.exe", None),
            (r#""unterminated"#, None),
            ("", None),
        ];
        for (raw, expected) in cases {
            let expected = expected.map(|(program, arguments)| Launch {
                program: program.to_string(),
                arguments: arguments.to_string(),
            });
            assert_eq!(
                split_command(raw, Some(system.as_path()), &disk),
                expected,
                "{raw}"
            );
        }
        assert_eq!(split_command("RunDll32 x", None, &disk), None);
    }
}
