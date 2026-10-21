#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FileKind {
    Text,
    Document,
    Sheet,
    Markdown,
    Log,
    Config,
    Data,
    Book,
    Slides,
    Image,
    Video,
    Audio,
    Font,
    Code,
    Script,
    Web,
    Binary,
    Archive,
    Package,
    Disc,
    Database,
    Key,
    Link,
    Unknown,
}

const ALIAS: [(&str, FileKind); 8] = [
    ("csv", FileKind::Sheet),
    ("rtf", FileKind::Document),
    ("css", FileKind::Web),
    ("key", FileKind::Key),
    ("out", FileKind::Log),
    ("jsonl", FileKind::Data),
    ("gz", FileKind::Archive),
    ("tgz", FileKind::Archive),
];

const TAG: [(&str, &str); 7] = [
    ("sqlite", "DB"),
    ("desktop", "LNK"),
    ("markdown", "MD"),
    ("jpeg", "JPG"),
    ("flatpak", "PAK"),
    ("appimage", "APP"),
    ("woff2", "WOFF"),
];

const TAG_LIMIT: usize = 4;

impl FileKind {
    pub(crate) const ALL: [Self; 24] = [
        Self::Text,
        Self::Document,
        Self::Sheet,
        Self::Markdown,
        Self::Log,
        Self::Config,
        Self::Data,
        Self::Book,
        Self::Slides,
        Self::Image,
        Self::Video,
        Self::Audio,
        Self::Font,
        Self::Code,
        Self::Script,
        Self::Web,
        Self::Binary,
        Self::Archive,
        Self::Package,
        Self::Disc,
        Self::Database,
        Self::Key,
        Self::Link,
        Self::Unknown,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Document => "Document",
            Self::Sheet => "Spreadsheet",
            Self::Markdown => "Markdown",
            Self::Log => "Log",
            Self::Config => "Settings file",
            Self::Data => "Data",
            Self::Book => "E-book",
            Self::Slides => "Presentation",
            Self::Image => "Image",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Font => "Font",
            Self::Code => "Source code",
            Self::Script => "Shell script",
            Self::Web => "Web page",
            Self::Binary => "Program",
            Self::Archive => "Archive",
            Self::Package => "Package",
            Self::Disc => "Disk image",
            Self::Database => "Database",
            Self::Key => "Key or certificate",
            Self::Link => "Shortcut",
            Self::Unknown => "Anything else",
        }
    }

    pub(crate) fn tag_color(self) -> u32 {
        match self {
            Self::Text => 0xa9a49a,
            Self::Document => 0xe5675c,
            Self::Sheet => 0x5cbf7a,
            Self::Markdown => 0xcfcbc3,
            Self::Log => 0x8b8880,
            Self::Config => 0x9daac0,
            Self::Data => 0xa77ef0,
            Self::Book => 0xd98ad6,
            Self::Slides => 0xe8a53a,
            Self::Image => 0x45b8b0,
            Self::Video => 0xe8668c,
            Self::Audio => 0x4cb6e6,
            Self::Font => 0xd6b98a,
            Self::Code => 0xde7a45,
            Self::Script => 0x78c896,
            Self::Web => 0x4f8ff0,
            Self::Binary => 0xf0cf4a,
            Self::Archive => 0xb8875a,
            Self::Package => 0x8fa36a,
            Self::Disc => 0xb7bcc6,
            Self::Database => 0x7d9bb8,
            Self::Key => 0xa6c85a,
            Self::Link => 0x9bb0c4,
            Self::Unknown => 0x8b8880,
        }
    }

    pub(crate) fn asset_stem(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Document => "document",
            Self::Sheet => "sheet",
            Self::Markdown => "markdown",
            Self::Log => "log",
            Self::Config => "config",
            Self::Data => "data",
            Self::Book => "book",
            Self::Slides => "slides",
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Font => "font",
            Self::Code => "code",
            Self::Script => "script",
            Self::Web => "web",
            Self::Binary => "binary",
            Self::Archive => "archive",
            Self::Package => "package",
            Self::Disc => "disc",
            Self::Database => "database",
            Self::Key => "key",
            Self::Link => "link",
            Self::Unknown => "unknown",
        }
    }

    fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Text => &["txt", "text"],
            Self::Document => &["pdf", "odt", "docx", "rtf"],
            Self::Sheet => &["ods", "xlsx", "csv"],
            Self::Markdown => &["md", "markdown"],
            Self::Log => &["log", "out"],
            Self::Config => &["conf", "toml", "ini", "yaml", "cfg"],
            Self::Data => &["json", "xml", "jsonl"],
            Self::Book => &["epub", "mobi"],
            Self::Slides => &["odp", "pptx", "key"],
            Self::Image => &["png", "jpg", "jpeg", "webp", "gif", "svg"],
            Self::Video => &["mp4", "mkv", "webm", "mov"],
            Self::Audio => &["flac", "mp3", "ogg", "wav", "opus"],
            Self::Font => &["ttf", "otf", "woff2"],
            Self::Code => &["rs", "js", "ts", "py", "c", "go"],
            Self::Script => &["sh", "bash", "zsh", "fish"],
            Self::Web => &["html", "htm", "css"],
            Self::Binary => &["bin", "exe", "run"],
            Self::Archive => &["zip", "tar.gz", "7z", "xz", "rar"],
            Self::Package => &["deb", "rpm", "appimage", "flatpak"],
            Self::Disc => &["iso", "img"],
            Self::Database => &["sqlite", "db"],
            Self::Key => &["gpg", "asc", "pem", "key", "crt"],
            Self::Link => &["desktop", "lnk", "url"],
            Self::Unknown => &[],
        }
    }
}

