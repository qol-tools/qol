mod platform;
mod scan;

use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Local, TimeZone};

use crate::listing::EntryKind;
use scan::{Counted, Scan};

const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "svg"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fact {
    pub label: &'static str,
    pub value: String,
}

impl Fact {
    pub(crate) fn new(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            label,
            value: value.into(),
        }
    }
}

pub(crate) struct Props {
    title: String,
    facts: Vec<Fact>,
    size_at: usize,
    preview: Option<PathBuf>,
    scan: Option<Scan>,
    seen: Option<Counted>,
}

impl Props {
    pub(crate) fn open(
        paths: &[PathBuf],
        file_kind: &'static str,
        wake: impl Fn() + Send + 'static,
    ) -> Option<Self> {
        match paths {
            [] => None,
            [path] => Some(Self::one(path, file_kind, wake)),
            many => Some(Self::many(many, wake)),
        }
    }

    fn one(path: &Path, file_kind: &'static str, wake: impl Fn() + Send + 'static) -> Self {
        let title = path
            .file_name()
            .map_or_else(|| path.to_string_lossy(), |name| name.to_string_lossy())
            .into_owned();
        let link = fs::symlink_metadata(path);
        let resolved = fs::metadata(path);
        let shown = resolved.as_ref().or(link.as_ref());
        let mut facts = vec![
            Fact::new("Name", title.clone()),
            Fact::new(
                "Kind",
                kind_label(link.as_ref().ok(), resolved.as_ref().ok(), file_kind),
            ),
            Fact::new("Path", path.to_string_lossy()),
        ];
        let size_at = facts.len();
        facts.push(Fact::new("Size", ""));
        let mut scan = None;
        match shown {
            Ok(metadata) => {
                if metadata.is_dir() {
                    scan = Some(Scan::start(vec![path.to_path_buf()], wake));
                } else {
                    facts[size_at].value = file_size(metadata.len());
                }
                facts.push(Fact::new("Modified", time_label(metadata.modified())));
                facts.push(Fact::new("Created", time_label(metadata.created())));
                facts.extend(platform::facts(metadata));
            }
            Err(error) => {
                facts[size_at].value = format!("Unavailable: {}", reason(error));
            }
        }
        if let Ok(target) = fs::read_link(path) {
            facts.push(Fact::new("Link target", target.to_string_lossy()));
        }
        let preview = resolved
            .as_ref()
            .ok()
            .filter(|metadata| metadata.is_file())
            .and_then(|_| previewable(path).then(|| path.to_path_buf()));
        Self {
            title,
            facts,
            size_at,
            preview,
            scan,
            seen: None,
        }
    }

    fn many(paths: &[PathBuf], wake: impl Fn() + Send + 'static) -> Self {
        let facts = vec![
            Fact::new("Items", paths.len().to_string()),
            Fact::new("Size", ""),
        ];
        Self {
            title: format!("{} items", paths.len()),
            facts,
            size_at: 1,
            preview: None,
            scan: Some(Scan::start(paths.to_vec(), wake)),
            seen: None,
        }
    }

    pub(crate) fn title(&self) -> &str {
        &self.title
    }

    pub(crate) fn preview(&self) -> Option<&Path> {
        self.preview.as_deref()
    }

    pub(crate) fn counting(&self) -> bool {
        self.scan.as_ref().is_some_and(|scan| !scan.counted().done)
    }

    pub(crate) fn refresh(&mut self) -> bool {
        let counted = self.scan.as_ref().map(Scan::counted);
        let changed = counted != self.seen;
        self.seen = counted;
        changed
    }

    pub(crate) fn rows(&self) -> Vec<Fact> {
        let mut rows = self.facts.clone();
        if let Some(counted) = self.scan.as_ref().map(Scan::counted) {
            rows[self.size_at].value = counted_label(&counted);
        }
        rows
    }
}

fn counted_label(counted: &Counted) -> String {
    let size = qol_gpui::format_bytes(counted.bytes);
    if !counted.done {
        return format!("{size} so far, still counting");
    }
    let items = counted.files + counted.folders;
    let items = qol_gpui::plural(
        usize::try_from(items).unwrap_or(usize::MAX),
        "item",
        "items",
    );
    let mut label = format!("{size} in {items}");
    if counted.unreadable > 0 {
        label.push_str(&format!(
            ", {} could not be read so the total is short",
            counted.unreadable
        ));
    }
    label
}

fn file_size(bytes: u64) -> String {
    format!("{} ({bytes} bytes)", qol_gpui::format_bytes(bytes))
}

