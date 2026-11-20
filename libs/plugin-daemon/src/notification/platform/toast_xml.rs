use qol_runtime::Notification;

const MAX_TAG_LEN: usize = 64;
const DEFAULT_GROUP: &str = "qol";

pub(super) fn toast_xml(notification: &Notification) -> String {
    format!(
        "<toast><visual><binding template=\"ToastGeneric\">{}<text>{}</text><text>{}</text></binding></visual></toast>",
        image_element(notification.meta.image.as_deref()),
        xml_escape(&notification.title),
        xml_escape(&notification.body)
    )
}

pub(super) fn toast_tag(notification: &Notification) -> Option<&str> {
    notification
        .meta
        .key
        .as_deref()
        .filter(|key| !key.is_empty() && key.chars().count() <= MAX_TAG_LEN)
}

pub(super) fn toast_group(plugin_id: Option<String>) -> String {
    plugin_id
        .filter(|id| !id.is_empty() && id.chars().count() <= MAX_TAG_LEN)
        .unwrap_or_else(|| DEFAULT_GROUP.to_string())
}

fn image_element(image: Option<&str>) -> String {
    image.map_or_else(String::new, |path| {
        format!(
            "<image placement=\"appLogoOverride\" src=\"{}\"/>",
            xml_escape(&file_uri(path))
        )
    })
}

fn file_uri(path: &str) -> String {
    let encoded = percent_encode(&path.replace('\\', "/"));
    match encoded.strip_prefix("//") {
        Some(share) => format!("file://{share}"),
        None => format!("file:///{}", encoded.trim_start_matches('/')),
    }
}

fn percent_encode(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                encoded.push(char::from(byte))
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

fn xml_escape(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            '\t' | '\n' | '\r' => escaped.push(character),
            '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => {}
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xml(title: &str, body: &str) -> String {
        toast_xml(&Notification::new(title, body))
    }

    #[test]
    fn toast_xml_escapes_markup_in_title_and_body() {
        let cases = [
            ("plain", "body", "<text>plain</text><text>body</text>"),
            (
                "a & b",
                "<tag> \"q\" 'a'",
                "<text>a &amp; b</text><text>&lt;tag&gt; &quot;q&quot; &apos;a&apos;</text>",
            ),
        ];
        for (title, body, fragment) in cases {
            assert!(xml(title, body).contains(fragment), "{title} {body}");
        }
    }

    #[test]
    fn toast_xml_drops_the_characters_xml_forbids() {
        let cases = [
            (
                "Pad\u{1}",
                "tab\tline\nreturn\r",
                "<text>Pad</text><text>tab\tline\nreturn\r</text>",
            ),
            (
                "\u{0}\u{b}\u{c}\u{1f}",
                "a\u{fffe}b\u{ffff}c\u{7f}",
                "<text></text><text>abc\u{7f}</text>",
            ),
        ];
        for (title, body, fragment) in cases {
            let xml = xml(title, body);
            assert!(xml.contains(fragment), "{xml:?}");
        }
    }

    #[test]
    fn toast_xml_shows_the_image_as_a_file_uri_logo() {
        let cases = [
            (
                r"C:\Users\me\a b.png",
                r#"<image placement="appLogoOverride" src="file:///C:/Users/me/a%20b.png"/>"#,
            ),
            (
                "C:/pics/a.png",
                r#"<image placement="appLogoOverride" src="file:///C:/pics/a.png"/>"#,
            ),
            (
                r"\\server\share\a.png",
                r#"<image placement="appLogoOverride" src="file://server/share/a.png"/>"#,
            ),
            (
                r#"C:\a&b\"c".png"#,
                r#"<image placement="appLogoOverride" src="file:///C:/a%26b/%22c%22.png"/>"#,
            ),
        ];
        for (path, element) in cases {
            let xml = toast_xml(&Notification::new("t", "b").image(path));
            assert!(
                xml.contains(&format!(
                    "<binding template=\"ToastGeneric\">{element}<text>t</text>"
                )),
                "{path}: {xml}"
            );
        }
    }

    #[test]
    fn toast_xml_has_no_image_element_without_an_image() {
        assert!(!xml("t", "b").contains("<image"));
    }

    #[test]
    fn the_tag_is_the_key_only_when_it_fits() {
        let long = "k".repeat(MAX_TAG_LEN + 1);
        let exact = "k".repeat(MAX_TAG_LEN);
        let wide = "\u{e9}".repeat(MAX_TAG_LEN);
        let cases = [
            (Notification::new("t", "b"), None),
            (Notification::new("t", "b").key(""), None),
            (Notification::new("t", "b").key("copy"), Some("copy")),
            (
                Notification::new("t", "b").key(exact.as_str()),
                Some(exact.as_str()),
            ),
            (Notification::new("t", "b").key(long.as_str()), None),
            (
                Notification::new("t", "b").key(wide.as_str()),
                Some(wide.as_str()),
            ),
        ];
        for (notification, expected) in cases {
            assert_eq!(toast_tag(&notification), expected, "{notification:?}");
        }
    }

    #[test]
    fn the_group_is_the_plugin_id_or_qol() {
        let long = "p".repeat(MAX_TAG_LEN + 1);
        let cases = [
            (None, "qol"),
            (Some(String::new()), "qol"),
            (Some("qol-shot".to_string()), "qol-shot"),
            (Some(long), "qol"),
        ];
        for (plugin_id, expected) in cases {
            assert_eq!(toast_group(plugin_id.clone()), expected, "{plugin_id:?}");
        }
    }
}
