use super::*;

const LINK: &str = r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Foo\Foo.lnk";
const MY_COMPUTER: [u8; 16] = [
    0xE0, 0x4F, 0xD0, 0x20, 0xEA, 0x3A, 0x69, 0x10, 0xA2, 0xD8, 0x08, 0x00, 0x2B, 0x30, 0x30, 0x9D,
];

#[derive(Default)]
struct Fixture {
    flags: u32,
    id_list: Option<Vec<u8>>,
    link_info: Option<Vec<u8>>,
    strings: Vec<(u32, String)>,
    extra: Vec<u8>,
}

impl Fixture {
    fn id_list(mut self, items: &[Vec<u8>]) -> Self {
        let mut list: Vec<u8> = items.concat();
        list.extend([0, 0]);
        self.flags |= HAS_ID_LIST;
        self.id_list = Some(list);
        self
    }

    fn link_info(mut self, info: Vec<u8>) -> Self {
        self.flags |= HAS_LINK_INFO;
        self.link_info = Some(info);
        self
    }

    fn string(mut self, flag: u32, value: &str) -> Self {
        self.flags |= flag;
        self.strings.push((flag, value.to_string()));
        self
    }

    fn flag(mut self, flag: u32) -> Self {
        self.flags |= flag;
        self
    }

    fn environment(mut self, target: &str) -> Self {
        let mut block = vec![0u8; ENVIRONMENT_BLOCK_SIZE];
        block[..4].copy_from_slice(&(ENVIRONMENT_BLOCK_SIZE as u32).to_le_bytes());
        block[4..8].copy_from_slice(&ENVIRONMENT_BLOCK.to_le_bytes());
        let unicode = wide(target);
        block[ENVIRONMENT_UNICODE.start..ENVIRONMENT_UNICODE.start + unicode.len()]
            .copy_from_slice(&unicode);
        self.extra = block;
        self
    }

    fn bytes(&self) -> Vec<u8> {
        let mut bytes = vec![0u8; HEADER_SIZE];
        bytes[..4].copy_from_slice(&(HEADER_SIZE as u32).to_le_bytes());
        bytes[4..20].copy_from_slice(&LINK_CLSID);
        bytes[FLAGS_OFFSET..FLAGS_OFFSET + 4].copy_from_slice(&self.flags.to_le_bytes());
        if let Some(list) = &self.id_list {
            bytes.extend((list.len() as u16).to_le_bytes());
            bytes.extend(list);
        }
        if let Some(info) = &self.link_info {
            bytes.extend(info);
        }
        let unicode = self.flags & IS_UNICODE != 0;
        let mut strings = self.strings.clone();
        strings.sort_by_key(|(flag, _)| *flag);
        for (_, value) in strings {
            bytes.extend((value.encode_utf16().count() as u16).to_le_bytes());
            if unicode {
                bytes.extend(value.encode_utf16().flat_map(u16::to_le_bytes));
            } else {
                bytes.extend(value.bytes());
            }
        }
        bytes.extend(&self.extra);
        bytes.extend([0, 0, 0, 0]);
        bytes
    }
}

