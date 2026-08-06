use gpui::*;

pub fn open_keepalive(cx: &mut App, app_id: Option<&str>) -> Option<AnyWindowHandle> {
    let title = keepalive_title(app_id, std::process::id());
    let bounds = Bounds::centered(None, size(px(1.0), px(1.0)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        kind: WindowKind::PopUp,
        focus: false,
        show: false,
        app_id: app_id.map(str::to_owned),
        ..Default::default()
    };
    let window_title = title.clone();
    let handle = cx
        .open_window(options, move |window, cx| {
            window.set_window_title(&window_title);
            cx.new(|_cx| KeepAlive)
        })
        .ok()?;
    let _reason = crate::popup_window::reason_scope("keepalive-open");
    let configured = crate::popup_window::configure_keepalive_window(&title);
    #[cfg(not(debug_assertions))]
    let _ = configured;
    qol_runtime::probe!(
        "KEEPALIVE",
        "title={title} configured={configured} phase=initial contract=non_focusable_unmapped"
    );

    cx.defer(move |_| {
        let _reason = crate::popup_window::reason_scope("keepalive-open");
        let configured = crate::popup_window::configure_keepalive_window(&title);
        #[cfg(not(debug_assertions))]
        let _ = configured;
        qol_runtime::probe!(
            "KEEPALIVE",
            "title={title} configured={configured} phase=settled contract=non_focusable_unmapped"
        );
    });
    Some(handle.into())
}

fn keepalive_title(app_id: Option<&str>, pid: u32) -> String {
    format!("{}-keepalive-{pid}", app_id.unwrap_or("qol"))
}

pub fn is_keepalive_title(title: &str) -> bool {
    let Some((app_id, pid)) = title.rsplit_once("-keepalive-") else {
        return false;
    };
    !app_id.is_empty() && !pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit())
}

struct KeepAlive;

impl Render for KeepAlive {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[cfg(test)]
mod tests {
    use super::{is_keepalive_title, keepalive_title};

    #[test]
    fn keepalive_title_is_unique_to_app_and_process() {
        let cases = [
            (Some("foo"), 42, "foo-keepalive-42"),
            (Some("bar"), 42, "bar-keepalive-42"),
            (None, 7, "qol-keepalive-7"),
        ];
        for (app_id, pid, expected) in cases {
            assert_eq!(keepalive_title(app_id, pid), expected, "app_id: {app_id:?}");
        }
    }

    #[test]
    fn every_title_this_module_builds_is_recognized_again() {
        for app_id in [Some("plugin-cli-sessions"), Some("qol-tray-launcher"), None] {
            let title = keepalive_title(app_id, 61790);
            assert!(is_keepalive_title(&title), "title: {title}");
        }
    }

    #[test]
    fn real_window_titles_are_not_mistaken_for_keepalive_surfaces() {
        let cases = [
            "cli-sessions-panel",
            "qol-launcher",
            "-keepalive-42",
            "foo-keepalive-",
            "foo-keepalive-4x2",
            "keepalive-42",
            "Keep Alive - 42",
        ];
        for title in cases {
            assert!(!is_keepalive_title(title), "title: {title}");
        }
    }
}
