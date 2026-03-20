pub const TOGGLE_MAIN: &str = "toggle-main";
pub const ON_MAIN: &str = "on-main";
pub const OFF_MAIN: &str = "off-main";
pub const BRIGHTER_MAIN: &str = "brighter-main";
pub const DIMMER_MAIN: &str = "dimmer-main";
pub const WARMER_MAIN: &str = "warmer-main";
pub const COOLER_MAIN: &str = "cooler-main";
pub const PRESET_1: &str = "preset-1";
pub const PRESET_2: &str = "preset-2";
pub const PRESET_3: &str = "preset-3";
pub const PRESET_4: &str = "preset-4";
pub const PRESET_5: &str = "preset-5";
pub const PRESET_6: &str = "preset-6";
pub const PRESET_7: &str = "preset-7";
pub const PRESET_8: &str = "preset-8";
pub const SETTINGS: &str = "settings";
pub const PAIR: &str = "pair";
pub const SET_COLOR_MAIN: &str = "set-color-main";
pub const SET_BRIGHTNESS_MAIN: &str = "set-brightness-main";
pub const SET_COLORTEMP_MAIN: &str = "set-colortemp-main";

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
    SET_COLOR_MAIN,
    SET_BRIGHTNESS_MAIN,
    SET_COLORTEMP_MAIN,
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
    SET_COLOR_MAIN,
    SET_BRIGHTNESS_MAIN,
    SET_COLORTEMP_MAIN,
];

pub fn is_run_action(action: &str) -> bool {
    RUN_ACTIONS.contains(&action)
}

pub fn is_supported_action(action: &str) -> bool {
    ALL_ACTIONS.contains(&action)
}
