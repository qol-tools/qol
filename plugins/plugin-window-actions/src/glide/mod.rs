#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub(crate) fn from_action(action: &str) -> Option<Self> {
        match action {
            "glide-left" => Some(Self::Left),
            "glide-right" => Some(Self::Right),
            "glide-up" => Some(Self::Up),
            "glide-down" => Some(Self::Down),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Start,
    Heartbeat,
    Stop,
}

impl Phase {
    pub(crate) fn from_input(input: &serde_json::Value) -> Result<Self, String> {
        match input.get("phase").and_then(serde_json::Value::as_str) {
            Some("start") => Ok(Self::Start),
            Some("heartbeat") => Ok(Self::Heartbeat),
            Some("stop") => Ok(Self::Stop),
            Some(phase) => Err(format!("Unknown continuous action phase: {phase}")),
            None => Err("Continuous action input requires a phase".into()),
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Heartbeat => "heartbeat",
            Self::Stop => "stop",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Direction, Phase};

    #[test]
    fn parses_glide_actions() {
        let cases = [
            ("glide-left", Some(Direction::Left)),
            ("glide-right", Some(Direction::Right)),
            ("glide-up", Some(Direction::Up)),
            ("glide-down", Some(Direction::Down)),
            ("snap-left", None),
        ];
        for (action, expected) in cases {
            assert_eq!(Direction::from_action(action), expected, "{action}");
        }
    }

    #[test]
    fn phase_requires_structured_start_or_stop() {
        assert_eq!(
            Phase::from_input(&serde_json::json!({ "phase": "start" })).unwrap(),
            Phase::Start
        );
        assert_eq!(
            Phase::from_input(&serde_json::json!({ "phase": "heartbeat" })).unwrap(),
            Phase::Heartbeat
        );
        assert_eq!(
            Phase::from_input(&serde_json::json!({ "phase": "stop" })).unwrap(),
            Phase::Stop
        );
        assert!(Phase::from_input(&serde_json::Value::Null).is_err());
        assert!(Phase::from_input(&serde_json::json!({ "phase": "repeat" })).is_err());
    }
}
