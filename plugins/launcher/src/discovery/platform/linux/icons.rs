use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

const ICON_SIZE: u32 = 48;
const EXTENSIONS: [&str; 2] = ["png", "svg"];
const FALLBACK_THEME: &str = "hicolor";
const UNKNOWN_FILE_ICONS: [&str; 2] = ["unknown", "text-x-generic"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SizeKind {
    Fixed,
    Scalable,
    Threshold,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IconDir {
    path: String,
    size: u32,
    scale: u32,
    min: u32,
    max: u32,
    threshold: u32,
    kind: SizeKind,
}

struct Theme {
    roots: Vec<PathBuf>,
    dirs: Vec<IconDir>,
}

struct Lookup {
    themes: Vec<Theme>,
    pixmaps: Vec<PathBuf>,
}

#[derive(Default)]
struct MimeDb {
    by_suffix: HashMap<String, (u32, String)>,
    by_name: HashMap<String, (u32, String)>,
    icons: HashMap<String, String>,
    generic: HashMap<String, String>,
}

type Memo = Mutex<HashMap<String, Option<PathBuf>>>;

pub(super) fn icon_path(name: &str) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let as_path = Path::new(name);
    if as_path.is_absolute() {
        return (is_renderable(as_path) && as_path.is_file()).then(|| as_path.to_path_buf());
    }
    if let Some(found) = memo().lock().ok().and_then(|memo| memo.get(name).cloned()) {
        return found;
    }
    let found = lookup().find(strip_image_extension(name));
    if let Ok(mut memo) = memo().lock() {
        memo.insert(name.to_owned(), found.clone());
    }
    found
}

pub(super) fn file_icon(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    mime_db()
        .icon_names(name)
        .iter()
        .map(String::as_str)
        .chain(UNKNOWN_FILE_ICONS)
        .find_map(icon_path)
}

fn memo() -> &'static Memo {
    static MEMO: OnceLock<Memo> = OnceLock::new();
    MEMO.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lookup() -> &'static Lookup {
    static LOOKUP: OnceLock<Lookup> = OnceLock::new();
    LOOKUP.get_or_init(|| Lookup::load(&current_theme(), &data_dirs()))
}

fn mime_db() -> &'static MimeDb {
    static DB: OnceLock<MimeDb> = OnceLock::new();
    DB.get_or_init(|| MimeDb::load(&data_dirs()))
}

fn is_renderable(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

fn strip_image_extension(name: &str) -> &str {
    name.rsplit_once('.')
        .filter(|(_, ext)| EXTENSIONS.contains(ext) || *ext == "xpm")
        .map_or(name, |(stem, _)| stem)
}

pub(super) fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs = Vec::new();
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|home| home.join(".local/share")))
    {
        dirs.push(data_home);
    }
    let shared = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    dirs.extend(
        shared
            .split(':')
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from),
    );
    dirs
}

fn current_theme() -> String {
    let cinnamon_first = std::env::var("XDG_CURRENT_DESKTOP")
        .is_ok_and(|desktop| desktop.to_ascii_lowercase().contains("cinnamon"));
    let schemas = if cinnamon_first {
        [
            "org.cinnamon.desktop.interface",
            "org.gnome.desktop.interface",
        ]
    } else {
        [
            "org.gnome.desktop.interface",
            "org.cinnamon.desktop.interface",
        ]
    };
    schemas
        .into_iter()
        .find_map(gsettings_icon_theme)
        .or_else(gtk_settings_icon_theme)
        .unwrap_or_else(|| FALLBACK_THEME.to_owned())
}

fn gsettings_icon_theme(schema: &str) -> Option<String> {
    let output = Command::new("gsettings")
        .args(["get", schema, "icon-theme"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim().trim_matches('\'');
    (!value.is_empty()).then(|| value.to_owned())
}

fn gtk_settings_icon_theme() -> Option<String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    let content = fs::read_to_string(config.join("gtk-3.0/settings.ini")).ok()?;
    content.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "gtk-icon-theme-name")
            .then(|| value.trim().trim_matches('"').to_owned())
            .filter(|value| !value.is_empty())
    })
}

