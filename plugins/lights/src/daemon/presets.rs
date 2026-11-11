use serde_json::Value;

use super::input::{self, BRIGHTNESS_RANGE, MIREK_RANGE, PRESET_BRIGHTNESS_RANGE};
use crate::config::model::{LightLook, PresetConfig, PresetSlot, PRESET_COUNT};
use crate::runtime::actions;

const ALL_IN_USE: &str = "All eight presets are in use. Remove one first.";
const NOT_IN_LIST: &str = "That preset is not in the list.";

pub const PRESET_ACTIONS: &[&str] = &[
    actions::ADD_PRESET,
    actions::REMOVE_PRESET,
    actions::SET_PRESET_NAME,
    actions::PRESET_POWER_ON,
    actions::PRESET_POWER_OFF,
    actions::SET_PRESET_COLOR,
    actions::SET_PRESET_BRIGHTNESS,
    actions::SET_PRESET_COLORTEMP,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresetEdit {
    Name(String),
    Power(bool),
    Color(String),
    Brightness(u8),
    Mirek(u16),
}

pub fn any_enabled(presets: &PresetConfig) -> bool {
    (1..=PRESET_COUNT).any(|number| presets.slot(number).is_some_and(|slot| slot.enabled))
}

pub fn is_preset_action(action: &str) -> bool {
    PRESET_ACTIONS.contains(&action)
}

pub fn apply(
    presets: &mut PresetConfig,
    action: &str,
    input: &Value,
    light_look: impl FnOnce(Option<&str>) -> Result<LightLook, String>,
) -> Result<(), String> {
    if action == actions::ADD_PRESET {
        let ieee = input::optional_ieee(input)?;
        let look = light_look(ieee.as_deref())?;
        return add(presets, &look).map(|_| ());
    }
    let number = input::preset_number(input)?;
    if action == actions::REMOVE_PRESET {
        remove(presets, number);
        return Ok(());
    }
    edit(presets, number, parse_edit(action, input)?)
}

fn parse_edit(action: &str, input: &Value) -> Result<PresetEdit, String> {
    match action {
        actions::SET_PRESET_NAME => input::name(input).map(PresetEdit::Name),
        actions::PRESET_POWER_ON => Ok(PresetEdit::Power(true)),
        actions::PRESET_POWER_OFF => Ok(PresetEdit::Power(false)),
        actions::SET_PRESET_COLOR => input::color(input).map(PresetEdit::Color),
        actions::SET_PRESET_BRIGHTNESS => {
            input::number(input, "Brightness", PRESET_BRIGHTNESS_RANGE)
                .map(|level| PresetEdit::Brightness(level as u8))
        }
        actions::SET_PRESET_COLORTEMP => {
            input::number(input, "Colour temperature", MIREK_RANGE).map(PresetEdit::Mirek)
        }
        _ => Err(format!("'{action}' is not a preset action.")),
    }
}

pub fn add(presets: &mut PresetConfig, look: &LightLook) -> Result<usize, String> {
    let number = (1..=PRESET_COUNT)
        .find(|number| presets.slot(*number).is_some_and(|slot| !slot.enabled))
        .ok_or(ALL_IN_USE)?;
    let slot = presets.slot_mut(number).ok_or(ALL_IN_USE)?;
    slot.enabled = true;
    slot.power_on = look.on;
    slot.brightness = look.brightness.clamp(0, BRIGHTNESS_RANGE.1 as u8);
    slot.color_hex = input::display_hex(&look.color_hex);
    slot.mirek = look.mirek.clamp(MIREK_RANGE.0, MIREK_RANGE.1);
    Ok(number)
}

pub fn remove(presets: &mut PresetConfig, number: usize) {
    if let Some(slot) = presets.slot_mut(number) {
        *slot = PresetSlot::named(&format!("Preset {number}"));
    }
}

pub fn edit(presets: &mut PresetConfig, number: usize, change: PresetEdit) -> Result<(), String> {
    let slot = presets
        .slot_mut(number)
        .filter(|slot| slot.enabled)
        .ok_or(NOT_IN_LIST)?;
    match change {
        PresetEdit::Name(name) => slot.name = name,
        PresetEdit::Power(on) => slot.power_on = on,
        PresetEdit::Color(color) => slot.color_hex = color,
        PresetEdit::Brightness(level) => slot.brightness = level,
        PresetEdit::Mirek(mirek) => slot.mirek = mirek,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn look() -> LightLook {
        LightLook {
            on: false,
            color_hex: "#ffaa00".into(),
            brightness: 40,
            mirek: 420,
        }
    }

    fn with_enabled(numbers: &[usize]) -> PresetConfig {
        let mut presets = PresetConfig::default();
        for number in numbers {
            presets.slot_mut(*number).unwrap().enabled = true;
        }
        presets
    }

    fn run(presets: &mut PresetConfig, action: &str, input: Value) -> Result<(), String> {
        apply(presets, action, &input, |_| Ok(look()))
    }

    #[test]
    fn adding_fills_the_first_free_slot_with_the_light_and_keeps_its_name() {
        let cases: [(&[usize], usize); 4] = [
            (&[], 1),
            (&[1], 2),
            (&[1, 3], 2),
            (&[1, 2, 3, 4, 5, 6, 7], 8),
        ];
        for (used, expected) in cases {
            let mut presets = with_enabled(used);

            let number = add(&mut presets, &look()).unwrap();

            assert_eq!(number, expected, "{used:?}");
            let slot = presets.slot(number).unwrap();
            assert!(slot.enabled);
            assert_eq!(slot.name, format!("Preset {number}"));
            assert!(!slot.power_on);
            assert_eq!(slot.brightness, 40);
            assert_eq!(slot.color_hex, "#FFAA00");
            assert_eq!(slot.mirek, 420);
        }
    }

    #[test]
    fn adding_with_every_slot_used_says_to_remove_one() {
        let mut presets = with_enabled(&[1, 2, 3, 4, 5, 6, 7, 8]);

        let error = add(&mut presets, &look()).unwrap_err();

        assert_eq!(error, "All eight presets are in use. Remove one first.");
    }

    #[test]
    fn removing_resets_the_slot_and_frees_it_for_the_next_add() {
        let mut presets = with_enabled(&[1, 2, 3]);
        edit(&mut presets, 2, PresetEdit::Name("Movie".into())).unwrap();

        remove(&mut presets, 2);

        let slot = presets.slot(2).unwrap();
        assert!(!slot.enabled);
        assert_eq!(slot.name, "Preset 2");
        assert_eq!(slot.brightness, 100);
        assert_eq!(add(&mut presets, &look()), Ok(2));
    }

    #[test]
    fn edits_change_one_field_of_an_enabled_slot() {
        let cases = [
            (PresetEdit::Name("Movie".into()), "name"),
            (PresetEdit::Power(false), "power_on"),
            (PresetEdit::Color("#010203".into()), "color_hex"),
            (PresetEdit::Brightness(0), "brightness"),
            (PresetEdit::Mirek(153), "mirek"),
        ];
        for (change, field) in cases {
            let mut presets = with_enabled(&[3]);
            let before = serde_json::to_value(presets.slot(3).unwrap()).unwrap();

            edit(&mut presets, 3, change).unwrap();

            let after = serde_json::to_value(presets.slot(3).unwrap()).unwrap();
            for key in ["name", "power_on", "color_hex", "brightness", "mirek"] {
                assert_eq!(before[key] != after[key], key == field, "{field} vs {key}");
            }
        }
    }

    #[test]
    fn a_preset_that_is_not_in_the_list_cannot_be_edited() {
        let mut presets = with_enabled(&[1]);
        let error = edit(&mut presets, 2, PresetEdit::Power(false)).unwrap_err();
        assert_eq!(error, "That preset is not in the list.");
    }

    #[test]
    fn actions_read_their_input_and_change_the_named_slot() {
        let mut presets = with_enabled(&[1, 4]);
        let steps = [
            (
                actions::SET_PRESET_NAME,
                json!({ "id": "preset_4", "value": " Reading " }),
            ),
            (actions::PRESET_POWER_OFF, json!({ "id": "preset_4" })),
            (
                actions::SET_PRESET_COLOR,
                json!({ "id": "preset_4", "value": "#00ff00" }),
            ),
            (
                actions::SET_PRESET_BRIGHTNESS,
                json!({ "id": "preset_4", "value": "0" }),
            ),
            (
                actions::SET_PRESET_COLORTEMP,
                json!({ "id": "preset_4", "value": 250 }),
            ),
        ];
        for (action, input) in steps {
            run(&mut presets, action, input).unwrap();
        }
        run(
            &mut presets,
            actions::PRESET_POWER_ON,
            json!({ "id": "preset_4" }),
        )
        .unwrap();

        let slot = presets.slot(4).unwrap();
        assert_eq!(slot.name, "Reading");
        assert!(slot.power_on);
        assert_eq!(slot.color_hex, "#00FF00");
        assert_eq!(slot.brightness, 0);
        assert_eq!(slot.mirek, 250);
        assert_eq!(presets.slot(1).unwrap().name, "Preset 1");
    }

    #[test]
    fn bad_input_is_an_error_and_changes_nothing() {
        let cases = [
            (
                actions::SET_PRESET_COLOR,
                json!({ "id": "preset_1", "value": "red" }),
            ),
            (
                actions::SET_PRESET_BRIGHTNESS,
                json!({ "id": "preset_1", "value": 101 }),
            ),
            (
                actions::SET_PRESET_COLORTEMP,
                json!({ "id": "preset_1", "value": 100 }),
            ),
            (
                actions::SET_PRESET_NAME,
                json!({ "id": "preset_1", "value": "" }),
            ),
            (actions::PRESET_POWER_OFF, json!({ "id": "preset_9" })),
            (actions::PRESET_POWER_OFF, json!({})),
            (actions::REMOVE_PRESET, Value::Null),
        ];
        for (action, input) in cases {
            let mut presets = with_enabled(&[1]);
            let before = serde_json::to_value(&presets).unwrap();

            assert!(
                run(&mut presets, action, input.clone()).is_err(),
                "{action} {input}"
            );

            assert_eq!(serde_json::to_value(&presets).unwrap(), before, "{action}");
        }
    }

    #[test]
    fn add_preset_asks_for_the_named_light_or_the_main_light() {
        let cases = [
            (json!({}), None),
            (Value::Null, None),
            (json!({ "ieee": "" }), None),
            (
                json!({ "ieee": "aa:bb:cc:dd:ee:ff:00:11" }),
                Some("AA:BB:CC:DD:EE:FF:00:11"),
            ),
        ];
        for (input, expected) in cases {
            let mut presets = PresetConfig::default();
            let mut asked = None;

            apply(&mut presets, actions::ADD_PRESET, &input, |ieee| {
                asked = Some(ieee.map(str::to_string));
                Ok(look())
            })
            .unwrap();

            assert_eq!(asked, Some(expected.map(str::to_string)), "{input}");
            assert!(presets.slot(1).unwrap().enabled);
        }
    }

    #[test]
    fn add_preset_stops_when_the_light_is_unknown() {
        let mut presets = PresetConfig::default();

        let error = apply(&mut presets, actions::ADD_PRESET, &Value::Null, |_| {
            Err("That light is not paired.".into())
        })
        .unwrap_err();

        assert_eq!(error, "That light is not paired.");
        assert!(!presets.slot(1).unwrap().enabled);
    }

    #[test]
    fn remove_preset_works_on_a_disabled_slot() {
        let mut presets = PresetConfig::default();
        run(
            &mut presets,
            actions::REMOVE_PRESET,
            json!({ "id": "preset_5" }),
        )
        .unwrap();
        assert_eq!(presets.slot(5).unwrap().name, "Preset 5");
    }

    #[test]
    fn presets_exist_once_any_slot_is_enabled() {
        assert!(!any_enabled(&PresetConfig::default()));
        assert!(any_enabled(&with_enabled(&[8])));
    }

    #[test]
    fn every_preset_action_is_a_runtime_action() {
        for action in PRESET_ACTIONS {
            assert!(actions::is_run_action(action), "{action}");
            assert!(is_preset_action(action));
        }
        assert!(!is_preset_action(actions::PRESET_1));
    }
}
