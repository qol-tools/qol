use std::fs;
use std::path::{Path, PathBuf};

const HEADER_SIZE: usize = 0x4C;
const LINK_CLSID: [u8; 16] = [
    0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];
const FLAGS_OFFSET: usize = 20;

const HAS_ID_LIST: u32 = 1 << 0;
const HAS_LINK_INFO: u32 = 1 << 1;
const HAS_NAME: u32 = 1 << 2;
const HAS_RELATIVE_PATH: u32 = 1 << 3;
const HAS_WORKING_DIR: u32 = 1 << 4;
const HAS_ARGUMENTS: u32 = 1 << 5;
const HAS_ICON_LOCATION: u32 = 1 << 6;
const IS_UNICODE: u32 = 1 << 7;
const FORCE_NO_LINK_INFO: u32 = 1 << 8;
const HAS_DARWIN_ID: u32 = 1 << 12;
const STRING_DATA: [u32; 5] = [
    HAS_NAME,
    HAS_RELATIVE_PATH,
    HAS_WORKING_DIR,
    HAS_ARGUMENTS,
    HAS_ICON_LOCATION,
];

const LOCAL_BASE_PATH_FLAG: u32 = 1;
const UNICODE_LINK_INFO_HEADER: u32 = 0x24;
const LOCAL_BASE_PATH_OFFSET: usize = 16;
const COMMON_PATH_SUFFIX_OFFSET: usize = 24;
const LOCAL_BASE_PATH_UNICODE_OFFSET: usize = 28;
const COMMON_PATH_SUFFIX_UNICODE_OFFSET: usize = 32;

const ENVIRONMENT_BLOCK: u32 = 0xA000_0001;
const ENVIRONMENT_BLOCK_SIZE: usize = 0x314;
const ENVIRONMENT_ANSI: std::ops::Range<usize> = 8..268;
const ENVIRONMENT_UNICODE: std::ops::Range<usize> = 268..788;

