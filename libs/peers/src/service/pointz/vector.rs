pub(super) const PIN: &str = "123456";
pub(super) const DEVICE: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
];
pub(super) const SERVER_ID: &str = "srv_test_server_id";
pub(super) const X: [u8; 32] = [0x0a; 32];
pub(super) const Y: [u8; 32] = [0x0b; 32];
pub(super) const SALT: [u8; 16] = [0x0c; 16];

pub(super) const W: &str = "e4b7db0584fedda97976ce77dbb049a49abd89ebbcd04705cc40851e4fefe308";
pub(super) const PA: &str = "c2619681afa765c1c73d952ec776d370ccbd7f3e5c64a78383cb459b95f10624";
pub(super) const PB: &str = "cf7d76f2a0091c706bd51ed10439558590f3031f5a2fd262521314f5995e170f";
pub(super) const K: &str = "9f4bdc7c8bc6bda13ca065c6f37b095f9bc63ed718282363408739d93d11da55";
pub(super) const TT: &str = "10000000000000000102030405060708090a0b0c0d0e0f1012000000000000007372765f746573745f7365727665725f69642000000000000000c2619681afa765c1c73d952ec776d370ccbd7f3e5c64a78383cb459b95f106242000000000000000cf7d76f2a0091c706bd51ed10439558590f3031f5a2fd262521314f5995e170f20000000000000009f4bdc7c8bc6bda13ca065c6f37b095f9bc63ed718282363408739d93d11da552000000000000000e4b7db0584fedda97976ce77dbb049a49abd89ebbcd04705cc40851e4fefe308";
pub(super) const KE: &str = "652524b5c2bb5f89a4d46732a6dec913";
pub(super) const KA: &str = "c488b10a9bf8799fa0b8dded00af3584";
pub(super) const KC_A: &str = "2092c35cbe36078612c86bd7a224f74a";
pub(super) const KC_B: &str = "a8edbd16895e7f91a4fa4eadf9fe8a09";
pub(super) const CONFIRM_A: &str =
    "a6e21298be44629b99c11163571612e0b6588586855d81b7d07363e6b86a092d";
pub(super) const CONFIRM_B: &str =
    "dadc8c80118a7ddcf2336b666596a6b604317a73cabfb8c7fd9ab1ed8308c2bd";
pub(super) const K_WRAP: &str = "7db26266ef6a13aad8b06502c8e21b8a13fe0c058cbec9dfcdd3f8115625268d";

pub(super) fn hex(encoded: &str) -> Vec<u8> {
    (0..encoded.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&encoded[index..index + 2], 16).unwrap())
        .collect()
}

pub(super) fn hex_array<const N: usize>(encoded: &str) -> [u8; N] {
    hex(encoded).try_into().unwrap()
}
