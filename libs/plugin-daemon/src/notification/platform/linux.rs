use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use qol_desktop_settings::gsettings;
use qol_runtime::protocol::NotificationLevel;
use qol_runtime::Notification;
use qol_sync::Mutex;
use zbus::blocking::Proxy;

use super::NotificationPlatform;

pub(super) struct Platform;

const GSETTINGS_TIMEOUT: Duration = Duration::from_secs(2);

static REPLACE_IDS: Mutex<BTreeMap<String, u32>> = Mutex::new(BTreeMap::new());

impl NotificationPlatform for Platform {
    fn send(&self, notification: &Notification) -> bool {
        let key = notification.meta.key.as_deref();
        let replace = key.and_then(remembered_id);
        let args = notify_send_args(notification, key.is_some(), replace);
        if let Some(output) = run_notify_send(&args) {
            if let Some(key) = key {
                remember_id(key, parse_notification_id(&output));
            }
            return true;
        }
        key.is_some() && run_notify_send(&notify_send_args(notification, false, None)).is_some()
    }

    fn os_do_not_disturb(&self) -> Option<bool> {
        self.os_banners_showing().map(|showing| !showing)
    }

    fn os_banners_showing(&self) -> Option<bool> {
        let (schema, key) = banner_setting()?;
        parse_gsettings_bool(&gsettings::get_within(schema, key, GSETTINGS_TIMEOUT).ok()?)
    }

    fn set_os_banners(&self, showing: bool) {
        if let Some((schema, key)) = banner_setting() {
            let value = if showing { "true" } else { "false" };
            let _ = gsettings::set_within(schema, key, value, GSETTINGS_TIMEOUT);
        }
    }