fn wide(value: &str) -> Vec<u8> {
    value.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn z(mut bytes: Vec<u8>, terminator: usize) -> Vec<u8> {
    bytes.extend(vec![0; terminator]);
    bytes
}

fn ansi_info(base: &str, suffix: &str) -> Vec<u8> {
    let header = 0x1Cu32;
    let volume = vec![0x10, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0x10, 0, 0, 0];
    let base_at = header as usize + volume.len();
    let base = z(base.as_bytes().to_vec(), 1);
    let suffix_at = base_at + base.len();
    let suffix = z(suffix.as_bytes().to_vec(), 1);
    let size = suffix_at + suffix.len();
    let mut info = Vec::new();
    for value in [
        size as u32,
        header,
        LOCAL_BASE_PATH_FLAG,
        header,
        base_at as u32,
        0,
        suffix_at as u32,
    ] {
        info.extend(value.to_le_bytes());
    }
    info.extend(volume);
    info.extend(base);
    info.extend(suffix);
    info
}

fn unicode_info(base: &str, suffix: &str) -> Vec<u8> {
    let header = UNICODE_LINK_INFO_HEADER;
    let ansi_base_at = header as usize;
    let ansi = [0u8, 0u8];
    let base_at = ansi_base_at + ansi.len();
    let base = z(wide(base), 2);
    let suffix_at = base_at + base.len();
    let suffix = z(wide(suffix), 2);
    let size = suffix_at + suffix.len();
    let mut info = Vec::new();
    for value in [
        size as u32,
        header,
        LOCAL_BASE_PATH_FLAG,
        0,
        ansi_base_at as u32,
        0,
        ansi_base_at as u32 + 1,
        base_at as u32,
        suffix_at as u32,
    ] {
        info.extend(value.to_le_bytes());
    }
    info.extend(ansi);
    info.extend(base);
    info.extend(suffix);
    info
}

fn network_info() -> Vec<u8> {
    let mut info = Vec::new();
    for value in [0x1Cu32, 0x1C, 2, 0, 0, 0x1C, 0] {
        info.extend(value.to_le_bytes());
    }
    info
}

fn root_item(clsid: [u8; 16]) -> Vec<u8> {
    let mut item = vec![0x14, 0x00, 0x1F, 0x50];
    item.extend(clsid);
    item
}

fn volume_item(drive: &str) -> Vec<u8> {
    let mut item = vec![0, 0, 0x2F];
    item.extend(z(drive.as_bytes().to_vec(), 1));
    item.resize(0x19, 0);
    item[0] = item.len() as u8;
    item
}

fn file_item(kind: u8, name: &str) -> Vec<u8> {
    let mut item = vec![0, 0, kind, 0];
    item.extend([0u8; FILE_NAME_OFFSET - 4]);
    if kind & UNICODE_FILE_ITEM != 0 {
        item.extend(z(wide(name), 2));
    } else {
        item.extend(z(name.as_bytes().to_vec(), 1));
    }
    item[0] = item.len() as u8;
    item
}

fn env(name: &str) -> Option<String> {
    match name.to_ascii_lowercase().as_str() {
        "windir" => Some(r"C:\Windows".into()),
        "programfiles" => Some(r"C:\Program Files".into()),
        _ => None,
    }
}

fn path(value: &str) -> LinkTarget {
    LinkTarget::Path(PathBuf::from(value))
}

#[test]
fn targets_resolve_from_each_source_in_priority_order() {
    let cases = [
        (
            "ansi local base path",
            Fixture::default().link_info(ansi_info(r"C:\Program Files\Foo\foo.exe", "")),
            path(r"C:\Program Files\Foo\foo.exe"),
        ),
        (
            "local base path plus common suffix",
            Fixture::default().link_info(ansi_info(r"C:\Program Files", r"Foo\foo.exe")),
            path(r"C:\Program Files\Foo\foo.exe"),
        ),
        (
            "unicode local base path",
            Fixture::default().link_info(unicode_info(r"C:\Tools\Zürich\z.exe", "")),
            path(r"C:\Tools\Zürich\z.exe"),
        ),
        (
            "link info beats the relative path",
            Fixture::default()
                .link_info(ansi_info(r"D:\Apps\Foo\foo.exe", ""))
                .string(HAS_RELATIVE_PATH, r"..\..\Bar\bar.exe"),
            path(r"D:\Apps\Foo\foo.exe"),
        ),
        (
            "forced off link info is ignored",
            Fixture::default()
                .link_info(ansi_info(r"D:\Apps\Foo\foo.exe", ""))
                .flag(FORCE_NO_LINK_INFO),
            LinkTarget::Unknown,
        ),
        (
            "network-only link info has no local path",
            Fixture::default().link_info(network_info()),
            LinkTarget::Unknown,
        ),
        (
            "environment block is expanded",
            Fixture::default()
                .flag(FORCE_NO_LINK_INFO)
                .environment(r"%windir%\system32\compmgmt.msc"),
            path(r"C:\Windows\system32\compmgmt.msc"),
        ),
        (
            "unknown environment variable falls through",
            Fixture::default().environment(r"%NOPE%\foo.exe"),
            LinkTarget::Unknown,
        ),
        (
            "relative path resolves against the shortcut folder",
            Fixture::default().string(
                HAS_RELATIVE_PATH,
                r"..\..\..\..\..\..\..\Program Files\Foo\foo.exe",
            ),
            path(r"C:\Program Files\Foo\foo.exe"),
        ),
        (
            "unicode relative path",
            Fixture::default()
                .flag(IS_UNICODE)
                .string(HAS_NAME, "Foo app")
                .string(HAS_RELATIVE_PATH, r".\bin\foo.exe"),
            path(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Foo\bin\foo.exe"),
        ),
        (
            "shell folder id list",
            Fixture::default().id_list(&[root_item(MY_COMPUTER)]),
            LinkTarget::ShellFolder,
        ),
        (
            "file system id list builds a path",
            Fixture::default().id_list(&[
                root_item(MY_COMPUTER),
                volume_item(r"C:\"),
                file_item(0x31, "Windows"),
                file_item(0x36, "explorer.exe"),
            ]),
            path(r"C:\Windows\explorer.exe"),
        ),
        (
            "file items without a volume are unresolved",
            Fixture::default().id_list(&[root_item(MY_COMPUTER), file_item(0x32, "foo.exe")]),
            LinkTarget::Unknown,
        ),
        (
            "link info beats the id list",
            Fixture::default()
                .id_list(&[root_item(MY_COMPUTER)])
                .link_info(ansi_info(r"C:\Program Files\Foo\foo.exe", "")),
            path(r"C:\Program Files\Foo\foo.exe"),
        ),
        (
            "advertised installer shortcut",
            Fixture::default()
                .flag(HAS_DARWIN_ID)
                .link_info(ansi_info(r"C:\Windows\Installer\{1}\icon.exe", "")),
            LinkTarget::Advertised,
        ),
        ("empty link", Fixture::default(), LinkTarget::Unknown),
    ];
    for (label, fixture, expected) in cases {
        let link = ShellLink::parse(&fixture.bytes()).unwrap_or_else(|| panic!("{label}"));
        assert_eq!(link.target(Path::new(LINK), env), expected, "{label}");
    }
}

#[test]
fn string_data_fields_are_read_in_order() {
    let cases = [false, true];
    for unicode in cases {
        let fixture = Fixture::default()
            .string(HAS_NAME, "Foo")
            .string(HAS_RELATIVE_PATH, r"..\Foo\foo.exe")
            .string(HAS_WORKING_DIR, r"C:\Foo")
            .string(HAS_ARGUMENTS, "--fast")
            .string(HAS_ICON_LOCATION, r"%ProgramFiles%\Foo\foo.ico");
        let fixture = if unicode {
            fixture.flag(IS_UNICODE)
        } else {
            fixture
        };
        let link = ShellLink::parse(&fixture.bytes()).unwrap();
        assert_eq!(
            link.relative_path.as_deref(),
            Some(r"..\Foo\foo.exe"),
            "{unicode}"
        );
        assert_eq!(
            link.icon_location.as_deref(),
            Some(r"%ProgramFiles%\Foo\foo.ico"),
            "{unicode}"
        );
    }
}

#[test]
fn malformed_links_are_rejected() {
    let good = Fixture::default()
        .link_info(ansi_info(r"C:\Foo\foo.exe", ""))
        .bytes();
    let mut wrong_clsid = good.clone();
    wrong_clsid[4] ^= 0xFF;
    let mut wrong_size = good.clone();
    wrong_size[0] = 0x4D;
    let truncated_link_info = good[..HEADER_SIZE + 8].to_vec();
    let truncated_string = {
        let mut bytes = Fixture::default()
            .string(HAS_RELATIVE_PATH, "foo.exe")
            .bytes();
        bytes.truncate(HEADER_SIZE + 4);
        bytes
    };
    let cases = [
        ("empty", Vec::new()),
        ("header only part", good[..40].to_vec()),
        ("wrong clsid", wrong_clsid),
        ("wrong header size", wrong_size),
        ("truncated link info", truncated_link_info),
        ("truncated string", truncated_string),
    ];
    for (label, bytes) in cases {
        assert_eq!(ShellLink::parse(&bytes), None, "{label}");
    }
}

#[test]
fn read_parses_a_file_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Foo.lnk");
    fs::write(
        &file,
        Fixture::default()
            .link_info(ansi_info(r"C:\Foo\foo.exe", ""))
            .bytes(),
    )
    .unwrap();
    assert_eq!(
        ShellLink::read(&file).map(|link| link.local_path),
        Some(Some(r"C:\Foo\foo.exe".to_string()))
    );
    assert_eq!(ShellLink::read(&dir.path().join("missing.lnk")), None);
}
