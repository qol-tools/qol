mod platform;

use qol_runtime::protocol::NotificationLayout;

const HOST_ARGUMENT: &str = "__qol-settings-surface-host";

#[derive(Debug, PartialEq)]
enum HostBoot {
    Warm,
    Open(String),
}

pub fn request(plugin_id: &str) -> anyhow::Result<bool> {
    let handled = platform::request(plugin_id)?;
    finish_request(plugin_id, handled, crate::paths::open_url)
}

fn finish_request(
    plugin_id: &str,
    handled: bool,
    open_url: impl FnOnce(&str) -> anyhow::Result<()>,
) -> anyhow::Result<bool> {
    if handled {
        return Ok(true);
    }
    open_url(&qol_conventions::settings_url(plugin_id))?;
    qol_runtime::probe!(
        "SURFACE_ACTIVATION",
        "plugin={plugin_id} phase=fallback reason=platform_unsupported outcome=opened"
    );
    Ok(true)
}

pub fn stop() {
    platform::stop();
}

pub fn show_toast(
    title: &str,
    body: &str,
    level: &str,
    action: Option<(&str, &str)>,
    layout: Option<NotificationLayout>,
) -> bool {
    platform::show_toast(title, body, level, action, layout)
}

pub fn prewarm() {
    platform::prewarm();
}

pub fn run_from_current_args() -> Option<anyhow::Result<()>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    requested_boot(&args).map(platform::run)
}

fn requested_boot(args: &[String]) -> Option<HostBoot> {
    match args {
        [argument] if argument == HOST_ARGUMENT => Some(HostBoot::Warm),
        [argument, plugin_id] if argument == HOST_ARGUMENT => {
            Some(HostBoot::Open(plugin_id.clone()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{finish_request, requested_boot, HostBoot, HOST_ARGUMENT};

    #[test]
    fn hidden_host_arguments_select_warm_or_single_plugin_boot() {
        let cases = [
            (vec![], None),
            (vec!["settings"], None),
            (vec![HOST_ARGUMENT], Some(HostBoot::Warm)),
            (
                vec![HOST_ARGUMENT, "plugin-a"],
                Some(HostBoot::Open("plugin-a".into())),
            ),
            (vec![HOST_ARGUMENT, "plugin-a", "extra"], None),
        ];
        for (args, expected) in cases {
            let args = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
            assert_eq!(requested_boot(&args), expected, "args: {args:?}");
        }
    }

    #[test]
    fn unsupported_native_surface_opens_browser_fallback() {
        let mut opened = None;
        let handled = finish_request("plugin-a", false, |url| {
            opened = Some(url.to_string());
            Ok(())
        })
        .unwrap();

        assert!(handled);
        assert_eq!(opened, Some(qol_conventions::settings_url("plugin-a")));
    }

    #[test]
    fn handled_native_surface_skips_browser_fallback() {
        let handled = finish_request("plugin-a", true, |_| {
            panic!("handled native settings must not open the browser")
        })
        .unwrap();

        assert!(handled);
    }
}
