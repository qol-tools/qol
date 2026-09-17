pub(crate) struct SinkRow {
    index: u32,
    name: String,
    state: String,
}

impl SinkRow {
    pub(crate) fn index(&self) -> u32 {
        self.index
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn running(&self) -> bool {
        self.state == "RUNNING"
    }
}

pub(crate) fn parse_short_sinks(listing: &[u8]) -> Vec<SinkRow> {
    String::from_utf8_lossy(listing)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let index = fields.next()?.parse::<u32>().ok()?;
            let name = fields.next()?.to_string();
            let state = fields.last()?.to_string();
            Some(SinkRow { index, name, state })
        })
        .collect()
}

pub(crate) fn running_sink_except(rows: &[SinkRow], prefix: &str) -> Option<String> {
    rows.iter()
        .find(|row| row.running() && !row.name().starts_with(prefix))
        .map(|row| row.name().to_string())
}

#[cfg(test)]
mod tests {
    use super::{parse_short_sinks, running_sink_except};

    const OWN: &str = "bluez_output.74_68_59_7F_5F_E9";
    const MIXED: &[u8] = b"53\talsa_output.pci.analog-stereo\tPipeWire\ts32le 2ch\tSUSPENDED\n5\tRUNNING\nx\tbluez_output.74_68_59_7F_5F_E9.1\tRUNNING\n\n8842\tbluez_output.88_0E_85_16_CA_67.1\tPipeWire\ts16le 2ch\tRUNNING\n";
    const LOWERCASE: &[u8] =
        b"75\tbluez_output.74_68_59_7F_5F_E9.1\tPipeWire\ts16le 2ch\trunning\n";

    #[test]
    fn short_sink_rows_keep_numeric_indexed_rows_in_listing_order() {
        let rows = parse_short_sinks(MIXED);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].index(), 53);
        assert_eq!(rows[0].name(), "alsa_output.pci.analog-stereo");
        assert!(!rows[0].running());
        assert_eq!(rows[1].index(), 8842);
        assert_eq!(rows[1].name(), "bluez_output.88_0E_85_16_CA_67.1");
        assert!(rows[1].running());
    }

    #[test]
    fn running_sink_state_is_case_sensitive() {
        let rows = parse_short_sinks(LOWERCASE);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].running());
    }

    #[test]
    fn running_sink_except_ignores_own_prefix_and_non_running_rows() {
        let cases = [
            ("", None),
            (
                "53\talsa_output.pci.analog-stereo\tPipeWire\ts32le 2ch\tSUSPENDED\n72\talsa_output.pci.hdmi-stereo\tPipeWire\ts32le 2ch\tIDLE\n",
                None,
            ),
            (
                "75\tbluez_output.74_68_59_7F_5F_E9.1\tPipeWire\ts16le 2ch\tRUNNING\n",
                None,
            ),
            (
                "75\tbluez_output.74_68_59_7F_5F_E9.1\tPipeWire\ts16le 2ch\tRUNNING\n8842\tbluez_output.74_68_59_7F_5F_E0.1\tPipeWire\ts16le 2ch\tRUNNING\n",
                Some("bluez_output.74_68_59_7F_5F_E0.1"),
            ),
            (
                "58\talsa_output.usb-hyperx.analog-stereo\tPipeWire\ts24le 2ch\tRUNNING\n75\tbluez_output.74_68_59_7F_5F_E9.1\tPipeWire\ts16le 2ch\tSUSPENDED\n",
                Some("alsa_output.usb-hyperx.analog-stereo"),
            ),
            (
                "75\tbluez_output.74_68_59_7F_5F_E9.1\tPipeWire\ts16le 2ch\trunning\n",
                None,
            ),
            ("5\tRUNNING\n", None),
        ];
        for (listing, expected) in cases {
            let rows = parse_short_sinks(listing.as_bytes());
            assert_eq!(
                running_sink_except(&rows, OWN).as_deref(),
                expected,
                "listing: {listing}"
            );
        }
    }
}
