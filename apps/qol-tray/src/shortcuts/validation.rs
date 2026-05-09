use super::model::{AppRef, Shortcut, ShortcutAction};

const MAX_NAME_LEN: usize = 128;
const MAX_URL_LEN: usize = 2048;
const MAX_PATH_LEN: usize = 1024;

pub fn validate_shortcut(shortcut: &Shortcut) -> Result<(), String> {
    validate_id(&shortcut.id)?;
    validate_name(&shortcut.name)?;
    validate_action(&shortcut.action)
}

pub fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("id must not be empty".into());
    }
    if id.len() > 64 {
        return Err("id must be at most 64 characters".into());
    }
    if id.starts_with('-') {
        return Err("id must not start with '-'".into());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("id must only contain [A-Za-z0-9_-]".into());
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("name must not be empty".into());
    }
    if name.len() > MAX_NAME_LEN {
        return Err(format!("name must be at most {} characters", MAX_NAME_LEN));
    }
    reject_null_bytes(name, "name")
}

fn validate_action(action: &ShortcutAction) -> Result<(), String> {
    match action {
        ShortcutAction::OpenUrl {
            url,
            browser_override,
        } => {
            validate_url(url)?;
            if let Some(app_ref) = browser_override {
                validate_app_ref(app_ref, "browser_override")?;
            }
            Ok(())
        }
        ShortcutAction::LaunchApp { app } => validate_app_ref(app, "app"),
    }
}

fn validate_url(url: &str) -> Result<(), String> {
    if url.trim().is_empty() {
        return Err("url must not be empty".into());
    }
    if url.len() > MAX_URL_LEN {
        return Err(format!("url must be at most {} characters", MAX_URL_LEN));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("url must start with http:// or https://".into());
    }
    reject_null_bytes(url, "url")
}

fn validate_app_ref(app_ref: &AppRef, field: &str) -> Result<(), String> {
    match app_ref {
        AppRef::BundleId { id } => {
            if id.trim().is_empty() {
                return Err(format!("{} bundle_id must not be empty", field));
            }
            reject_null_bytes(id, field)
        }
        AppRef::Path { path } => {
            if path.trim().is_empty() {
                return Err(format!("{} path must not be empty", field));
            }
            if path.len() > MAX_PATH_LEN {
                return Err(format!(
                    "{} path must be at most {} characters",
                    field, MAX_PATH_LEN
                ));
            }
            reject_null_bytes(path, field)?;
            reject_traversal(path, field)
        }
        AppRef::Name { name } => {
            if name.trim().is_empty() {
                return Err(format!("{} name must not be empty", field));
            }
            reject_null_bytes(name, field)
        }
    }
}

fn reject_null_bytes(s: &str, field: &str) -> Result<(), String> {
    if s.contains('\0') {
        return Err(format!("{} must not contain null bytes", field));
    }
    Ok(())
}

