use std::fs::{self, File};
use std::io::BufReader;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use md5::{Digest, Md5};

use super::super::super::{Flavor, ThumbError};

const SHARED_FLAVORS: [&str; 4] = ["normal", "large", "x-large", "xx-large"];
const URI_SAFE: &[u8] = b"-._~/!$&'()*+,=:@";
const URI_KEY: &str = "Thumb::URI";
const MTIME_KEY: &str = "Thumb::MTime";

pub(super) struct Stamp {
    pub(super) uri: String,
    hash: String,
    mtime: u64,
}

impl Stamp {
    pub(super) fn of(path: &Path) -> Result<Self, ThumbError> {
        let unreadable = |error: std::io::Error| ThumbError::Unavailable(error.to_string());
        let meta = fs::metadata(path).map_err(unreadable)?;
        if !meta.is_file() {
            return Err(ThumbError::Unavailable("only files have previews".into()));
        }
        let mtime = meta
            .modified()
            .map_err(unreadable)?
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let uri = file_uri(path);
        let hash = format!("{:x}", Md5::digest(uri.as_bytes()));
        Ok(Self { uri, hash, mtime })
    }

    fn own_name(&self) -> String {
        format!("{}-{}.png", self.hash, self.mtime)
    }
}

pub(super) fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || URI_SAFE.contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

pub(super) fn shared(stamp: &Stamp, flavor: Flavor) -> Option<PathBuf> {
    let root = shared_root()?;
    let first = SHARED_FLAVORS
        .iter()
        .position(|name| *name == flavor.name())?;
    SHARED_FLAVORS[first..]
        .iter()
        .map(|name| root.join(name).join(format!("{}.png", stamp.hash)))
        .find(|candidate| describes(candidate, stamp))
}

pub(super) fn own(stamp: &Stamp, flavor: Flavor) -> Result<PathBuf, ThumbError> {
    let root = qol_config::cache_dir()
        .ok_or_else(|| ThumbError::Unavailable("the cache folder is not known".into()))?;
    Ok(own_in(&root, stamp, flavor))
}

fn own_in(root: &Path, stamp: &Stamp, flavor: Flavor) -> PathBuf {
    root.join("explorer")
        .join("thumbnails")
        .join(flavor.name())
        .join(stamp.own_name())
}

pub(super) fn forget_older(stamp: &Stamp, kept: &Path) {
    let Some(folder) = kept.parent() else {
        return;
    };
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    let prefix = format!("{}-", stamp.hash);
    for entry in entries.flatten() {
        let path = entry.path();
        let older = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(&prefix));
        if older && path != kept {
            let _ = fs::remove_file(path);
        }
    }
}

fn shared_root() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| dirs::home_dir().map(|home| home.join(".cache")))?;
    Some(cache.join("thumbnails"))
}

fn describes(candidate: &Path, stamp: &Stamp) -> bool {
    let Ok(file) = File::open(candidate) else {
        return false;
    };
    let Ok(reader) = png::Decoder::new(BufReader::new(file)).read_info() else {
        return false;
    };
    let info = reader.info();
    let texts = info
        .uncompressed_latin1_text
        .iter()
        .map(|chunk| (chunk.keyword.as_str(), chunk.text.clone()))
        .chain(
            info.utf8_text
                .iter()
                .filter_map(|chunk| Some((chunk.keyword.as_str(), chunk.get_text().ok()?))),
        )
        .collect::<Vec<_>>();
    let says = |key: &str, wanted: &str| {
        texts
            .iter()
            .any(|(found, text)| *found == key && text == wanted)
    };
    says(URI_KEY, &stamp.uri) && says(MTIME_KEY, &stamp.mtime.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_escape_like_glib_so_shared_thumbnails_match() {
        let cases = [
            (
                "/home/me/Videos/clip+0100.webm",
                "file:///home/me/Videos/clip+0100.webm",
            ),
            (
                "/tmp/a;b?c#d e%f!$&'()*+,=:@~[]",
                "file:///tmp/a%3Bb%3Fc%23d%20e%25f!$&'()*+,=:@~%5B%5D",
            ),
            ("/tmp/é", "file:///tmp/%C3%A9"),
        ];
        for (path, uri) in cases {
            assert_eq!(file_uri(Path::new(path)), uri, "{path}");
        }
    }

    #[test]
    fn own_thumbnails_are_keyed_by_uri_hash_and_mtime() {
        let stamp = Stamp {
            uri: "file:///a.png".into(),
            hash: format!("{:x}", Md5::digest(b"file:///a.png")),
            mtime: 42,
        };
        let path = own_in(Path::new("/cache"), &stamp, Flavor::Large);
        assert_eq!(
            path,
            Path::new("/cache/explorer/thumbnails/large").join(format!("{}-42.png", stamp.hash))
        );
    }

    fn write_png(path: &Path, texts: &[(&str, &str)]) {
        let file = File::create(path).unwrap();
        let mut encoder = png::Encoder::new(file, 1, 1);
        encoder.set_color(png::ColorType::Grayscale);
        for (key, text) in texts {
            encoder
                .add_text_chunk(key.to_string(), text.to_string())
                .unwrap();
        }
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[0])
            .unwrap();
    }

    #[test]
    fn a_shared_thumbnail_counts_only_when_its_uri_and_mtime_match() {
        let temp = tempfile::tempdir().unwrap();
        let stamp = Stamp {
            uri: "file:///v.webm".into(),
            hash: "h".into(),
            mtime: 7,
        };
        let cases: [(&[(&str, &str)], bool); 4] = [
            (&[(URI_KEY, "file:///v.webm"), (MTIME_KEY, "7")], true),
            (&[(URI_KEY, "file:///v.webm"), (MTIME_KEY, "8")], false),
            (&[(URI_KEY, "file:///w.webm"), (MTIME_KEY, "7")], false),
            (&[(URI_KEY, "file:///v.webm")], false),
        ];
        for (index, (texts, expected)) in cases.into_iter().enumerate() {
            let path = temp.path().join(format!("{index}.png"));
            write_png(&path, texts);
            assert_eq!(describes(&path, &stamp), expected, "case {index}");
        }
        assert!(!describes(&temp.path().join("missing.png"), &stamp));
    }
}
