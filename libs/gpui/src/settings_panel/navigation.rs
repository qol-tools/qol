use std::borrow::Cow;
use std::rc::Rc;

use gpui::App;

pub type CustomPanelInvalidator = Rc<dyn Fn(&mut App)>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsDestination {
    label: Cow<'static, str>,
}

impl SettingsDestination {
    pub fn new(label: impl Into<String>) -> anyhow::Result<Self> {
        let trimmed = label.into().trim().to_string();
        if trimmed.is_empty() {
            anyhow::bail!("a settings destination label must contain visible text");
        }
        Ok(Self {
            label: Cow::Owned(trimmed),
        })
    }

    pub const fn from_static(label: &'static str) -> Self {
        require_visible_text(label.as_bytes());
        Self {
            label: Cow::Borrowed(label),
        }
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

pub trait CustomSettingsBreadcrumbs {
    fn settings_breadcrumbs(&self) -> Vec<SettingsDestination>;
}

const fn require_visible_text(bytes: &[u8]) {
    if !has_visible_text(bytes) {
        panic!("a static settings destination label must contain visible text");
    }
}

const fn has_visible_text(bytes: &[u8]) -> bool {
    let mut index = 0;
    while index < bytes.len() {
        let (code_point, width) = decode_code_point(bytes, index);
        if !is_unicode_whitespace(code_point) {
            return true;
        }
        index += width;
    }
    false
}

const fn decode_code_point(bytes: &[u8], start: usize) -> (u32, usize) {
    let first = bytes[start];
    if first < 0x80 {
        return (first as u32, 1);
    }
    if first < 0xE0 {
        let second = continuation_byte(bytes, start, 1);
        return (((first & 0x1F) as u32) << 6 | (second & 0x3F) as u32, 2);
    }
    if first < 0xF0 {
        let second = continuation_byte(bytes, start, 1);
        let third = continuation_byte(bytes, start, 2);
        return (
            ((first & 0x0F) as u32) << 12 | ((second & 0x3F) as u32) << 6 | (third & 0x3F) as u32,
            3,
        );
    }
    let second = continuation_byte(bytes, start, 1);
    let third = continuation_byte(bytes, start, 2);
    let fourth = continuation_byte(bytes, start, 3);
    (
        ((first & 0x07) as u32) << 18
            | ((second & 0x3F) as u32) << 12
            | ((third & 0x3F) as u32) << 6
            | (fourth & 0x3F) as u32,
        4,
    )
}

const fn continuation_byte(bytes: &[u8], start: usize, offset: usize) -> u8 {
    bytes[start + offset]
}

const fn is_unicode_whitespace(code_point: u32) -> bool {
    matches!(
        code_point,
        0x09..=0x0D
            | 0x20
            | 0x85
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
    )
}

#[cfg(test)]
mod tests {
    use super::SettingsDestination;

    #[test]
    fn dynamic_labels_are_trimmed_and_accepted_when_visible() {
        let destination = SettingsDestination::new("  Add Hotkey  ").unwrap();
        assert_eq!(destination.label(), "Add Hotkey");
    }

    #[test]
    fn blank_dynamic_labels_are_rejected() {
        assert!(SettingsDestination::new("").is_err());
        assert!(SettingsDestination::new("   ").is_err());
        assert!(SettingsDestination::new("\t\n ").is_err());
    }

    #[test]
    fn unicode_whitespace_only_dynamic_labels_are_rejected() {
        assert!(SettingsDestination::new("\u{00A0}").is_err());
        assert!(SettingsDestination::new("\u{3000}").is_err());
        assert!(SettingsDestination::new(" \u{2028}\u{202F}").is_err());
        assert!(SettingsDestination::new("\u{0085}\n").is_err());
    }

    #[test]
    fn dynamic_labels_trim_unicode_whitespace_like_str_trim() {
        let destination = SettingsDestination::new(" \u{3000}Add Shortcut\u{00A0}").unwrap();
        assert_eq!(destination.label(), "Add Shortcut");
        let inner = SettingsDestination::new("Add\u{00A0}Shortcut").unwrap();
        assert_eq!(inner.label(), "Add\u{00A0}Shortcut");
    }

    #[test]
    fn static_labels_with_unicode_visible_text_are_accepted_verbatim() {
        let destination = SettingsDestination::from_static("Add\u{00A0}Shortcut");
        assert_eq!(destination.label(), "Add\u{00A0}Shortcut");
    }

    #[test]
    #[should_panic(expected = "must contain visible text")]
    fn static_unicode_whitespace_only_labels_are_rejected() {
        SettingsDestination::from_static(" \u{3000}\u{0085}");
    }

    #[test]
    fn static_labels_keep_their_exact_text() {
        let destination = SettingsDestination::from_static("Edit Shortcut");
        assert_eq!(destination.label(), "Edit Shortcut");
    }

    #[test]
    fn destinations_compare_by_label() {
        assert_eq!(
            SettingsDestination::from_static("Add Shortcut"),
            SettingsDestination::new("Add Shortcut").unwrap()
        );
        assert_ne!(
            SettingsDestination::from_static("Add Shortcut"),
            SettingsDestination::from_static("Edit Shortcut")
        );
    }
}
