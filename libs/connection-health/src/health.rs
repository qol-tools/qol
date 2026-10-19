use serde::{Deserialize, Deserializer, Serialize};

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    #[default]
    Unknown,
    Good,
    Slow,
    Bad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    SlowAnswers,
    Waiting,
    Unreachable,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionHealth {
    #[serde(default, deserialize_with = "known_level")]
    pub level: Level,
    #[serde(
        default,
        deserialize_with = "recognized",
        skip_serializing_if = "Option::is_none"
    )]
    pub cause: Option<Cause>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
}

impl ConnectionHealth {
    pub fn is_degraded(&self) -> bool {
        matches!(self.level, Level::Slow | Level::Bad)
    }

    pub fn same_signal(&self, other: &Self) -> bool {
        self.level == other.level && self.cause == other.cause
    }
}

fn known_level<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Level, D::Error> {
    recognized(deserializer).map(Option::unwrap_or_default)
}

fn recognized<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Lenient<T> {
        Known(T),
        Unrecognized(serde::de::IgnoredAny),
    }
    Ok(match Lenient::<T>::deserialize(deserializer)? {
        Lenient::Known(value) => Some(value),
        Lenient::Unrecognized(_) => None,
    })
}

#[cfg(test)]
mod tests {
    use super::{Cause, ConnectionHealth, Level};

    #[test]
    fn levels_order_by_severity() {
        assert!(Level::Unknown < Level::Good);
        assert!(Level::Good < Level::Slow);
        assert!(Level::Slow < Level::Bad);
    }

    #[test]
    fn wire_names_are_snake_case() {
        let health = ConnectionHealth {
            level: Level::Bad,
            cause: Some(Cause::SlowAnswers),
            latency_ms: Some(4200),
        };
        assert_eq!(
            serde_json::to_string(&health).unwrap(),
            r#"{"level":"bad","cause":"slow_answers","latency_ms":4200}"#
        );
    }

    #[test]
    fn every_level_and_cause_keeps_its_wire_name() {
        let levels = [
            (Level::Unknown, "unknown"),
            (Level::Good, "good"),
            (Level::Slow, "slow"),
            (Level::Bad, "bad"),
        ];
        for (level, name) in levels {
            assert_eq!(serde_json::to_value(level).unwrap(), name);
            assert_eq!(serde_json::from_value::<Level>(name.into()).unwrap(), level);
        }
        let causes = [
            (Cause::SlowAnswers, "slow_answers"),
            (Cause::Waiting, "waiting"),
            (Cause::Unreachable, "unreachable"),
        ];
        for (cause, name) in causes {
            assert_eq!(serde_json::to_value(cause).unwrap(), name);
            assert_eq!(serde_json::from_value::<Cause>(name.into()).unwrap(), cause);
        }
    }

    #[test]
    fn a_level_from_a_newer_tray_reads_as_unknown() {
        let health: ConnectionHealth =
            serde_json::from_str(r#"{"level":"offline","latency_ms":90}"#).unwrap();
        assert_eq!(health.level, Level::Unknown);
        assert_eq!(health.latency_ms, Some(90));
    }

    #[test]
    fn a_cause_from_a_newer_tray_reads_as_none() {
        let health: ConnectionHealth =
            serde_json::from_str(r#"{"level":"bad","cause":"dns_failed"}"#).unwrap();
        assert_eq!(health.level, Level::Bad);
        assert_eq!(health.cause, None);
        let known: ConnectionHealth =
            serde_json::from_str(r#"{"level":"bad","cause":"unreachable","latency_ms":null}"#)
                .unwrap();
        assert_eq!(known.cause, Some(Cause::Unreachable));
        let null: ConnectionHealth =
            serde_json::from_str(r#"{"level":"good","cause":null}"#).unwrap();
        assert_eq!(null.cause, None);
    }

    #[test]
    fn missing_fields_parse_as_unknown() {
        let health: ConnectionHealth = serde_json::from_str("{}").unwrap();
        assert_eq!(health, ConnectionHealth::default());
        assert_eq!(health.level, Level::Unknown);
    }

    #[test]
    fn unknown_health_omits_optional_fields() {
        assert_eq!(
            serde_json::to_string(&ConnectionHealth::default()).unwrap(),
            r#"{"level":"unknown"}"#
        );
    }

    #[test]
    fn only_slow_and_bad_are_degraded() {
        let at = |level| ConnectionHealth {
            level,
            ..ConnectionHealth::default()
        };
        assert!(!at(Level::Unknown).is_degraded());
        assert!(!at(Level::Good).is_degraded());
        assert!(at(Level::Slow).is_degraded());
        assert!(at(Level::Bad).is_degraded());
    }

    #[test]
    fn latency_alone_does_not_change_the_signal() {
        let first = ConnectionHealth {
            level: Level::Good,
            cause: None,
            latency_ms: Some(120),
        };
        let second = ConnectionHealth {
            latency_ms: Some(340),
            ..first
        };
        let slower = ConnectionHealth {
            level: Level::Slow,
            cause: Some(Cause::SlowAnswers),
            latency_ms: Some(2100),
        };
        assert!(first.same_signal(&second));
        assert!(!first.same_signal(&slower));
    }
}