impl Lookup {
    fn load(theme: &str, data_dirs: &[PathBuf]) -> Self {
        let mut bases = Vec::new();
        if let Some(home) = std::env::var_os("HOME") {
            bases.push(PathBuf::from(home).join(".icons"));
        }
        bases.extend(data_dirs.iter().map(|dir| dir.join("icons")));
        let mut themes = Vec::new();
        let mut seen = HashSet::new();
        collect_theme(theme, &bases, &mut themes, &mut seen);
        collect_theme(FALLBACK_THEME, &bases, &mut themes, &mut seen);
        Self {
            themes,
            pixmaps: vec![PathBuf::from("/usr/share/pixmaps")],
        }
    }

    fn find(&self, name: &str) -> Option<PathBuf> {
        self.themes
            .iter()
            .find_map(|theme| theme.find(name))
            .or_else(|| {
                self.pixmaps
                    .iter()
                    .flat_map(|dir| EXTENSIONS.map(|ext| dir.join(format!("{name}.{ext}"))))
                    .find(|candidate| candidate.is_file())
            })
    }
}

fn collect_theme(
    name: &str,
    bases: &[PathBuf],
    themes: &mut Vec<Theme>,
    seen: &mut HashSet<String>,
) {
    if !seen.insert(name.to_owned()) {
        return;
    }
    let roots: Vec<PathBuf> = bases
        .iter()
        .map(|base| base.join(name))
        .filter(|root| root.is_dir())
        .collect();
    let Some(index) = roots
        .iter()
        .find_map(|root| fs::read_to_string(root.join("index.theme")).ok())
    else {
        return;
    };
    let (parents, dirs) = parse_index(&index);
    themes.push(Theme { roots, dirs });
    for parent in parents {
        collect_theme(&parent, bases, themes, seen);
    }
}

impl Theme {
    fn find(&self, name: &str) -> Option<PathBuf> {
        let mut best: Option<(u32, PathBuf)> = None;
        for dir in &self.dirs {
            let distance = dir.distance(ICON_SIZE);
            if best
                .as_ref()
                .is_some_and(|(nearest, _)| *nearest <= distance)
            {
                continue;
            }
            let found = self.roots.iter().find_map(|root| {
                EXTENSIONS
                    .map(|ext| root.join(&dir.path).join(format!("{name}.{ext}")))
                    .into_iter()
                    .find(|candidate| candidate.is_file())
            });
            if let Some(found) = found {
                best = Some((distance, found));
            }
        }
        best.map(|(_, found)| found)
    }
}

impl IconDir {
    fn distance(&self, size: u32) -> u32 {
        let (low, high) = match self.kind {
            SizeKind::Fixed => (self.size, self.size),
            SizeKind::Scalable => (self.min, self.max),
            SizeKind::Threshold => (
                self.size.saturating_sub(self.threshold),
                self.size + self.threshold,
            ),
        };
        let (low, high) = (low * self.scale, high * self.scale);
        if size < low {
            low - size
        } else {
            size.saturating_sub(high)
        }
    }
}

