use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(try_from = "String", into = "String")]
pub enum FlowEnter {
    Copy,
    Paste,
    #[default]
    Detail,
    Action(String),
}

const ACTION_PREFIX: &str = "action:";

impl TryFrom<String> for FlowEnter {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "copy" => Ok(Self::Copy),
            "paste" => Ok(Self::Paste),
            "detail" => Ok(Self::Detail),
            other => match other.strip_prefix(ACTION_PREFIX) {
                Some(id) if !id.trim().is_empty() => Ok(Self::Action(id.to_string())),
                _ => Err(format!(
                    "invalid launcher.enter: {other} (expected copy, paste, detail or action:<id>)"
                )),
            },
        }
    }
}

impl From<FlowEnter> for String {
    fn from(value: FlowEnter) -> Self {
        match value {
            FlowEnter::Copy => "copy".to_string(),
            FlowEnter::Paste => "paste".to_string(),
            FlowEnter::Detail => "detail".to_string(),
            FlowEnter::Action(id) => format!("{ACTION_PREFIX}{id}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FlowPanel {
    Preview,
    #[default]
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_parses_every_wire_value() {
        for (wire, expected) in [
            ("copy", FlowEnter::Copy),
            ("paste", FlowEnter::Paste),
            ("detail", FlowEnter::Detail),
            ("action:pin", FlowEnter::Action("pin".to_string())),
        ] {
            let parsed: FlowEnter = serde_json::from_value(serde_json::json!(wire)).unwrap();
            assert_eq!(parsed, expected);
            assert_eq!(serde_json::to_value(&parsed).unwrap(), wire);
        }
    }

    #[test]
    fn enter_rejects_unknown_values() {
        for wire in ["open", "action:", "action: ", "Copy", ""] {
            assert!(serde_json::from_value::<FlowEnter>(serde_json::json!(wire)).is_err());
        }
    }

    #[test]
    fn panel_parses_and_defaults_to_none() {
        assert_eq!(
            serde_json::from_value::<FlowPanel>(serde_json::json!("preview")).unwrap(),
            FlowPanel::Preview
        );
        assert_eq!(
            serde_json::from_value::<FlowPanel>(serde_json::json!("none")).unwrap(),
            FlowPanel::None
        );
        assert!(serde_json::from_value::<FlowPanel>(serde_json::json!("side")).is_err());
        assert_eq!(FlowPanel::default(), FlowPanel::None);
        assert_eq!(FlowEnter::default(), FlowEnter::Detail);
    }
}
