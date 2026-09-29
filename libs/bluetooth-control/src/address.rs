#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("invalid Bluetooth address `{0}`; expected AA:BB:CC:DD:EE:FF")]
pub struct AddressError(pub String);

pub fn normalize_address(value: &str) -> Result<String, AddressError> {
    let parts = value.trim().split(':').collect::<Vec<_>>();
    if parts.len() != 6
        || parts
            .iter()
            .any(|part| part.len() != 2 || !part.chars().all(|char| char.is_ascii_hexdigit()))
    {
        return Err(AddressError(value.to_string()));
    }
    Ok(parts.join(":").to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_six_colon_separated_hex_octets_name_a_peripheral() {
        assert_eq!(
            normalize_address(" aa:bb:cc:dd:ee:0f ").unwrap(),
            "AA:BB:CC:DD:EE:0F"
        );
        for invalid in [
            "",
            "AA:BB:CC:DD:EE",
            "AA:BB:CC:DD:EE:GG",
            "AA-BB-CC-DD-EE-FF",
        ] {
            assert!(normalize_address(invalid).is_err(), "{invalid}");
        }
    }
}