    fn acquire_inhibit(&self) -> Option<NotificationInhibit> {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(acquire_inhibit_blocking());
        });
        receiver.recv().ok().flatten()
    }

    fn release_toast_identity(&self) -> std::io::Result<()> {
        Ok(())
    }

    fn install_toast_identity(&self) -> std::io::Result<()> {
        Ok(())
    }

    fn uninstall_toast_identity(&self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct NotificationInhibit {
    connection: zbus::blocking::Connection,
    cookie: u32,
}

impl Drop for NotificationInhibit {
    fn drop(&mut self) {
        let proxy = Proxy::new(
            &self.connection,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
        );
        if let Ok(proxy) = proxy {
            let _ = proxy.call::<_, _, ()>("Uninhibit", &(self.cookie,));
        }
    }
}

fn acquire_inhibit_blocking() -> Option<NotificationInhibit> {
    let connection = zbus::blocking::Connection::session().ok()?;
    let proxy = Proxy::new(
        &connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .ok()?;
    let capabilities: Vec<String> = proxy.call("GetCapabilities", &()).ok()?;
    if !capabilities
        .iter()
        .any(|capability| capability == "inhibition")
    {
        return None;
    }
    let cookie: u32 = proxy
        .call("Inhibit", &("qol", "qol handles notifications"))
        .ok()?;
    Some(NotificationInhibit { connection, cookie })
}

fn run_notify_send(args: &[String]) -> Option<String> {
    let output = Command::new("notify-send")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn notify_send_args(
    notification: &Notification,
    print_id: bool,
    replace: Option<u32>,
) -> Vec<String> {
    let mut args = vec!["-u".to_string(), urgency(notification.level).to_string()];
    if let Some(duration_ms) = notification.meta.duration_ms {
        args.push("-t".to_string());
        args.push(
            duration_ms
                .min(u64::from(i32::MAX.unsigned_abs()))
                .to_string(),
        );
    }
    if let Some(image) = &notification.meta.image {
        args.push("-i".to_string());
        args.push(image.clone());
    }
    if print_id {
        args.push("-p".to_string());
    }
    if let Some(id) = replace {
        args.push("-r".to_string());
        args.push(id.to_string());
    }
    args.push("--".to_string());
    args.push(notification.title.clone());
    args.push(notification.body.clone());
    args
}

fn urgency(level: NotificationLevel) -> &'static str {
    match level {
        NotificationLevel::Info => "low",
        NotificationLevel::Warn => "normal",
        NotificationLevel::Error => "critical",
    }
}

fn parse_notification_id(output: &str) -> Option<u32> {
    output.lines().next()?.trim().parse().ok()
}

fn remembered_id(key: &str) -> Option<u32> {
    let ids = REPLACE_IDS.lock();
    ids.get(key).copied()
}

fn remember_id(key: &str, id: Option<u32>) {
    let Some(id) = id else {
        return;
    };
    let mut ids = REPLACE_IDS.lock();
    ids.insert(key.to_string(), id);
}

fn banner_setting() -> Option<(&'static str, &'static str)> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok()?;
    if desktop.contains("X-Cinnamon") {
        return Some((
            "org.cinnamon.desktop.notifications",
            "display-notifications",
        ));
    }
    if desktop.contains("GNOME") {
        return Some(("org.gnome.desktop.notifications", "show-banners"));
    }
    None
}

fn parse_gsettings_bool(output: &str) -> Option<bool> {
    match output.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(level: NotificationLevel) -> Notification {
        Notification::new("Title", "Body").level(level)
    }

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().copied().map(String::from).collect()
    }

    #[test]
    fn notify_send_args_carry_what_the_notification_sets() {
        let cases = [
            (
                notification(NotificationLevel::Info),
                false,
                None,
                vec!["-u", "low", "--", "Title", "Body"],
            ),
            (
                notification(NotificationLevel::Warn),
                false,
                None,
                vec!["-u", "normal", "--", "Title", "Body"],
            ),
            (
                notification(NotificationLevel::Error),
                false,
                None,
                vec!["-u", "critical", "--", "Title", "Body"],
            ),
            (
                notification(NotificationLevel::Info).until_dismissed(),
                false,
                None,
                vec!["-u", "low", "-t", "0", "--", "Title", "Body"],
            ),
            (
                notification(NotificationLevel::Info).stay(Duration::from_millis(2500)),
                false,
                None,
                vec!["-u", "low", "-t", "2500", "--", "Title", "Body"],
            ),
            (
                notification(NotificationLevel::Info).stay(Duration::MAX),
                false,
                None,
                vec!["-u", "low", "-t", "2147483647", "--", "Title", "Body"],
            ),
            (
                notification(NotificationLevel::Info).image("/tmp/a.png"),
                false,
                None,
                vec!["-u", "low", "-i", "/tmp/a.png", "--", "Title", "Body"],
            ),
            (
                notification(NotificationLevel::Info).key("copy"),
                true,
                None,
                vec!["-u", "low", "-p", "--", "Title", "Body"],
            ),
            (
                notification(NotificationLevel::Info).key("copy"),
                true,
                Some(42),
                vec!["-u", "low", "-p", "-r", "42", "--", "Title", "Body"],
            ),
            (
                notification(NotificationLevel::Error)
                    .key("copy")
                    .image("/tmp/a.png")
                    .stay(Duration::from_secs(3)),
                true,
                Some(7),
                vec![
                    "-u",
                    "critical",
                    "-t",
                    "3000",
                    "-i",
                    "/tmp/a.png",
                    "-p",
                    "-r",
                    "7",
                    "--",
                    "Title",
                    "Body",
                ],
            ),
            (
                Notification::new("--icon=/x", "-x"),
                false,
                None,
                vec!["-u", "low", "--", "--icon=/x", "-x"],
            ),
            (
                notification(NotificationLevel::Info).key("copy"),
                false,
                None,
                vec!["-u", "low", "--", "Title", "Body"],
            ),
        ];
        for (notification, print_id, replace, expected) in cases {
            assert_eq!(
                notify_send_args(&notification, print_id, replace),
                strings(&expected),
                "{notification:?} print_id={print_id} replace={replace:?}"
            );
        }
    }

    #[test]
    fn the_printed_id_is_the_first_line_as_a_number() {
        let cases = [
            ("42\n", Some(42)),
            ("7", Some(7)),
            ("  15  \n", Some(15)),
            ("9\nextra\n", Some(9)),
            ("", None),
            ("\n", None),
            ("abc\n", None),
            ("-3\n", None),
            ("99999999999\n", None),
        ];
        for (output, expected) in cases {
            assert_eq!(parse_notification_id(output), expected, "{output:?}");
        }
    }

    #[test]
    fn remembered_ids_are_kept_per_key_and_only_when_parsed() {
        assert_eq!(remembered_id("linux-test-a"), None);
        remember_id("linux-test-a", None);
        assert_eq!(remembered_id("linux-test-a"), None);
        remember_id("linux-test-a", Some(5));
        remember_id("linux-test-b", Some(6));
        assert_eq!(remembered_id("linux-test-a"), Some(5));
        assert_eq!(remembered_id("linux-test-b"), Some(6));
        remember_id("linux-test-a", Some(8));
        assert_eq!(remembered_id("linux-test-a"), Some(8));
    }
}
