use super::ZclFrame;

pub fn off() -> ZclFrame {
    ZclFrame::cluster_command(0x00, vec![])
}

pub fn on() -> ZclFrame {
    ZclFrame::cluster_command(0x01, vec![])
}

pub fn toggle() -> ZclFrame {
    ZclFrame::cluster_command(0x02, vec![])
}

#[cfg(test)]
mod tests {
    use super::*;

    type OnOffCase = (&'static str, fn() -> ZclFrame, u8);

    #[test]
    fn on_off_commands() {
        let cases: &[OnOffCase] = &[
            ("off", off, 0x00),
            ("on", on, 0x01),
            ("toggle", toggle, 0x02),
        ];
        for (name, build, expected_cmd) in cases {
            let frame = build();
            assert_eq!(frame.fcf, 0x01, "{name}: FCF must be cluster-specific");
            assert_eq!(frame.command_id, *expected_cmd, "{name}: command_id");
            assert!(frame.payload.is_empty(), "{name}: payload must be empty");
        }
    }
}