pub(crate) fn kind_of(name: &str, executable: bool) -> FileKind {
    let Some(extension) = extension(&name.to_lowercase()) else {
        return if executable {
            FileKind::Binary
        } else {
            FileKind::Unknown
        };
    };
    if let Some((_, kind)) = ALIAS.iter().find(|(alias, _)| *alias == extension) {
        return *kind;
    }
    FileKind::ALL
        .into_iter()
        .find(|kind| kind.extensions().contains(&extension.as_str()))
        .unwrap_or(FileKind::Unknown)
}

pub(crate) fn tag_text(name: &str, kind: FileKind) -> String {
    let source = extension(name)
        .map(|extension| {
            let lower = extension.to_ascii_lowercase();
            TAG.iter()
                .find(|(from, _)| *from == lower)
                .map_or(extension, |(_, to)| (*to).to_string())
        })
        .unwrap_or_else(|| kind.extensions().first().copied().unwrap_or("").to_string());
    let tag = source
        .chars()
        .take(TAG_LIMIT)
        .collect::<String>()
        .to_uppercase();
    if !tag.is_empty() {
        return tag;
    }
    let fallback = if kind == FileKind::Binary { "BIN" } else { "?" };
    fallback.to_string()
}

fn extension(name: &str) -> Option<String> {
    let (_, extension) = name.rsplit_once('.')?;
    let plain = extension.bytes().all(|byte| byte.is_ascii_alphanumeric());
    (plain && !extension.is_empty()).then(|| extension.to_string())
}

#[cfg(test)]
mod tests {
    use super::{kind_of, tag_text, FileKind};