fn reject_traversal(path: &str, field: &str) -> Result<(), String> {
    if path.contains("..") {
        return Err(format!("{} must not contain path traversal", field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn shortcut(id: &str, name: &str, action: ShortcutAction) -> Shortcut {
        Shortcut {
            id: id.to_string(),
            name: name.to_string(),
            enabled: true,
            export_to_launcher: false,
            action,
        }
    }

    fn url_action(url: &str) -> ShortcutAction {
        ShortcutAction::OpenUrl {
            url: url.to_string(),
            browser_override: None,
        }
    }

    #[test]
    fn validate_id_accepts_typical_kebab_and_underscore_ids() {
        for id in ["x", "open-docs", "open_docs", "abc123", "A", "Z9"] {
            assert!(validate_id(id).is_ok(), "id should accept: {id:?}");
        }
    }

    #[test]
    fn validate_id_rejects_invalid_shapes() {
        let cases: &[(&str, &str)] = &[
            ("", "empty"),
            ("-leading-dash", "leading dash"),
            ("has space", "space"),
            ("dot.in.id", "dot"),
            ("slash/in/id", "slash"),
        ];
        for (id, label) in cases {
            assert!(validate_id(id).is_err(), "{label} should reject: {id:?}");
        }
        assert!(
            validate_id("emoji-\u{1F525}").is_err(),
            "non-ascii must reject"
        );
    }

    #[test]
    fn validate_id_accepts_64_chars_and_rejects_65() {
        assert!(validate_id(&"a".repeat(64)).is_ok());
        assert!(validate_id(&"a".repeat(65)).is_err());
    }

    #[test]
    fn validate_shortcut_rejects_empty_or_whitespace_name() {
        for name in ["", " ", "\t", "\n"] {
            let r = validate_shortcut(&shortcut("ok", name, url_action("https://x.io")));
            assert!(r.is_err(), "name {name:?} should reject");
        }
    }

    #[test]
    fn validate_shortcut_rejects_overlong_name() {
        let long = "x".repeat(MAX_NAME_LEN + 1);
        let r = validate_shortcut(&shortcut("ok", &long, url_action("https://x.io")));
        assert!(r.is_err());
    }

    #[test]
    fn validate_url_requires_http_or_https_scheme() {
        let invalid = [
            "",
            "x.io",
            "ftp://x.io",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<script>",
        ];
        for url in invalid {
            let r = validate_shortcut(&shortcut("ok", "Open", url_action(url)));
            assert!(r.is_err(), "url {url:?} should reject");
        }
        for url in ["http://x.io", "https://x.io", "https://x.io/path?q=v#frag"] {
            let r = validate_shortcut(&shortcut("ok", "Open", url_action(url)));
            assert!(r.is_ok(), "url {url:?} should accept");
        }
    }

    #[test]
    fn validate_url_rejects_overlong_payload() {
        let url = format!("https://x.io/{}", "a".repeat(MAX_URL_LEN));
        let r = validate_shortcut(&shortcut("ok", "Open", url_action(&url)));
        assert!(r.is_err());
    }

    #[test]
    fn validate_path_app_rejects_traversal_and_overlong() {
        for path in ["../etc/passwd", "/safe/../escape", ".."] {
            let r = validate_shortcut(&shortcut(
                "ok",
                "Open",
                ShortcutAction::LaunchApp {
                    app: AppRef::Path {
                        path: path.to_string(),
                    },
                },
            ));
            assert!(r.is_err(), "traversal {path:?} should reject");
        }
        let overlong = "a".repeat(MAX_PATH_LEN + 1);
        let r = validate_shortcut(&shortcut(
            "ok",
            "Open",
            ShortcutAction::LaunchApp {
                app: AppRef::Path { path: overlong },
            },
        ));
        assert!(r.is_err(), "overlong path should reject");
    }

    #[test]
    fn validate_app_ref_rejects_empty_for_every_variant() {
        let empties = [
            ShortcutAction::LaunchApp {
                app: AppRef::BundleId { id: " ".into() },
            },
            ShortcutAction::LaunchApp {
                app: AppRef::Path { path: "".into() },
            },
            ShortcutAction::LaunchApp {
                app: AppRef::Name { name: "".into() },
            },
        ];
        for action in empties {
            let r = validate_shortcut(&shortcut("ok", "n", action));
            assert!(r.is_err(), "empty app ref must reject");
        }
    }

    #[test]
    fn validate_rejects_null_byte_in_any_field() {
        let cases = [
            ShortcutAction::OpenUrl {
                url: "https://x.io/\0evil".into(),
                browser_override: None,
            },
            ShortcutAction::LaunchApp {
                app: AppRef::Path {
                    path: "/safe\0/path".into(),
                },
            },
            ShortcutAction::LaunchApp {
                app: AppRef::BundleId {
                    id: "com.app\0evil".into(),
                },
            },
            ShortcutAction::LaunchApp {
                app: AppRef::Name {
                    name: "App\0".into(),
                },
            },
        ];
        for action in cases {
            let r = validate_shortcut(&shortcut("ok", "Open", action));
            assert!(r.is_err(), "null byte must reject");
        }
        let r = validate_shortcut(&shortcut("ok", "Op\0en", url_action("https://x.io")));
        assert!(r.is_err(), "null byte in name must reject");
    }

    #[test]
    fn validate_open_url_validates_browser_override_too() {
        let r = validate_shortcut(&shortcut(
            "ok",
            "Open",
            ShortcutAction::OpenUrl {
                url: "https://x.io".into(),
                browser_override: Some(AppRef::Path {
                    path: "../escape".into(),
                }),
            },
        ));
        assert!(
            r.is_err(),
            "browser_override must be validated, not just url"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_id_accepts_any_charset_within_length(id in "[A-Za-z0-9_][A-Za-z0-9_-]{0,63}") {
            prop_assert!(validate_id(&id).is_ok(), "id={id:?}");
        }

        #[test]
        fn prop_id_rejects_overlong_input(id in "[A-Za-z0-9_]{65,200}") {
            prop_assert!(validate_id(&id).is_err(), "len {} should reject", id.len());
        }

        #[test]
        fn prop_id_rejects_leading_dash(rest in "[A-Za-z0-9_-]{0,63}") {
            let id = format!("-{rest}");
            prop_assert!(validate_id(&id).is_err(), "id={id:?}");
        }

        #[test]
        fn prop_id_rejects_disallowed_chars(
            prefix in "[A-Za-z0-9_]{0,16}",
            bad in "[^A-Za-z0-9_\\-]",
            suffix in "[A-Za-z0-9_-]{0,16}",
        ) {
            let id = format!("{prefix}{bad}{suffix}");
            prop_assert!(validate_id(&id).is_err(), "id={id:?} bad={bad:?}");
        }

        #[test]
        fn prop_url_accepts_only_http_or_https_scheme(
            scheme in "(http|https)",
            host in "[A-Za-z0-9.-]{1,32}",
            path in "[A-Za-z0-9/_.?=&-]{0,128}",
        ) {
            let url = format!("{scheme}://{host}/{path}");
            let r = validate_shortcut(&shortcut("ok", "n", url_action(&url)));
            prop_assert!(r.is_ok(), "url {url:?}");
        }

        #[test]
        fn prop_url_rejects_non_http_schemes(
            scheme in "(ftp|file|javascript|data|gopher|ssh|chrome|about)",
            rest in "[A-Za-z0-9/:.,_-]{0,64}",
        ) {
            let url = format!("{scheme}:{rest}");
            let r = validate_shortcut(&shortcut("ok", "n", url_action(&url)));
            prop_assert!(r.is_err(), "url {url:?}");
        }

        #[test]
        fn prop_path_rejects_traversal_anywhere(
            prefix in "[A-Za-z0-9/_-]{0,32}",
            suffix in "[A-Za-z0-9/_-]{0,32}",
        ) {
            let path = format!("{prefix}..{suffix}");
            let r = validate_shortcut(
                &shortcut(
                    "ok",
                    "n",
                    ShortcutAction::LaunchApp { app: AppRef::Path { path } },
                ),
            );
            prop_assert!(r.is_err());
        }

        #[test]
        fn prop_null_byte_in_url_or_name_rejects(
            prefix in "[A-Za-z0-9 ]{0,16}",
            suffix in "[A-Za-z0-9 ]{0,16}",
        ) {
            let name = format!("{prefix}\0{suffix}");
            let r = validate_shortcut(&shortcut("ok", &name, url_action("https://x.io")));
            prop_assert!(r.is_err(), "null in name must reject");
        }

        #[test]
        fn prop_name_length_boundary(extra in 0usize..200) {
            let name: String = "a".repeat(MAX_NAME_LEN + extra);
            let r = validate_shortcut(&shortcut("ok", &name, url_action("https://x.io")));
            if extra == 0 {
                prop_assert!(r.is_ok(), "len = MAX_NAME_LEN must accept");
            } else {
                prop_assert!(r.is_err(), "len > MAX_NAME_LEN must reject");
            }
        }
    }
}
