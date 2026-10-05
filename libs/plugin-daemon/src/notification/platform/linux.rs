use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use zbus::blocking::Proxy;

use super::NotificationPlatform;

pub(super) struct Platform;

impl NotificationPlatform for Platform {
    fn send_notification(&self, title: &str, message: &str) -> bool {
        Command::new("notify-send")
            .arg(title)
            .arg(message)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn os_do_not_disturb(&self) -> Option<bool> {
        let (schema, key) = banner_setting()?;
        parse_gsettings_bool(&gsettings(&["get", schema, key])?).map(|showing| !showing)
    }

    fn set_os_banners(&self, showing: bool) {
        if let Some((schema, key)) = banner_setting() {
            gsettings(&["set", schema, key, if showing { "true" } else { "false" }]);
        }
    }

    fn acquire_inhibit(&self) -> Option<NotificationInhibit> {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(acquire_inhibit_blocking());
        });
        receiver.recv().ok().flatten()
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

fn gsettings(args: &[&str]) -> Option<String> {
    let mut child = Command::new("gsettings")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait().ok()? {
            Some(status) if !status.success() => return None,
            Some(_) => break,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let mut output = String::new();
    child.stdout.as_mut()?.read_to_string(&mut output).ok()?;
    Some(output)
}

fn parse_gsettings_bool(output: &str) -> Option<bool> {
    match output.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}
