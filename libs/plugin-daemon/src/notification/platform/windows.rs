use std::ffi::c_void;
use std::sync::OnceLock;

use qol_platform::native::registry::{self, Hive};
use windows::core::HSTRING;
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows_sys::Win32::UI::Shell::{
    SHQueryUserNotificationState, QUERY_USER_NOTIFICATION_STATE, QUNS_ACCEPTS_NOTIFICATIONS,
};

use super::NotificationPlatform;

const TOAST_APP_ID: &str = "QolTools.QolTray";
const APP_ID_REGISTRY_KEY: &str = r"Software\Classes\AppUserModelId\";
const FOCUS_ASSIST_OFF: u32 = 0;
const WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED: u64 = 0x0D83_063E_A3BF_1C75;

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryWnfStateData(
        state_name: *const u64,
        type_id: *const c_void,
        explicit_scope: *const c_void,
        change_stamp: *mut u32,
        buffer: *mut c_void,
        buffer_size: *mut u32,
    ) -> i32;
}

pub(super) struct Platform;

impl NotificationPlatform for Platform {
    fn send_notification(&self, title: &str, message: &str) -> bool {
        if !app_id_registered() {
            log::warn!("[notifications] could not register the {TOAST_APP_ID} toast identity");
        }
        match show_toast(title, message) {
            Ok(()) => true,
            Err(error) => {
                log::warn!("[notifications] Windows toast failed: {error}");
                false
            }
        }
    }

    fn os_do_not_disturb(&self) -> Option<bool> {
        do_not_disturb(focus_assist_profile(), shell_notification_state())
    }

    fn set_os_banners(&self, _showing: bool) {}

    fn acquire_inhibit(&self) -> Option<NotificationInhibit> {
        None
    }
}

pub struct NotificationInhibit;

fn app_id_registered() -> bool {
    static REGISTERED: OnceLock<bool> = OnceLock::new();
    *REGISTERED.get_or_init(register_app_id)
}

fn register_app_id() -> bool {
    registry::write_string(
        Hive::CurrentUser,
        &format!("{APP_ID_REGISTRY_KEY}{TOAST_APP_ID}"),
        "DisplayName",
        qol_conventions::TRAY_DISPLAY_NAME,
    )
    .is_ok()
}

fn show_toast(title: &str, message: &str) -> windows::core::Result<()> {
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(toast_xml(title, message)))?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(TOAST_APP_ID))?.Show(&toast)
}

fn toast_xml(title: &str, message: &str) -> String {
    format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        xml_escape(title),
        xml_escape(message)
    )
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
            other => escaped.push(other),
        }
    }
    escaped
}

fn focus_assist_profile() -> Option<u32> {
    let state_name = WNF_SHEL_QUIETHOURS_ACTIVE_PROFILE_CHANGED;
    let mut change_stamp = 0u32;
    let mut profile = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        NtQueryWnfStateData(
            &state_name,
            std::ptr::null(),
            std::ptr::null(),
            &mut change_stamp,
            std::ptr::from_mut(&mut profile).cast(),
            &mut size,
        )
    };
    (status >= 0).then_some(if size == 0 { FOCUS_ASSIST_OFF } else { profile })
}

fn shell_notification_state() -> Option<QUERY_USER_NOTIFICATION_STATE> {
    let mut state: QUERY_USER_NOTIFICATION_STATE = 0;
    (unsafe { SHQueryUserNotificationState(&mut state) } >= 0).then_some(state)
}

fn do_not_disturb(
    focus_profile: Option<u32>,
    shell_state: Option<QUERY_USER_NOTIFICATION_STATE>,
) -> Option<bool> {
    let focus = focus_profile.map(|profile| profile != FOCUS_ASSIST_OFF);
    let shell = shell_state.map(|state| state != QUNS_ACCEPTS_NOTIFICATIONS);
    match (focus, shell) {
        (None, None) => None,
        (focus, shell) => Some(focus.unwrap_or(false) || shell.unwrap_or(false)),
    }
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::UI::Shell::{
        QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_QUIET_TIME, QUNS_RUNNING_D3D_FULL_SCREEN,
    };

    use super::*;

    #[test]
    fn do_not_disturb_combines_focus_assist_and_shell_state() {
        let cases = [
            (None, None, None),
            (Some(0), None, Some(false)),
            (Some(1), None, Some(true)),
            (Some(2), None, Some(true)),
            (None, Some(QUNS_ACCEPTS_NOTIFICATIONS), Some(false)),
            (None, Some(QUNS_BUSY), Some(true)),
            (None, Some(QUNS_PRESENTATION_MODE), Some(true)),
            (None, Some(QUNS_RUNNING_D3D_FULL_SCREEN), Some(true)),
            (None, Some(QUNS_QUIET_TIME), Some(true)),
            (Some(0), Some(QUNS_ACCEPTS_NOTIFICATIONS), Some(false)),
            (Some(1), Some(QUNS_ACCEPTS_NOTIFICATIONS), Some(true)),
            (Some(0), Some(QUNS_BUSY), Some(true)),
        ];
        for (focus, shell, expected) in cases {
            assert_eq!(
                do_not_disturb(focus, shell),
                expected,
                "focus={focus:?} shell={shell:?}"
            );
        }
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
            assert!(toast_xml(title, body).contains(fragment), "{title} {body}");
        }
    }
}
