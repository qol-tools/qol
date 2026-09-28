use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

const RECENT_FILE: &str = "recently-used.xbel";

pub(super) fn recent_files(limit: usize) -> Vec<PathBuf> {
    let Some(data_home) = std::env::var_os("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
    else {
        return Vec::new();
    };
    let Ok(content) = std::fs::read_to_string(data_home.join(RECENT_FILE)) else {
        return Vec::new();
    };
    newest_first(&content)
        .into_iter()
        .filter(|path| path.is_file())
        .take(limit)
        .collect()
}

fn newest_first(content: &str) -> Vec<PathBuf> {
    let mut bookmarks: Vec<(String, PathBuf)> = content
        .split("<bookmark ")
        .skip(1)
        .filter_map(|chunk| {
            let head = format!(" {}", chunk.split('>').next()?);
            let path = file_path(&unescape(attribute(&head, "href")?))?;
            let when = ["modified", "visited", "added"]
                .into_iter()
                .find_map(|key| attribute(&head, key))
                .unwrap_or_default()
                .to_owned();
            Some((when, path))
        })
        .collect();
    bookmarks.sort_by(|left, right| right.0.cmp(&left.0));
    let mut seen = std::collections::HashSet::new();
    bookmarks
        .into_iter()
        .map(|(_, path)| path)
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

fn attribute<'a>(head: &'a str, key: &str) -> Option<&'a str> {
    let marker = format!(" {key}=\"");
    let start = head.find(&marker)? + marker.len();
    let end = head[start..].find('"')?;
    Some(&head[start..start + end])
}

fn unescape(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn file_path(uri: &str) -> Option<PathBuf> {
    let encoded = uri.strip_prefix("file://")?.as_bytes();
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut index = 0;
    while index < encoded.len() {
        let byte = encoded[index];
        let hex = encoded
            .get(index + 1..index + 3)
            .and_then(|pair| std::str::from_utf8(pair).ok())
            .and_then(|pair| u8::from_str_radix(pair, 16).ok());
        match (byte, hex) {
            (b'%', Some(decoded)) => {
                bytes.push(decoded);
                index += 3;
            }
            _ => {
                bytes.push(byte);
                index += 1;
            }
        }
    }
    bytes
        .starts_with(b"/")
        .then(|| PathBuf::from(OsString::from_vec(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmarks_sort_newest_first_and_decode_their_paths() {
        let xbel = r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0">
  <bookmark href="file:///home/qol/Documents/old.txt" added="2026-01-01T10:00:00Z" modified="2026-01-02T10:00:00Z" visited="2026-01-02T10:00:00Z">
  </bookmark>
  <bookmark href="file:///home/qol/My%20Notes/new&amp;draft.md" added="2026-09-01T10:00:00Z" modified="2026-09-27T09:00:00Z" visited="2026-09-27T09:00:00Z">
  </bookmark>
  <bookmark href="https://example.com/page" modified="2026-09-28T10:00:00Z">
  </bookmark>
  <bookmark href="file:///home/qol/Documents/old.txt" modified="2025-01-01T10:00:00Z">
  </bookmark>
</xbel>"#;
        assert_eq!(
            newest_first(xbel),
            vec![
                PathBuf::from("/home/qol/My Notes/new&draft.md"),
                PathBuf::from("/home/qol/Documents/old.txt"),
            ]
        );
    }

    #[test]
    fn file_uris_decode_percent_escapes_and_reject_other_schemes() {
        assert_eq!(
            file_path("file:///tmp/a%2Fb%20c"),
            Some(PathBuf::from("/tmp/a/b c"))
        );
        assert_eq!(
            file_path("file:///tmp/100%"),
            Some(PathBuf::from("/tmp/100%"))
        );
        assert_eq!(file_path("trash:///x"), None);
    }
}
