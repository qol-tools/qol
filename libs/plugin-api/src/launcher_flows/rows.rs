use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct FlowRows {
    #[serde(default)]
    pub rows: Vec<FlowRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Badge>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct FlowRow {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_image: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mono: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tag>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<Preview>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swatch: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub masked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<Badge>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Tag {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Kind,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Preview {
    pub text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mono: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Badge {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
}

impl FlowRow {
    /// The local image the thumb names: an absolute path, bare or as a `file://` URL.
    pub fn thumb_path(&self) -> Option<PathBuf> {
        let thumb = self.thumb.as_deref()?;
        let path = thumb.strip_prefix("file://").unwrap_or(thumb);
        let bytes = path.as_bytes();
        let drive = |at: usize| {
            bytes.len() > at + 2
                && bytes[at].is_ascii_alphabetic()
                && bytes[at + 1] == b':'
                && bytes[at + 2] == b'/'
        };
        if bytes.first() == Some(&b'/') && drive(1) {
            return Some(PathBuf::from(&path[1..]));
        }
        (bytes.first() == Some(&b'/') || drive(0)).then(|| PathBuf::from(path))
    }
}

/// A local image as a thumb URL, written with forward slashes on every OS.
pub fn thumb_url(path: &Path) -> String {
    let normalized = path.display().to_string().replace('\\', "/");
    format!("file:///{}", normalized.trim_start_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn thumbs_name_absolute_local_paths_only() {
        let cases = [
            ("file:///home/me/a.png", Some("/home/me/a.png")),
            ("/a.png", Some("/a.png")),
            ("file:///C:/Users/me/a.png", Some("C:/Users/me/a.png")),
            ("C:/Users/me/a.png", Some("C:/Users/me/a.png")),
            ("relative.png", None),
            ("https://x/y.png", None),
            ("file://relative.png", None),
        ];
        for (thumb, expected) in cases {
            let row = FlowRow {
                thumb: Some(thumb.to_string()),
                ..FlowRow::default()
            };
            assert_eq!(row.thumb_path(), expected.map(PathBuf::from), "{thumb}");
        }
    }

    #[test]
    fn a_thumb_url_reads_back_as_its_path() {
        let cases = [
            ("/data/blobs/c_1.png", "file:///data/blobs/c_1.png"),
            (
                r"C:\Users\me\blobs\c_1.png",
                "file:///C:/Users/me/blobs/c_1.png",
            ),
        ];
        for (path, url) in cases {
            assert_eq!(thumb_url(Path::new(path)), url, "{path}");
            let row = FlowRow {
                thumb: Some(url.to_string()),
                ..FlowRow::default()
            };
            assert!(row.thumb_path().is_some(), "{url}");
        }
    }

    #[test]
    fn memory_rows_sent_today_still_parse() {
        let payload = json!({
            "verdict": "answered",
            "confidence": "high",
            "rows": [
                {
                    "title": "The clipboard ring survives tray restarts",
                    "subtitle": "capture 2026-08-02",
                    "copy": "The clipboard ring survives tray restarts",
                    "key": "u-1",
                    "kind": "answer",
                    "lead": "Buffers survive restarts",
                    "host": "pc-alpha",
                    "sources": 3,
                    "nearby": true,
                    "trail": [
                        { "at": "2026-08-02", "tag": "true now", "text": "newest fact" },
                        { "at": "2026-07-30", "tag": "superseded", "text": "older", "struck": true }
                    ],
                    "detail": [{ "label": "verdict", "value": "kept" }]
                },
                {
                    "title": "nearby memory",
                    "copy": "nearby memory",
                    "key": "u-2",
                    "kind": "decision"
                }
            ],
            "verification": { "status": "pending" },
            "outcome": "supported",
            "reason_code": null
        });

        let parsed: FlowRows = serde_json::from_value(payload).unwrap();

        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.status, None);
        let answer = &parsed.rows[0];
        assert_eq!(answer.title, "The clipboard ring survives tray restarts");
        assert_eq!(answer.subtitle.as_deref(), Some("capture 2026-08-02"));
        assert_eq!(
            answer.copy.as_deref(),
            Some("The clipboard ring survives tray restarts")
        );
        assert_eq!(answer.kind.as_deref(), Some("answer"));
        assert!(answer.tags.is_empty());
        assert!(answer.facts.is_empty());
        assert!(!answer.mono);
        assert_eq!(parsed.rows[1].kind.as_deref(), Some("decision"));
    }

    #[test]
    fn a_minimal_memory_row_parses_to_defaults() {
        let row = json!({
            "title": "t",
            "copy": "c",
            "key": "k",
            "kind": "note"
        });

        let parsed: FlowRow = serde_json::from_value(row).unwrap();

        assert_eq!(
            parsed,
            FlowRow {
                title: "t".to_string(),
                kind: Some("note".to_string()),
                copy: Some("c".to_string()),
                ..FlowRow::default()
            }
        );
    }

    #[test]
    fn a_full_v1_row_parses_every_field() {
        let payload = json!({
            "rows": [{
                "id": "c_01",
                "kind": "link",
                "title": "github.com/qol-tools/qol-monorepo/pull/147",
                "subtitle": "Link · Firefox · 2 min",
                "copy": "https://github.com/qol-tools/qol-monorepo/pull/147",
                "copy_image": "/data/blobs/c_01.png",
                "mono": true,
                "group": "Now",
                "tags": [{ "text": "Link", "tone": "kind" }, { "text": "Firefox" }],
                "preview": { "text": "fix(monitor): save display mode changes", "mono": false },
                "facts": [["Copied from", "Firefox, 2 min"], ["Computer", "This computer"]],
                "thumb": "file:///thumbs/c_01.webp",
                "swatch": "#d9822b",
                "masked": false,
                "badge": { "text": "clears in 22 s", "tone": "warning" }
            }],
            "status": { "text": "8 clips", "tone": "info" }
        });

        let parsed: FlowRows = serde_json::from_value(payload).unwrap();

        let row = &parsed.rows[0];
        assert_eq!(row.id.as_deref(), Some("c_01"));
        assert!(row.mono);
        assert_eq!(row.group.as_deref(), Some("Now"));
        assert_eq!(
            row.tags,
            vec![
                Tag {
                    text: "Link".to_string(),
                    tone: Some(Tone::Kind)
                },
                Tag {
                    text: "Firefox".to_string(),
                    tone: None
                }
            ]
        );
        assert_eq!(
            row.preview,
            Some(Preview {
                text: "fix(monitor): save display mode changes".to_string(),
                mono: false
            })
        );
        assert_eq!(
            row.facts[0],
            ("Copied from".to_string(), "Firefox, 2 min".to_string())
        );
        assert_eq!(row.copy_image, Some(PathBuf::from("/data/blobs/c_01.png")));
        assert_eq!(row.swatch.as_deref(), Some("#d9822b"));
        assert_eq!(
            row.badge,
            Some(Badge {
                text: "clears in 22 s".to_string(),
                tone: Some(Tone::Warning)
            })
        );
        assert_eq!(
            parsed.status,
            Some(Badge {
                text: "8 clips".to_string(),
                tone: Some(Tone::Info)
            })
        );
    }

    #[test]
    fn a_row_without_a_title_is_rejected() {
        assert!(serde_json::from_value::<FlowRow>(json!({ "copy": "c" })).is_err());
    }

    #[test]
    fn rows_round_trip_through_json() {
        let rows = FlowRows {
            rows: vec![FlowRow {
                title: "t".to_string(),
                mono: true,
                tags: vec![Tag {
                    text: "Pinned".to_string(),
                    tone: None,
                }],
                ..FlowRow::default()
            }],
            status: Some(Badge {
                text: "Capture is paused".to_string(),
                tone: Some(Tone::Error),
            }),
        };

        let json = serde_json::to_value(&rows).unwrap();

        assert_eq!(serde_json::from_value::<FlowRows>(json).unwrap(), rows);
    }
}
