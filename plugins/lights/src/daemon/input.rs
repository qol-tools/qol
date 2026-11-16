use serde_json::Value;

use crate::config::model::PRESET_COUNT;

pub const BRIGHTNESS_RANGE: (u16, u16) = (1, 100);
pub const PRESET_BRIGHTNESS_RANGE: (u16, u16) = (0, 100);
pub const MIREK_RANGE: (u16, u16) = (153, 500);
pub const PRESET_NAME_LIMIT: usize = 60;

pub fn ieee(input: &Value) -> Result<String, String> {
    let text = text_field(input, "ieee").ok_or("Pick a light first.")?;
    let ieee = text.trim().to_ascii_uppercase();
    let parts = ieee.split(':').collect::<Vec<_>>();
    let valid = parts.len() == 8
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_hexdigit()));
    if !valid {
        return Err("That is not a light address.".into());
    }
    Ok(ieee)
}

pub fn optional_ieee(input: &Value) -> Result<Option<String>, String> {
    match text_field(input, "ieee") {
        Some(text) if !text.trim().is_empty() => ieee(input).map(Some),
        _ => Ok(None),
    }
}

pub fn preset_number(input: &Value) -> Result<usize, String> {
    let id = text_field(input, "id").ok_or("Pick a preset first.")?;
    id.trim()
        .strip_prefix("preset_")
        .and_then(|number| number.parse::<usize>().ok())
        .filter(|number| (1..=PRESET_COUNT).contains(number))
        .ok_or_else(|| "That preset does not exist.".to_string())
}

pub fn color(input: &Value) -> Result<String, String> {
    let text = text_field(input, "value").ok_or("Pick a colour first.")?;
    let (red, green, blue) = qol_color::parse_hex_color(text.trim())
        .ok_or("A colour looks like #RRGGBB, for example #FFAA00.")?;
    Ok(hex_string(red, green, blue))
}

pub fn name(input: &Value) -> Result<String, String> {
    let text = text_field(input, "value").ok_or("A preset needs a name.")?;
    let name = text.trim();
    if name.is_empty() {
        return Err("A preset needs a name.".into());
    }
    if name.chars().count() > PRESET_NAME_LIMIT {
        return Err(format!(
            "A preset name can have at most {PRESET_NAME_LIMIT} characters."
        ));
    }
    Ok(name.to_string())
}

pub fn number(input: &Value, label: &str, range: (u16, u16)) -> Result<u16, String> {
    let (min, max) = range;
    let wrong = || format!("{label} must be a number from {min} to {max}.");
    let parsed = match input.get("value") {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite())
    .ok_or_else(wrong)?;
    let rounded = parsed.round();
    if rounded < f64::from(min) || rounded > f64::from(max) {
        return Err(wrong());
    }
    Ok(rounded as u16)
}

pub fn hex_string(red: u8, green: u8, blue: u8) -> String {
    format!("#{red:02X}{green:02X}{blue:02X}")
}

pub fn display_hex(hex: &str) -> String {
    match qol_color::parse_hex_color(hex.trim()) {
        Some((red, green, blue)) => hex_string(red, green, blue),
        None => hex_string(255, 255, 255),
    }
}

