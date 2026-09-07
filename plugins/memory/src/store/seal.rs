use std::io::Read;
use std::path::Path;

pub const SEAL_SCHEMA: &str = "qol-memory-seal-v1";

pub fn try_sealed_text(root: &Path, raw: &[u8]) -> Option<String> {
    let marker_path = root.join("units.seal.json");
    let blob_path = root.join("units.seal.gz");
    if !marker_path.exists() || !blob_path.exists() {
        return None;
    }
    let marker_text = std::fs::read_to_string(&marker_path).ok()?;
    let marker: serde_json::Value = serde_json::from_str(&marker_text).ok()?;
    if marker.get("schema")?.as_str()? != SEAL_SCHEMA {
        return None;
    }
    let prefix_len = marker.get("prefix_len")?.as_i64()?;
    if !(0..=raw.len() as i64).contains(&prefix_len) {
        return None;
    }
    let blob_len = marker.get("blob_len")?.as_i64()?;
    let blob_size = std::fs::metadata(&blob_path).ok()?.len();
    if blob_len < 0 || blob_size != blob_len as u64 {
        return None;
    }
    let mut decoded = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&blob_path).ok()?)
        .read_to_end(&mut decoded)
        .ok()?;
    if decoded.len() != prefix_len as usize {
        return None;
    }
    decoded.extend_from_slice(&raw[prefix_len as usize..]);
    Some(String::from_utf8_lossy(&decoded).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "qol-memory-seal-{}-{}-{}",
                tag,
                std::process::id(),
                nanos
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn write_marker(path: &Path, prefix_len: usize, blob_len: u64) {
        let marker = serde_json::json!({
            "schema": SEAL_SCHEMA,
            "prefix_len": prefix_len,
            "blob": "units.seal.gz",
            "blob_len": blob_len,
            "sealed_units": 2,
            "created": "2026-08-27T08:39:05.554Z"
        });
        let pretty = serde_json::to_string_pretty(&marker).unwrap();
        std::fs::write(path, format!("{}\n", pretty)).unwrap();
    }

    #[test]
    fn sealed_round_trip_recovers_prefix_plus_tail() {
        let dir = TempDir::new("roundtrip");
        let prefix = b"{\"key\":\"a\",\"text\":\"h\\u00e9llo w\\u00f6rld\"}\n{\"key\":\"b\"}\n";
        let suffix = "the r\u{00e9}sum\u{00e9} tail\n".as_bytes();
        let mut raw = prefix.to_vec();
        raw.extend_from_slice(suffix);
        let blob = gzip(prefix);
        std::fs::write(dir.0.join("units.seal.gz"), &blob).unwrap();
        write_marker(
            &dir.0.join("units.seal.json"),
            prefix.len(),
            blob.len() as u64,
        );
        assert_eq!(
            try_sealed_text(dir.0.as_path(), &raw).unwrap(),
            String::from_utf8_lossy(&raw)
        );
    }

    #[test]
    fn tampered_blob_len_returns_none() {
        let dir = TempDir::new("tampered");
        let prefix = b"{\"key\":\"a\"}\n";
        let mut raw = prefix.to_vec();
        raw.extend_from_slice(b"tail\n");
        let blob = gzip(prefix);
        std::fs::write(dir.0.join("units.seal.gz"), &blob).unwrap();
        write_marker(
            &dir.0.join("units.seal.json"),
            prefix.len(),
            blob.len() as u64 + 7,
        );
        assert!(try_sealed_text(dir.0.as_path(), &raw).is_none());
    }

    #[test]
    fn missing_marker_bad_schema_and_short_decode_return_none() {
        let dir = TempDir::new("cases");
        let raw = b"{\"key\":\"a\"}\n".to_vec();
        let blob = gzip(b"xyz");
        std::fs::write(dir.0.join("units.seal.gz"), &blob).unwrap();

        assert!(try_sealed_text(dir.0.as_path(), &raw).is_none());

        std::fs::write(
            dir.0.join("units.seal.json"),
            "{\"schema\":\"other-v9\",\"prefix_len\":3,\"blob\":\"units.seal.gz\",\"blob_len\":40}\n",
        )
        .unwrap();
        assert!(try_sealed_text(dir.0.as_path(), &raw).is_none());

        std::fs::write(
            dir.0.join("units.seal.json"),
            format!(
                "{{\"schema\":\"{}\",\"prefix_len\":10,\"blob\":\"units.seal.gz\",\"blob_len\":{}}}\n",
                SEAL_SCHEMA,
                blob.len()
            ),
        )
        .unwrap();
        assert!(try_sealed_text(dir.0.as_path(), &raw).is_none());
    }
}
