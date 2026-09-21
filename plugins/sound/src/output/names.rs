/// Builds the tile label, such as `Luna 2 · Bluetooth speaker`. The panel
/// splits it at the dot. It never changes the saved value.
pub fn label(device: &str, kind: &str) -> String {
    match (device.is_empty(), kind.is_empty()) {
        (true, true) => String::new(),
        (true, false) => kind.to_owned(),
        (false, true) => device.to_owned(),
        (false, false) => format!("{device}{SEPARATOR}{kind}"),
    }
}

const SEPARATOR: &str = " \u{00b7} ";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_names_every_device_kind() {
        assert_eq!(
            label("Luna 2", "Bluetooth speaker"),
            "Luna 2 \u{00b7} Bluetooth speaker"
        );
        assert_eq!(
            label("Luna 2", "Built-in speaker"),
            "Luna 2 \u{00b7} Built-in speaker"
        );
        assert_eq!(
            label("Luna 2", "USB speaker"),
            "Luna 2 \u{00b7} USB speaker"
        );
        assert_eq!(
            label("Luna 2", "HDMI output"),
            "Luna 2 \u{00b7} HDMI output"
        );
    }

    #[test]
    fn unknown_kind_leaves_the_device_name_alone() {
        assert_eq!(label("Luna 2", ""), "Luna 2");
    }

    #[test]
    fn device_name_with_a_middle_dot_is_preserved() {
        assert_eq!(
            label("Luna\u{00b7}2", "Bluetooth speaker"),
            format!("Luna\u{00b7}2{SEPARATOR}Bluetooth speaker")
        );
        assert_eq!(
            label("Luna \u{00b7} 2", "Bluetooth speaker"),
            format!("Luna \u{00b7} 2{SEPARATOR}Bluetooth speaker")
        );
    }

    #[test]
    fn empty_device_name_produces_no_dangling_separator() {
        assert_eq!(label("", "Bluetooth speaker"), "Bluetooth speaker");
        assert_eq!(label("", ""), "");
    }
}
