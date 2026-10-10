use crate::grammar::{Key, NamedKey};

const VK_BACK: u16 = 0x08;
const VK_TAB: u16 = 0x09;
const VK_RETURN: u16 = 0x0D;
const VK_PAUSE: u16 = 0x13;
const VK_ESCAPE: u16 = 0x1B;
const VK_SPACE: u16 = 0x20;
const VK_PRIOR: u16 = 0x21;
const VK_NEXT: u16 = 0x22;
const VK_END: u16 = 0x23;
const VK_HOME: u16 = 0x24;
const VK_LEFT: u16 = 0x25;
const VK_UP: u16 = 0x26;
const VK_RIGHT: u16 = 0x27;
const VK_DOWN: u16 = 0x28;
const VK_SNAPSHOT: u16 = 0x2C;
const VK_INSERT: u16 = 0x2D;
const VK_DELETE: u16 = 0x2E;
const VK_0: u16 = 0x30;
const VK_A: u16 = 0x41;
const VK_F1: u16 = 0x70;

pub fn key_to_vk(key: Key) -> Option<u16> {
    Some(match key {
        Key::Letter(index) if index < 26 => VK_A + u16::from(index),
        Key::Digit(index) if index < 10 => VK_0 + u16::from(index),
        Key::Function(number @ 1..=12) => VK_F1 + u16::from(number - 1),
        Key::Named(named) => named_to_vk(named),
        Key::Letter(_) | Key::Digit(_) | Key::Function(_) | Key::Symbol(_) => return None,
    })
}

fn named_to_vk(named: NamedKey) -> u16 {
    match named {
        NamedKey::Space => VK_SPACE,
        NamedKey::Enter => VK_RETURN,
        NamedKey::Escape => VK_ESCAPE,
        NamedKey::Tab => VK_TAB,
        NamedKey::Backspace => VK_BACK,
        NamedKey::Delete => VK_DELETE,
        NamedKey::Insert => VK_INSERT,
        NamedKey::Home => VK_HOME,
        NamedKey::End => VK_END,
        NamedKey::PageUp => VK_PRIOR,
        NamedKey::PageDown => VK_NEXT,
        NamedKey::Up => VK_UP,
        NamedKey::Down => VK_DOWN,
        NamedKey::Left => VK_LEFT,
        NamedKey::Right => VK_RIGHT,
        NamedKey::PrintScreen => VK_SNAPSHOT,
        NamedKey::Pause => VK_PAUSE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar;

    fn vk(input: &str) -> Option<u16> {
        key_to_vk(grammar::parse(input).unwrap().key)
    }

    #[test]
    fn maps_grammar_keys_to_virtual_keys() {
        assert_eq!(vk("super+space"), Some(VK_SPACE));
        assert_eq!(vk("r"), Some(0x52));
        assert_eq!(vk("0"), Some(0x30));
        assert_eq!(vk("9"), Some(0x39));
        assert_eq!(vk("f1"), Some(0x70));
        assert_eq!(vk("f12"), Some(0x7B));
        assert_eq!(vk("backspace"), Some(VK_BACK));
        assert_eq!(vk("delete"), Some(VK_DELETE));
        assert_eq!(vk("printscreen"), Some(VK_SNAPSHOT));
    }

    #[test]
    fn rejects_out_of_range_and_layout_dependent_keys() {
        assert_eq!(key_to_vk(Key::Letter(26)), None);
        assert_eq!(key_to_vk(Key::Digit(10)), None);
        assert_eq!(key_to_vk(Key::Function(0)), None);
        assert_eq!(key_to_vk(Key::Function(13)), None);
        assert_eq!(key_to_vk(Key::Symbol('+')), None);
    }
}
