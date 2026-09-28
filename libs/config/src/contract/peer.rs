use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PeerReplay {
    Idempotent,
    Never,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PeerExposure {
    pub replay: PeerReplay,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize, Serialize, PartialEq)]
    struct Holder {
        #[serde(default)]
        peer: Option<PeerExposure>,
    }

    #[test]
    fn absent_declaration_denies_exposure() {
        let holder: Holder = toml::from_str("").unwrap();
        assert_eq!(holder.peer, None);
    }

    #[test]
    fn replay_values_round_trip() {
        let cases = [
            ("idempotent", PeerReplay::Idempotent),
            ("never", PeerReplay::Never),
        ];
        for (raw, expected) in cases {
            let holder: Holder =
                toml::from_str(&format!("peer = {{ replay = \"{raw}\" }}")).unwrap();
            assert_eq!(
                holder.peer,
                Some(PeerExposure { replay: expected }),
                "case: {raw}"
            );
        }
    }

    #[test]
    fn replay_is_required_when_exposure_is_declared() {
        assert!(toml::from_str::<Holder>("peer = {}").is_err());
    }

    #[test]
    fn unknown_replay_value_is_rejected() {
        assert!(toml::from_str::<Holder>("peer = { replay = \"sometimes\" }").is_err());
    }

    #[test]
    fn malformed_or_unknown_exposure_fields_are_rejected() {
        for source in [
            "peer = true",
            "peer = []",
            "peer = { replay = false }",
            "peer = { replay = \"never\", enabled = false }",
            "peer = { replay = \"idempotent\", scope = \"admin\" }",
        ] {
            assert!(toml::from_str::<Holder>(source).is_err(), "{source}");
        }
    }
}
