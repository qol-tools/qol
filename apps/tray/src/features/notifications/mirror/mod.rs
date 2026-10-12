mod platform;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MirrorAccess {
    Unsupported,
    Readable,
    Unreadable(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Mirrored {
    app_id: String,
    title: String,
    subtitle: String,
    body: String,
}

pub(super) fn sync(enabled: bool) {
    platform::sync(enabled);
}

pub(super) fn access() -> MirrorAccess {
    platform::access()
}

fn show(app_name: &str, notification: &Mirrored) {
    let (title, body) = toast_text(app_name, notification);
    let shown = crate::settings_surface::show_toast(
        crate::settings_surface::ToastSource {
            group: &notification.app_id,
            name: app_name,
            mark: None,
        },
        &title,
        &body,
        "info",
        None,
        None,
        Some(qol_runtime::protocol::NotificationLayout {
            style: Some("compact".to_string()),
            ..Default::default()
        }),
        None,
    );
    match shown {
        Ok(true) => {}
        Ok(false) => log::debug!("notification mirror: toast host not ready, dropped one"),
        Err(error) => log::warn!("notification mirror: toast failed: {error:#}"),
    }
}

fn toast_text(app_name: &str, notification: &Mirrored) -> (String, String) {
    let title = if notification.title.is_empty() {
        app_name.to_string()
    } else {
        notification.title.clone()
    };
    let body = [notification.subtitle.as_str(), notification.body.as_str()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    (title, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mirrored(title: &str, subtitle: &str, body: &str) -> Mirrored {
        Mirrored {
            app_id: "com.example.chat".to_string(),
            title: title.to_string(),
            subtitle: subtitle.to_string(),
            body: body.to_string(),
        }
    }

    #[test]
    fn toast_text_cases() {
        let cases = [
            (("Ada", "", "hi"), ("Ada", "hi")),
            (("Ada", "#general", "hi"), ("Ada", "#general\nhi")),
            (("", "", "hi"), ("Chat", "hi")),
            (("Ada", "#general", ""), ("Ada", "#general")),
        ];
        for ((title, subtitle, body), (want_title, want_body)) in cases {
            let (got_title, got_body) = toast_text("Chat", &mirrored(title, subtitle, body));
            assert_eq!(
                (got_title.as_str(), got_body.as_str()),
                (want_title, want_body)
            );
        }
    }
}
