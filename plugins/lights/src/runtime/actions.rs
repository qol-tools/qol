pub const TOGGLE_MAIN: &str = "toggle_main";
pub const ON_MAIN: &str = "on_main";
pub const OFF_MAIN: &str = "off_main";
pub const BRIGHTER_MAIN: &str = "brighter_main";
pub const DIMMER_MAIN: &str = "dimmer_main";
pub const WARMER_MAIN: &str = "warmer_main";
pub const COOLER_MAIN: &str = "cooler_main";
pub const PRESET_1: &str = "preset_1";
pub const PRESET_2: &str = "preset_2";
pub const PRESET_3: &str = "preset_3";
pub const PRESET_4: &str = "preset_4";
pub const PRESET_5: &str = "preset_5";
pub const PRESET_6: &str = "preset_6";
pub const PRESET_7: &str = "preset_7";
pub const PRESET_8: &str = "preset_8";
pub const SETTINGS: &str = "settings";
pub const PAIR: &str = "pair";
pub const STOP_PAIR: &str = "stop_pair";
pub const SET_COLOR_MAIN: &str = "set_color_main";
pub const SET_BRIGHTNESS_MAIN: &str = "set_brightness_main";
pub const SET_COLORTEMP_MAIN: &str = "set_colortemp_main";
pub const RELOAD: &str = "reload";

pub const RUN_ACTIONS: &[&str] = &[
    TOGGLE_MAIN,
    ON_MAIN,
    OFF_MAIN,
    BRIGHTER_MAIN,
    DIMMER_MAIN,
    WARMER_MAIN,
    COOLER_MAIN,
    PRESET_1,
    PRESET_2,
    PRESET_3,
    PRESET_4,
    PRESET_5,
    PRESET_6,
    PRESET_7,
    PRESET_8,
    PAIR,
    STOP_PAIR,
    SET_COLOR_MAIN,
    SET_BRIGHTNESS_MAIN,
    SET_COLORTEMP_MAIN,
    RELOAD,
];

pub const ALL_ACTIONS: &[&str] = &[
    TOGGLE_MAIN,
    ON_MAIN,
    OFF_MAIN,
    BRIGHTER_MAIN,
    DIMMER_MAIN,
    WARMER_MAIN,
    COOLER_MAIN,
    PRESET_1,
    PRESET_2,
    PRESET_3,
    PRESET_4,
    PRESET_5,
    PRESET_6,
    PRESET_7,
    PRESET_8,
    SETTINGS,
    PAIR,
    STOP_PAIR,
    SET_COLOR_MAIN,
    SET_BRIGHTNESS_MAIN,
    SET_COLORTEMP_MAIN,
    RELOAD,
];

pub fn is_run_action(action: &str) -> bool {
    RUN_ACTIONS.contains(&action)
}

pub fn is_supported_action(action: &str) -> bool {
    ALL_ACTIONS.contains(&action)
}

#[cfg(test)]
mod tests {
    use super::{is_run_action, is_supported_action};
    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn manifest_actions_have_runtime_handlers() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");

        for action in manifest.executable_actions() {
            let args = manifest
                .catalog_runtime_args(&action.id)
                .expect("executable action must have runtime args");
            let command = args.first().expect("runtime args must name a command");

            assert!(
                is_supported_action(command),
                "action={} command={}",
                action.id,
                command
            );
        }
    }

    #[test]
    fn runtime_contract_actions_have_runtime_handlers() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("qol-runtime.toml");
        let runtime = qol_config::contract::parse_runtime_spec(path).expect("runtime contract");

        for action in runtime.actions.keys() {
            assert!(
                is_run_action(action),
                "qol-runtime action {action} has no Rust runtime backing"
            );
        }
    }
}
