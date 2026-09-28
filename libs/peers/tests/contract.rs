use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use qol_peers::PeerId;

#[test]
fn peer_id_json_round_trips_all_digest_bytes() {
    for byte in 0..=255 {
        let digest = [byte; 32];
        let encoded = URL_SAFE_NO_PAD.encode(digest);
        let id: PeerId = encoded.parse().unwrap();
        assert_eq!(id.as_bytes(), &digest, "byte: {byte}");
        assert_eq!(id.to_string(), encoded, "byte: {byte}");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{encoded}\""), "byte: {byte}");
        assert_eq!(
            serde_json::from_str::<PeerId>(&json).unwrap(),
            id,
            "byte: {byte}"
        );
    }
}

#[test]
fn peer_id_rejects_noncanonical_or_wrong_length_json() {
    let canonical = URL_SAFE_NO_PAD.encode([0; 32]);
    let cases = [
        String::new(),
        URL_SAFE_NO_PAD.encode([0; 31]),
        URL_SAFE_NO_PAD.encode([0; 33]),
        format!("{canonical}="),
        format!(" {canonical}"),
        format!("{canonical}\n"),
        format!("{}B", &canonical[..42]),
        "+".repeat(43),
        "/".repeat(43),
        "é".repeat(43),
    ];
    for value in cases {
        assert!(value.parse::<PeerId>().is_err(), "value: {value:?}");
        let json = serde_json::to_string(&value).unwrap();
        assert!(
            serde_json::from_str::<PeerId>(&json).is_err(),
            "value: {value:?}"
        );
    }
    for json in ["null", "32", "[]", "{}"] {
        assert!(
            serde_json::from_str::<PeerId>(json).is_err(),
            "json: {json}"
        );
    }
}