    #[test]
    fn every_listed_extension_and_alias_names_its_kind() {
        let cases = [
            ("txt", FileKind::Text),
            ("text", FileKind::Text),
            ("pdf", FileKind::Document),
            ("odt", FileKind::Document),
            ("docx", FileKind::Document),
            ("rtf", FileKind::Document),
            ("ods", FileKind::Sheet),
            ("xlsx", FileKind::Sheet),
            ("csv", FileKind::Sheet),
            ("md", FileKind::Markdown),
            ("markdown", FileKind::Markdown),
            ("log", FileKind::Log),
            ("out", FileKind::Log),
            ("conf", FileKind::Config),
            ("toml", FileKind::Config),
            ("ini", FileKind::Config),
            ("yaml", FileKind::Config),
            ("cfg", FileKind::Config),
            ("json", FileKind::Data),
            ("xml", FileKind::Data),
            ("jsonl", FileKind::Data),
            ("epub", FileKind::Book),
            ("mobi", FileKind::Book),
            ("odp", FileKind::Slides),
            ("pptx", FileKind::Slides),
            ("png", FileKind::Image),
            ("jpg", FileKind::Image),
            ("webp", FileKind::Image),
            ("gif", FileKind::Image),
            ("svg", FileKind::Image),
            ("mp4", FileKind::Video),
            ("mkv", FileKind::Video),
            ("webm", FileKind::Video),
            ("mov", FileKind::Video),
            ("flac", FileKind::Audio),
            ("mp3", FileKind::Audio),
            ("ogg", FileKind::Audio),
            ("wav", FileKind::Audio),
            ("opus", FileKind::Audio),
            ("ttf", FileKind::Font),
            ("otf", FileKind::Font),
            ("woff2", FileKind::Font),
            ("rs", FileKind::Code),
            ("js", FileKind::Code),
            ("ts", FileKind::Code),
            ("py", FileKind::Code),
            ("c", FileKind::Code),
            ("go", FileKind::Code),
            ("sh", FileKind::Script),
            ("bash", FileKind::Script),
            ("zsh", FileKind::Script),
            ("fish", FileKind::Script),
            ("html", FileKind::Web),
            ("htm", FileKind::Web),
            ("css", FileKind::Web),
            ("bin", FileKind::Binary),
            ("exe", FileKind::Binary),
            ("run", FileKind::Binary),
            ("zip", FileKind::Archive),
            ("7z", FileKind::Archive),
            ("xz", FileKind::Archive),
            ("rar", FileKind::Archive),
            ("gz", FileKind::Archive),
            ("tgz", FileKind::Archive),
            ("deb", FileKind::Package),
            ("rpm", FileKind::Package),
            ("AppImage", FileKind::Package),
            ("flatpak", FileKind::Package),
            ("iso", FileKind::Disc),
            ("img", FileKind::Disc),
            ("sqlite", FileKind::Database),
            ("db", FileKind::Database),
            ("gpg", FileKind::Key),
            ("asc", FileKind::Key),
            ("pem", FileKind::Key),
            ("key", FileKind::Key),
            ("crt", FileKind::Key),
            ("desktop", FileKind::Link),
            ("lnk", FileKind::Link),
            ("url", FileKind::Link),
            ("xyz", FileKind::Unknown),
        ];
        for (extension, kind) in cases {
            assert_eq!(
                kind_of(&format!("file.{extension}"), false),
                kind,
                "{extension}"
            );
        }
    }

    #[test]
    fn executables_without_a_plain_extension_are_programs_and_tarballs_are_archives() {
        let cases = [
            ("firmware", true, FileKind::Binary),
            ("trailing.", true, FileKind::Binary),
            ("odd.b-c", true, FileKind::Binary),
            ("Makefile", false, FileKind::Unknown),
            ("LICENSE", false, FileKind::Unknown),
            ("install.sh", true, FileKind::Script),
            ("backup.tar.gz", false, FileKind::Archive),
            ("backup.TGZ", false, FileKind::Archive),
            ("README.MD", false, FileKind::Markdown),
            (".bashrc", false, FileKind::Unknown),
        ];
        for (name, executable, kind) in cases {
            assert_eq!(kind_of(name, executable), kind, "{name}");
        }
    }

    #[test]
    fn the_tag_reads_the_extension_with_overrides() {
        let cases = [
            ("report.pdf", "PDF"),
            ("kitty.desktop", "LNK"),
            ("history.sqlite", "DB"),
            ("firmware", "BIN"),
            ("README.md", "MD"),
            ("Saira.ttf", "TTF"),
            ("backup.tar.gz", "GZ"),
            ("mystery.xyz", "XYZ"),
            ("notes.jsonl", "JSON"),
            ("Saira.woff2", "WOFF"),
            ("app.AppImage", "APP"),
        ];
        for (name, tag) in cases {
            assert_eq!(
                tag_text(name, kind_of(name, name == "firmware")),
                tag,
                "{name}"
            );
        }
    }

    #[test]
    fn a_missing_extension_falls_back_to_the_kind() {
        assert_eq!(tag_text("notes", FileKind::Text), "TXT");
        assert_eq!(tag_text("thing", FileKind::Unknown), "?");
    }

    #[test]
    fn every_kind_has_a_distinct_asset() {
        let mut stems: Vec<_> = FileKind::ALL.iter().map(|kind| kind.asset_stem()).collect();
        stems.sort_unstable();
        stems.dedup();
        assert_eq!(stems.len(), FileKind::ALL.len());
    }
}
