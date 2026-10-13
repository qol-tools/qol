pub(in crate::shell_link) fn decode_ansi(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}