const ITEM_CLASS_MASK: u8 = 0x70;
const VOLUME_CLASS: u8 = 0x20;
const FILE_CLASS: u8 = 0x30;
const NETWORK_CLASS: u8 = 0x40;
const ROOT_EXTENSION_ITEM: u8 = 0x2E;
const DELEGATE_ITEM: u8 = 0x74;
const UNICODE_FILE_ITEM: u8 = 0x04;
const VOLUME_NAME_OFFSET: usize = 3;
const FILE_NAME_OFFSET: usize = 14;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdList {
    ShellFolder,
    Path(String),
    Unresolved,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShellLink {
    pub id_list: Option<IdList>,
    pub local_path: Option<String>,
    pub relative_path: Option<String>,
    pub icon_location: Option<String>,
    pub environment_target: Option<String>,
    pub advertised: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    Path(PathBuf),
    ShellFolder,
    Advertised,
    Unknown,
}

impl ShellLink {
    pub fn read(path: &Path) -> Option<Self> {
        Self::parse(&fs::read(path).ok()?)
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if u32_at(bytes, 0)? as usize != HEADER_SIZE || bytes.get(4..20)? != LINK_CLSID {
            return None;
        }
        let flags = u32_at(bytes, FLAGS_OFFSET)?;
        let has = |flag: u32| flags & flag != 0;
        let mut link = Self {
            advertised: has(HAS_DARWIN_ID),
            ..Self::default()
        };
        let mut offset = HEADER_SIZE;
        if has(HAS_ID_LIST) {
            let size = usize::from(u16_at(bytes, offset)?);
            link.id_list = id_list(bytes.get(offset + 2..offset + 2 + size)?);
            offset += 2 + size;
        }
        if has(HAS_LINK_INFO) {
            let size = u32_at(bytes, offset)? as usize;
            let info = bytes.get(offset..offset + size)?;
            if !has(FORCE_NO_LINK_INFO) {
                link.local_path = local_path(info);
            }
            offset += size;
        }
        for flag in STRING_DATA.into_iter().filter(|flag| has(*flag)) {
            let (value, next) = string_data(bytes, offset, has(IS_UNICODE))?;
            offset = next;
            match flag {
                HAS_RELATIVE_PATH => link.relative_path = non_empty(value),
                HAS_ICON_LOCATION => link.icon_location = non_empty(value),
                _ => {}
            }
        }
        link.environment_target = bytes.get(offset..).and_then(environment_target);
        Some(link)
    }

    pub fn target(&self, link: &Path, env: impl Fn(&str) -> Option<String>) -> LinkTarget {
        if self.advertised {
            return LinkTarget::Advertised;
        }
        let path = self
            .local_path
            .clone()
            .or_else(|| {
                self.environment_target
                    .as_deref()
                    .and_then(|raw| expand_environment(raw, &env))
            })
            .or_else(|| {
                self.relative_path
                    .as_deref()
                    .and_then(|relative| resolve_relative(link, relative))
            })
            .or_else(|| match &self.id_list {
                Some(IdList::Path(path)) => Some(path.clone()),
                _ => None,
            });
        match (path, &self.id_list) {
            (Some(path), _) => LinkTarget::Path(PathBuf::from(path)),
            (None, Some(IdList::ShellFolder)) => LinkTarget::ShellFolder,
            (None, _) => LinkTarget::Unknown,
        }
    }
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn ansi(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| char::from(*byte))
        .collect()
}

fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn id_list(mut items: &[u8]) -> Option<IdList> {
    let mut path: Option<String> = None;
    let mut names_file = false;
    let mut resolvable = true;
    loop {
        let size = usize::from(u16_at(items, 0)?);
        if size == 0 {
            break;
        }
        let item = items.get(..size).filter(|item| item.len() > 2)?;
        let kind = item[2];
        let class = kind & ITEM_CLASS_MASK;
        if class == VOLUME_CLASS && kind != ROOT_EXTENSION_ITEM {
            names_file = true;
            path = item
                .get(VOLUME_NAME_OFFSET..)
                .map(ansi)
                .map(|name| name.trim_end_matches('\\').to_string())
                .and_then(non_empty);
        } else if class == FILE_CLASS {
            names_file = true;
            let name = item.get(FILE_NAME_OFFSET..).map(|raw| {
                if kind & UNICODE_FILE_ITEM != 0 {
                    utf16(raw)
                } else {
                    ansi(raw)
                }
            });
            match (path.as_mut(), name.and_then(non_empty)) {
                (Some(path), Some(name)) => {
                    path.push('\\');
                    path.push_str(&name);
                }
                _ => resolvable = false,
            }
        } else if class == NETWORK_CLASS || kind == DELEGATE_ITEM {
            names_file = true;
            resolvable = false;
        }
        items = &items[size..];
    }
    Some(match (names_file, resolvable, path) {
        (false, _, _) => IdList::ShellFolder,
        (true, true, Some(path)) => IdList::Path(path),
        (true, _, _) => IdList::Unresolved,
    })
}

fn local_path(info: &[u8]) -> Option<String> {
    if u32_at(info, 8)? & LOCAL_BASE_PATH_FLAG == 0 {
        return None;
    }
    let unicode = u32_at(info, 4)? >= UNICODE_LINK_INFO_HEADER;
    let field = |ansi_at: usize, unicode_at: usize| -> Option<String> {
        let unicode_offset = unicode
            .then(|| u32_at(info, unicode_at))
            .flatten()
            .filter(|offset| *offset != 0);
        match unicode_offset {
            Some(offset) => info.get(offset as usize..).map(utf16),
            None => info.get(u32_at(info, ansi_at)? as usize..).map(ansi),
        }
    };
    let base = field(LOCAL_BASE_PATH_OFFSET, LOCAL_BASE_PATH_UNICODE_OFFSET)?;
    let suffix =
        field(COMMON_PATH_SUFFIX_OFFSET, COMMON_PATH_SUFFIX_UNICODE_OFFSET).unwrap_or_default();
    let joined = if suffix.is_empty() || base.ends_with('\\') {
        format!("{base}{suffix}")
    } else {
        format!(r"{base}\{suffix}")
    };
    non_empty(joined)
}

fn string_data(bytes: &[u8], offset: usize, unicode: bool) -> Option<(String, usize)> {
    let count = usize::from(u16_at(bytes, offset)?);
    let length = if unicode { count * 2 } else { count };
    let start = offset + 2;
    let raw = bytes.get(start..start + length)?;
    let value = if unicode { utf16(raw) } else { ansi(raw) };
    Some((value, start + length))
}

fn environment_target(mut extra: &[u8]) -> Option<String> {
    loop {
        let size = u32_at(extra, 0)? as usize;
        let block = extra.get(..size).filter(|block| block.len() >= 8)?;
        if u32_at(block, 4)? == ENVIRONMENT_BLOCK && size == ENVIRONMENT_BLOCK_SIZE {
            return non_empty(utf16(&block[ENVIRONMENT_UNICODE]))
                .or_else(|| non_empty(ansi(&block[ENVIRONMENT_ANSI])));
        }
        extra = &extra[size..];
    }
}

fn expand_environment(raw: &str, env: impl Fn(&str) -> Option<String>) -> Option<String> {
    let mut expanded = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        expanded.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('%')?;
        expanded.push_str(&env(&after[..end])?);
        rest = &after[end + 1..];
    }
    expanded.push_str(rest);
    Some(expanded)
}

fn resolve_relative(link: &Path, relative: &str) -> Option<String> {
    let link = link.to_string_lossy();
    let dir = &link[..link.rfind(['\\', '/'])?];
    let joined = format!(r"{dir}\{relative}");
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split(['\\', '/']) {
        match part {
            "" | "." => {}
            ".." if parts.len() > 1 => {
                parts.pop();
            }
            ".." => {}
            _ => parts.push(part),
        }
    }
    let prefix = if joined.starts_with(r"\\") { r"\\" } else { "" };
    non_empty(format!("{prefix}{}", parts.join("\\")))
}

#[cfg(test)]
mod tests;