fn reason(error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound => "it no longer exists".to_string(),
        std::io::ErrorKind::PermissionDenied => "permission denied".to_string(),
        _ => error.to_string(),
    }
}

pub(crate) fn previewable(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            IMAGE_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
}

fn kind_label(
    link: Option<&Metadata>,
    resolved: Option<&Metadata>,
    file_kind: &'static str,
) -> String {
    let Some(link) = link else {
        return "Unknown".to_string();
    };
    if link.file_type().is_symlink() {
        return match resolved {
            Some(target) if target.is_dir() => "Link to a folder".to_string(),
            Some(_) => "Link to a file".to_string(),
            None => "Broken link".to_string(),
        };
    }
    if link.is_dir() {
        return "Folder".to_string();
    }
    match crate::places::special_kind(&link.file_type()) {
        Some(EntryKind::Pipe) => "Named pipe".to_string(),
        Some(EntryKind::Socket) => "Socket".to_string(),
        Some(EntryKind::Device) => "Device".to_string(),
        _ => file_kind.to_string(),
    }
}

fn time_label(time: std::io::Result<SystemTime>) -> String {
    match time {
        Ok(time) => stamp(&DateTime::<Local>::from(time)),
        Err(_) => "Not recorded".to_string(),
    }
}

fn stamp<Zone: TimeZone>(time: &DateTime<Zone>) -> String
where
    Zone::Offset: std::fmt::Display,
{
    time.format("%Y-%m-%d %H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use chrono::{TimeZone, Utc};

    use super::{counted_label, previewable, stamp, Counted, Props};

    #[test]
    fn only_decodable_image_extensions_get_a_preview() {
        let cases = [
            ("a.png", true),
            ("a.JPG", true),
            ("a.jpeg", true),
            ("a.gif", true),
            ("a.webp", true),
            ("a.svg", true),
            ("a.txt", false),
            ("a.pngx", false),
            ("png", false),
            ("a", false),
        ];
        for (name, expected) in cases {
            assert_eq!(previewable(Path::new(name)), expected, "{name}");
        }
    }

    #[test]
    fn dates_read_year_first_to_the_second() {
        let time = Utc.with_ymd_and_hms(2026, 10, 25, 9, 5, 7).unwrap();
        assert_eq!(stamp(&time), "2026-10-25 09:05:07");
    }

    #[test]
    fn counted_totals_say_when_they_are_partial() {
        let base = Counted {
            bytes: 1_500,
            files: 2,
            folders: 1,
            unreadable: 0,
            done: false,
        };
        assert_eq!(counted_label(&base), "1.5 KB so far, still counting");
        let done = Counted { done: true, ..base };
        assert_eq!(counted_label(&done), "1.5 KB in 3 items");
        let short = Counted {
            unreadable: 2,
            ..done
        };
        assert_eq!(
            counted_label(&short),
            "1.5 KB in 3 items, 2 could not be read so the total is short"
        );
    }

    #[test]
    fn a_file_lists_its_facts_and_a_png_gets_a_preview() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("pic.png");
        fs::write(&path, [0u8; 10]).expect("file");
        let props = Props::open(std::slice::from_ref(&path), "PNG file", || {}).expect("props");
        let rows = props.rows();
        let labels: Vec<&str> = rows.iter().map(|fact| fact.label).collect();
        assert_eq!(
            &labels[..6],
            ["Name", "Kind", "Path", "Size", "Modified", "Created"]
        );
        assert_eq!(rows[0].value, "pic.png");
        assert_eq!(rows[1].value, "PNG file");
        assert_eq!(rows[3].value, "10 B (10 bytes)");
        assert_eq!(props.preview(), Some(path.as_path()));
        assert!(!props.counting());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_shows_its_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("real.txt");
        let link = dir.path().join("alias");
        fs::write(&target, b"hi").expect("file");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        let rows = Props::open(&[link], "Text", || {}).expect("props").rows();
        assert_eq!(rows[1].value, "Link to a file");
        let last = rows.last().expect("row");
        assert_eq!(last.label, "Link target");
        assert_eq!(last.value, target.to_string_lossy());
    }

    #[test]
    fn several_picks_summarise_count_and_start_counting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths: Vec<_> = ["a", "b", "c"]
            .iter()
            .map(|name| {
                let path = dir.path().join(name);
                fs::write(&path, b"xy").expect("file");
                path
            })
            .collect();
        let props = Props::open(&paths, "Text", || {}).expect("props");
        assert_eq!(props.title(), "3 items");
        assert_eq!(props.rows()[0].value, "3");
        assert!(props.preview().is_none());
        assert!(Props::open(&[], "Text", || {}).is_none());
    }
}
