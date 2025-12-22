use serde::Deserialize;

/// Mouse button type alias for clarity
pub type MouseButton = u8;

/// Modifier keys state
#[derive(Deserialize, Debug, Clone, Default)]
pub struct ModifierKeys {
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub meta: bool,
}

/// Command sent from client to server
#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum Command {
    MouseMove { x: f64, y: f64 },
    MouseClick { button: MouseButton },
    MouseDown { button: MouseButton },
    MouseUp { button: MouseButton },
    MouseScroll { delta_x: f64, delta_y: f64 },
    KeyPress { key: String, #[serde(default)] modifiers: ModifierKeys },
    KeyRelease { key: String, #[serde(default)] modifiers: ModifierKeys },
    ModifierPress { modifier: String },
    ModifierRelease { modifier: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_parse_mouse_move() {
        let json = r#"{"type":"MouseMove","x":100.5,"y":200.5}"#;
        let cmd: Command = serde_json::from_str(json).unwrap();
        match cmd {
            Command::MouseMove { x, y } => {
                assert_eq!(x, 100.5);
                assert_eq!(y, 200.5);
            }
            _ => panic!("Expected MouseMove"),
        }
    }

    #[test]
    fn test_parse_key_press_with_modifiers() {
        let json = r#"{"type":"KeyPress","key":"a","modifiers":{"ctrl":true,"shift":false}}"#;
        let cmd: Command = serde_json::from_str(json).unwrap();
        match cmd {
            Command::KeyPress { key, modifiers } => {
                assert_eq!(key, "a");
                assert!(modifiers.ctrl);
                assert!(!modifiers.shift);
            }
            _ => panic!("Expected KeyPress"),
        }
    }

    #[test]
    fn test_parse_mouse_click() {
        let json = r#"{"type":"MouseClick","button":1}"#;
        let cmd: Command = serde_json::from_str(json).unwrap();
        match cmd {
            Command::MouseClick { button } => assert_eq!(button, 1),
            _ => panic!("Expected MouseClick"),
        }
    }

    #[test]
    fn test_parse_invalid_json() {
        let json = r#"{"type":"InvalidCommand"}"#;
        assert!(serde_json::from_str::<Command>(json).is_err());
    }

    #[test]
    fn test_modifier_keys_default() {
        let json = r#"{"type":"KeyPress","key":"a"}"#;
        let cmd: Command = serde_json::from_str(json).unwrap();
        match cmd {
            Command::KeyPress { modifiers, .. } => {
                assert!(!modifiers.ctrl);
                assert!(!modifiers.alt);
                assert!(!modifiers.shift);
                assert!(!modifiers.meta);
            }
            _ => panic!("Expected KeyPress"),
        }
    }
}

