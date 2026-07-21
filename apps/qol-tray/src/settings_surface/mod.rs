mod platform;

const HOST_ARGUMENT: &str = "__qol-settings-surface-host";
const HOST_SEARCH_ARGUMENT: &str = "__qol-settings-search-host";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SurfaceRequest {
    Plugin(String),
    Search,
}

pub fn request(plugin_id: &str) -> anyhow::Result<bool> {
    let handled = platform::request(plugin_id)?;
    finish_request(plugin_id, handled, crate::paths::open_url)
}

pub fn request_search() -> anyhow::Result<bool> {
    let handled = platform::request_search()?;
    if handled {
        return Ok(true);
    }
    crate::paths::open_url(&crate::commands::deeplink_url(
        "plugins",
        qol_conventions::DEFAULT_PORT,
    ))?;
    Ok(true)
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

pub fn run_from_current_args() -> Option<anyhow::Result<()>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    requested_surface(&args).map(platform::run)
}

fn requested_surface(args: &[String]) -> Option<SurfaceRequest> {
    match args {
        [argument, plugin_id] if argument == HOST_ARGUMENT => {
            Some(SurfaceRequest::Plugin(plugin_id.clone()))
        }
        [argument] if argument == HOST_SEARCH_ARGUMENT => Some(SurfaceRequest::Search),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        finish_request, requested_surface, SurfaceRequest, HOST_ARGUMENT, HOST_SEARCH_ARGUMENT,
    };

    #[test]
    fn hidden_host_arguments_parse_plugin_and_search_requests() {
        let cases = [
            (vec![], None),
            (vec!["settings"], None),
            (
                vec![HOST_ARGUMENT, "plugin-a"],
                Some(SurfaceRequest::Plugin("plugin-a".into())),
            ),
            (vec![HOST_SEARCH_ARGUMENT], Some(SurfaceRequest::Search)),
            (vec![HOST_ARGUMENT, "plugin-a", "extra"], None),
        ];
        for (args, expected) in cases {
            let args = args.into_iter().map(str::to_string).collect::<Vec<_>>();
            assert_eq!(requested_surface(&args), expected);
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
