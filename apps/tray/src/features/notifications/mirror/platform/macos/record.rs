use serde::Deserialize;

use super::super::super::Mirrored;

#[derive(Deserialize)]
struct Record {
    app: String,
    req: Request,
}

#[derive(Deserialize)]
struct Request {
    #[serde(default)]
    titl: Option<String>,
    #[serde(default)]
    subt: Option<String>,
    #[serde(default)]
    body: Option<String>,
}

pub(super) fn decode(data: &[u8]) -> Option<Mirrored> {
    let record: Record = plist::from_bytes(data).ok()?;
    let mirrored = Mirrored {
        app_id: record.app,
        title: record.req.titl.unwrap_or_default(),
        subtitle: record.req.subt.unwrap_or_default(),
        body: record.req.body.unwrap_or_default(),
    };
    let empty =
        mirrored.title.is_empty() && mirrored.subtitle.is_empty() && mirrored.body.is_empty();
    (!empty).then_some(mirrored)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(value: plist::Value) -> Vec<u8> {
        let mut bytes = Vec::new();
        plist::to_writer_binary(&mut bytes, &value).unwrap();
        bytes
    }

    fn record(app: &str, request: &[(&str, &str)]) -> Vec<u8> {
        let request = request
            .iter()
            .map(|(key, value)| (key.to_string(), plist::Value::from(*value)))
            .collect::<plist::Dictionary>();
        let mut root = plist::Dictionary::new();
        root.insert("app".to_string(), plist::Value::from(app));
        root.insert("date".to_string(), plist::Value::Real(813_503_938.5));
        root.insert("req".to_string(), plist::Value::Dictionary(request));
        encode(plist::Value::Dictionary(root))
    }

    #[test]
    fn decodes_title_subtitle_and_body() {
        let bytes = record(
            "com.microsoft.teams2",
            &[
                ("titl", "Ada"),
                ("subt", "#general"),
                ("body", "lunch?"),
                ("cate", "actions:"),
            ],
        );
        assert_eq!(
            decode(&bytes),
            Some(Mirrored {
                app_id: "com.microsoft.teams2".to_string(),
                title: "Ada".to_string(),
                subtitle: "#general".to_string(),
                body: "lunch?".to_string(),
            })
        );
    }

    #[test]
    fn missing_fields_default_to_empty() {
        let bytes = record("com.apple.ScriptEditor2", &[("body", "done")]);
        let decoded = decode(&bytes).unwrap();
        assert_eq!(
            (
                decoded.title.as_str(),
                decoded.subtitle.as_str(),
                decoded.body.as_str()
            ),
            ("", "", "done")
        );
    }

    #[test]
    fn rejects_empty_and_malformed_records() {
        assert_eq!(decode(&record("com.example", &[])), None);
        assert_eq!(decode(b"not a plist"), None);
        let mut no_request = plist::Dictionary::new();
        no_request.insert("app".to_string(), plist::Value::from("com.example"));
        assert_eq!(decode(&encode(plist::Value::Dictionary(no_request))), None);
    }
}