fn parse_index(content: &str) -> (Vec<String>, Vec<IconDir>) {
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current = String::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            current = section.to_owned();
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            sections
                .entry(current.clone())
                .or_default()
                .insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    let head = sections.get("Icon Theme");
    let list = |key: &str| -> Vec<String> {
        head.and_then(|head| head.get(key))
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    let parents = list("Inherits");
    let dirs = list("Directories")
        .into_iter()
        .chain(list("ScaledDirectories"))
        .filter_map(|path| {
            let section = sections.get(&path)?;
            let number = |key: &str| section.get(key).and_then(|value| value.parse::<u32>().ok());
            let size = number("Size")?;
            Some(IconDir {
                size,
                scale: number("Scale").unwrap_or(1).max(1),
                min: number("MinSize").unwrap_or(size),
                max: number("MaxSize").unwrap_or(size),
                threshold: number("Threshold").unwrap_or(2),
                kind: match section.get("Type").map(String::as_str) {
                    Some("Fixed") => SizeKind::Fixed,
                    Some("Scalable") => SizeKind::Scalable,
                    _ => SizeKind::Threshold,
                },
                path,
            })
        })
        .collect();
    (parents, dirs)
}

impl MimeDb {
    fn load(data_dirs: &[PathBuf]) -> Self {
        let mut db = Self::default();
        for dir in data_dirs.iter().rev().map(|dir| dir.join("mime")) {
            if let Ok(globs) = fs::read_to_string(dir.join("globs2")) {
                db.add_globs(&globs);
            }
            if let Ok(icons) = fs::read_to_string(dir.join("icons")) {
                add_pairs(&mut db.icons, &icons);
            }
            if let Ok(generic) = fs::read_to_string(dir.join("generic-icons")) {
                add_pairs(&mut db.generic, &generic);
            }
        }
        db
    }

    fn add_globs(&mut self, content: &str) {
        for line in content.lines().filter(|line| !line.starts_with('#')) {
            let mut parts = line.splitn(4, ':');
            let (Some(weight), Some(mime), Some(glob)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let Ok(weight) = weight.parse::<u32>() else {
                continue;
            };
            let entry = (weight, mime.to_owned());
            if let Some(suffix) = glob.strip_prefix("*.") {
                if !suffix.contains(['*', '?', '[']) {
                    keep_heavier(&mut self.by_suffix, suffix.to_lowercase(), entry);
                }
            } else if !glob.contains(['*', '?', '[']) {
                keep_heavier(&mut self.by_name, glob.to_lowercase(), entry);
            }
        }
    }

    fn mime_for(&self, file_name: &str) -> Option<&str> {
        let lower = file_name.to_lowercase();
        if let Some((_, mime)) = self.by_name.get(&lower) {
            return Some(mime);
        }
        lower
            .match_indices('.')
            .filter_map(|(at, _)| {
                self.by_suffix
                    .get(&lower[at + 1..])
                    .map(|entry| (lower.len() - at, entry))
            })
            .max_by_key(|(length, (weight, _))| (*weight, *length))
            .map(|(_, (_, mime))| mime.as_str())
    }

    fn icon_names(&self, file_name: &str) -> Vec<String> {
        let Some(mime) = self.mime_for(file_name) else {
            return Vec::new();
        };
        let media = mime.split('/').next().unwrap_or(mime);
        self.icons
            .get(mime)
            .cloned()
            .into_iter()
            .chain([mime.replace('/', "-")])
            .chain(self.generic.get(mime).cloned())
            .chain([format!("{media}-x-generic")])
            .collect()
    }
}

fn keep_heavier(map: &mut HashMap<String, (u32, String)>, key: String, entry: (u32, String)) {
    match map.get(&key) {
        Some((weight, _)) if *weight >= entry.0 => {}
        _ => {
            map.insert(key, entry);
        }
    }
}

fn add_pairs(map: &mut HashMap<String, String>, content: &str) {
    for line in content.lines().filter(|line| !line.starts_with('#')) {
        if let Some((mime, icon)) = line.split_once(':') {
            map.insert(mime.trim().to_owned(), icon.trim().to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = "[Icon Theme]\nName=Test\nInherits=Parent, hicolor\n\
        Directories=16x16/apps,48x48/apps,scalable/mimetypes,24x24/actions,32x32/legacy\n\
        ScaledDirectories=24x24@2x/apps\n\n\
        [16x16/apps]\nContext=Applications\nSize=16\nType=Fixed\n\n\
        [48x48/apps]\nContext=Applications\nSize=48\nType=Fixed\n\n\
        [scalable/mimetypes]\nContext=MimeTypes\nSize=16\nMinSize=8\nMaxSize=512\nType=Scalable\n\n\
        [24x24/actions]\nContext=Actions\nSize=24\n\n\
        [32x32/legacy]\nContext=Legacy\nSize=32\nType=Fixed\n\n\
        [24x24@2x/apps]\nContext=Applications\nSize=24\nScale=2\nType=Fixed\n";

    #[test]
    fn index_parses_parents_and_dirs() {
        let (parents, dirs) = parse_index(INDEX);
        assert_eq!(parents, vec!["Parent".to_owned(), "hicolor".to_owned()]);
        let paths: Vec<_> = dirs.iter().map(|dir| dir.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "16x16/apps",
                "48x48/apps",
                "scalable/mimetypes",
                "24x24/actions",
                "32x32/legacy",
                "24x24@2x/apps",
            ]
        );
        let distance = |path: &str| {
            dirs.iter()
                .find(|dir| dir.path == path)
                .map(|dir| dir.distance(ICON_SIZE))
                .unwrap()
        };
        assert_eq!(distance("48x48/apps"), 0);
        assert_eq!(distance("24x24@2x/apps"), 0);
        assert_eq!(distance("scalable/mimetypes"), 0);
        assert_eq!(distance("16x16/apps"), 32);
    }

    #[test]
    fn theme_lookup_prefers_the_nearest_size_and_falls_back_to_parents() {
        let base = tempfile::TempDir::new().unwrap();
        let theme = base.path().join("Test");
        let parent = base.path().join("Parent");
        for (root, file) in [
            (&theme, "16x16/apps/editor.png"),
            (&theme, "48x48/apps/editor.svg"),
            (&parent, "48x48/apps/terminal.png"),
            (&theme, "scalable/mimetypes/text-x-generic.svg"),
            (&theme, "24x24/actions/cs-date-time.png"),
        ] {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        fs::write(theme.join("index.theme"), INDEX).unwrap();
        fs::write(
            parent.join("index.theme"),
            "[Icon Theme]\nDirectories=48x48/apps\n[48x48/apps]\nContext=Applications\nSize=48\nType=Fixed\n",
        )
        .unwrap();
        let mut themes = Vec::new();
        collect_theme(
            "Test",
            &[base.path().to_path_buf()],
            &mut themes,
            &mut HashSet::new(),
        );
        let lookup = Lookup {
            themes,
            pixmaps: Vec::new(),
        };
        assert_eq!(
            lookup.find("editor"),
            Some(theme.join("48x48/apps/editor.svg"))
        );
        assert_eq!(
            lookup.find("terminal"),
            Some(parent.join("48x48/apps/terminal.png"))
        );
        assert_eq!(
            lookup.find("text-x-generic"),
            Some(theme.join("scalable/mimetypes/text-x-generic.svg"))
        );
        assert_eq!(
            lookup.find("cs-date-time"),
            Some(theme.join("24x24/actions/cs-date-time.png"))
        );
    }

    #[test]
    fn mime_names_follow_the_heaviest_longest_glob() {
        let mut db = MimeDb::default();
        db.add_globs(
            "50:text/x-rust:*.rs\n50:application/gzip:*.gz\n50:application/x-compressed-tar:*.tar.gz\n\
             50:text/x-makefile:makefile\n",
        );
        add_pairs(&mut db.generic, "text/x-rust:text-x-generic\n");
        assert_eq!(db.mime_for("render.RS"), Some("text/x-rust"));
        assert_eq!(db.mime_for("Makefile"), Some("text/x-makefile"));
        assert_eq!(
            db.mime_for("backup.tar.gz"),
            Some("application/x-compressed-tar")
        );
        assert_eq!(db.mime_for("notes"), None);
        assert_eq!(
            db.icon_names("render.rs"),
            vec![
                "text-x-rust".to_owned(),
                "text-x-generic".to_owned(),
                "text-x-generic".to_owned(),
            ]
        );
    }

    #[test]
    fn image_extensions_are_stripped_from_icon_names() {
        assert_eq!(strip_image_extension("firefox.png"), "firefox");
        assert_eq!(
            strip_image_extension("org.gnome.Terminal"),
            "org.gnome.Terminal"
        );
        assert_eq!(strip_image_extension("legacy.xpm"), "legacy");
    }
}
