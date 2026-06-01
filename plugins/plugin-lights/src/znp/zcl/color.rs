use super::ZclFrame;

pub fn move_to_hue_sat(hue: u8, saturation: u8, transition_tenths: u16) -> ZclFrame {
    let mut payload = Vec::with_capacity(4);
    payload.push(hue);
    payload.push(saturation);
    payload.extend_from_slice(&transition_tenths.to_le_bytes());
    ZclFrame::cluster_command(0x06, payload)
}

pub fn move_to_color(color_x: u16, color_y: u16, transition_tenths: u16) -> ZclFrame {
    let mut payload = Vec::with_capacity(6);
    payload.extend_from_slice(&color_x.to_le_bytes());
    payload.extend_from_slice(&color_y.to_le_bytes());
    payload.extend_from_slice(&transition_tenths.to_le_bytes());
    ZclFrame::cluster_command(0x07, payload)
}

pub fn move_to_color_temp(mirek: u16, transition_tenths: u16) -> ZclFrame {
    let mut payload = Vec::with_capacity(4);
    payload.extend_from_slice(&mirek.to_le_bytes());
    payload.extend_from_slice(&transition_tenths.to_le_bytes());
    ZclFrame::cluster_command(0x0A, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_to_hue_sat_encoding() {
        let frame = move_to_hue_sat(0xC0, 0xFF, 0x0005);
        assert_eq!(frame.command_id, 0x06, "command_id MoveToHueSat");
        let encoded = frame.encode();
        assert_eq!(encoded[3], 0xC0, "hue byte");
        assert_eq!(encoded[4], 0xFF, "saturation byte");
        assert_eq!(encoded[5], 0x05, "transition low byte");
        assert_eq!(encoded[6], 0x00, "transition high byte");
    }

    #[test]
    fn move_to_color_temp_encoding() {
        let frame = move_to_color_temp(0x00FA, 0x000A);
        assert_eq!(frame.command_id, 0x0A, "command_id MoveToColorTemp");
        let encoded = frame.encode();
        let mirek_bytes = 0x00FAu16.to_le_bytes();
        assert_eq!(encoded[3], mirek_bytes[0], "mirek low byte");
        assert_eq!(encoded[4], mirek_bytes[1], "mirek high byte");
        assert_eq!(encoded[5], 0x0A, "transition low byte");
        assert_eq!(encoded[6], 0x00, "transition high byte");
    }
}
