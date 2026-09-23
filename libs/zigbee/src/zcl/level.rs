use super::ZclFrame;

pub fn move_to_level(level: u8, transition_tenths: u16) -> ZclFrame {
    let mut payload = Vec::with_capacity(3);
    payload.push(level);
    payload.extend_from_slice(&transition_tenths.to_le_bytes());
    ZclFrame::cluster_command(0x04, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_to_level_encoding() {
        let frame = move_to_level(0x80, 0x000A);
        assert_eq!(frame.command_id, 0x04, "command_id MoveToLevelWithOnOff");
        let encoded = frame.encode();
        assert_eq!(encoded[3], 0x80, "level byte");
        assert_eq!(encoded[4], 0x0A, "transition low byte");
        assert_eq!(encoded[5], 0x00, "transition high byte");
    }

    #[test]
    fn level_boundary_values() {
        let cases: &[(u8, u16)] = &[(0, 0), (254, 0), (128, 0xFFFF)];
        for (level, trans) in cases {
            let frame = move_to_level(*level, *trans);
            let encoded = frame.encode();
            assert_eq!(
                encoded[3], *level,
                "level={level} trans={trans}: level byte"
            );
            let trans_bytes = trans.to_le_bytes();
            assert_eq!(
                encoded[4], trans_bytes[0],
                "level={level} trans={trans}: low byte"
            );
            assert_eq!(
                encoded[5], trans_bytes[1],
                "level={level} trans={trans}: high byte"
            );
        }
    }
}