fn text_field<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_arrive_as_json_numbers_or_strings() {
        let cases = [
            (json!({ "value": 40 }), Some(40)),
            (json!({ "value": "40" }), Some(40)),
            (json!({ "value": " 40 " }), Some(40)),
            (json!({ "value": 40.4 }), Some(40)),
            (json!({ "value": "39.6" }), Some(40)),
            (json!({ "value": 100 }), Some(100)),
            (json!({ "value": 1 }), Some(1)),
            (json!({ "value": 0 }), None),
            (json!({ "value": 101 }), None),
            (json!({ "value": -5 }), None),
            (json!({ "value": "loud" }), None),
            (json!({ "value": true }), None),
            (json!({}), None),
            (Value::Null, None),
        ];
        for (input, expected) in cases {
            let parsed = number(&input, "Brightness", BRIGHTNESS_RANGE).ok();
            assert_eq!(parsed, expected, "{input}");
        }
    }

    #[test]
    fn a_bad_number_says_what_range_is_allowed() {
        let error = number(&json!({ "value": 9 }), "Colour temperature", MIREK_RANGE).unwrap_err();
        assert_eq!(
            error,
            "Colour temperature must be a number from 153 to 500."
        );
    }

    #[test]
    fn preset_brightness_accepts_zero() {
        let input = json!({ "value": "0" });
        assert_eq!(number(&input, "Brightness", PRESET_BRIGHTNESS_RANGE), Ok(0));
    }

    #[test]
    fn light_addresses_are_normalised_or_refused() {
        let cases = [
            (
                json!({ "ieee": "aa:bb:cc:dd:ee:ff:00:11" }),
                Some("AA:BB:CC:DD:EE:FF:00:11"),
            ),
            (
                json!({ "ieee": " 00:11:22:33:44:55:66:77 " }),
                Some("00:11:22:33:44:55:66:77"),
            ),
            (json!({ "ieee": "0x1234" }), None),
            (json!({ "ieee": "AA:BB" }), None),
            (json!({ "ieee": "GG:BB:CC:DD:EE:FF:00:11" }), None),
            (json!({ "ieee": "" }), None),
            (json!({ "ieee": 5 }), None),
            (json!({}), None),
            (Value::Null, None),
        ];
        for (input, expected) in cases {
            assert_eq!(ieee(&input).ok().as_deref(), expected, "{input}");
        }
        assert_eq!(ieee(&json!({})).unwrap_err(), "Pick a light first.");
    }

    #[test]
    fn an_optional_light_address_may_be_absent_but_not_wrong() {
        assert_eq!(optional_ieee(&Value::Null), Ok(None));
        assert_eq!(optional_ieee(&json!({ "ieee": "" })), Ok(None));
        assert_eq!(
            optional_ieee(&json!({ "ieee": "aa:bb:cc:dd:ee:ff:00:11" })),
            Ok(Some("AA:BB:CC:DD:EE:FF:00:11".to_string()))
        );
        assert!(optional_ieee(&json!({ "ieee": "nope" })).is_err());
    }

    #[test]
    fn preset_ids_run_from_one_to_eight() {
        let cases = [
            ("preset_1", Some(1)),
            ("preset_8", Some(8)),
            ("preset_0", None),
            ("preset_9", None),
            ("preset_", None),
            ("one", None),
        ];
        for (id, expected) in cases {
            let parsed = preset_number(&json!({ "id": id })).ok();
            assert_eq!(parsed, expected, "{id}");
        }
        assert!(preset_number(&json!({})).is_err());
    }

    #[test]
    fn colours_come_back_as_upper_case_hash_hex() {
        let cases = [
            ("#ffaa00", Some("#FFAA00")),
            ("FFAA00", Some("#FFAA00")),
            ("#fa0", None),
            ("red", None),
        ];
        for (value, expected) in cases {
            let parsed = color(&json!({ "value": value })).ok();
            assert_eq!(parsed.as_deref(), expected, "{value}");
        }
        assert!(color(&json!({ "value": 5 })).is_err());
    }

    #[test]
    fn a_stored_colour_that_is_not_hex_shows_as_white() {
        let cases = [
            ("#ffaa00", "#FFAA00"),
            ("ffaa00", "#FFAA00"),
            ("nonsense", "#FFFFFF"),
            ("", "#FFFFFF"),
        ];
        for (stored, shown) in cases {
            assert_eq!(display_hex(stored), shown, "{stored}");
        }
    }

    #[test]
    fn preset_names_are_trimmed_and_bounded() {
        assert_eq!(
            name(&json!({ "value": "  Movie  " })).as_deref(),
            Ok("Movie")
        );
        assert!(name(&json!({ "value": "   " })).is_err());
        assert!(name(&json!({ "value": "x".repeat(PRESET_NAME_LIMIT + 1) })).is_err());
        assert!(name(&json!({})).is_err());
    }
}
