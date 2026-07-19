mod platform;

const HOST_ARGUMENT: &str = "__qol-settings-surface-host";

pub fn request(plugin_id: &str) -> anyhow::Result<bool> {
    platform::request(plugin_id)
}

pub fn stop() {
    platform::stop();
}

pub fn run_from_current_args() -> Option<anyhow::Result<()>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    requested_plugin_id(&args).map(platform::run)
}

fn requested_plugin_id(args: &[String]) -> Option<String> {
    match args {
        [argument, plugin_id] if argument == HOST_ARGUMENT => Some(plugin_id.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{requested_plugin_id, HOST_ARGUMENT};

    #[test]
    fn hidden_host_arguments_require_exactly_one_plugin_id() {
        let cases = [
            (vec![], None),
            (vec!["settings"], None),
            (vec![HOST_ARGUMENT, "plugin-a"], Some("plugin-a")),
            (vec![HOST_ARGUMENT, "plugin-a", "extra"], None),
        ];
        for (args, expected) in cases {
            let args = args.into_iter().map(str::to_string).collect::<Vec<_>>();
            assert_eq!(requested_plugin_id(&args).as_deref(), expected);
        }
    }
}
