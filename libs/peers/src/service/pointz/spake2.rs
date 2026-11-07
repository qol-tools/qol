use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
use curve25519_dalek::edwards::{CompressedEdwardsY, EdwardsPoint};
use curve25519_dalek::scalar::Scalar;
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256, Sha512};
use zeroize::Zeroizing;

type HmacSha256 = Hmac<Sha256>;

const PASSWORD_LABEL: &[u8] = b"pointz-spake2-v2";
const CONFIRMATION_INFO: &[u8] = b"ConfirmationKeys";
const WRAP_INFO: &[u8] = b"pointz-pair-wrap-v2";

const MASK_M: [u8; 32] = [
    0xd0, 0x48, 0x03, 0x2c, 0x6e, 0xa0, 0xb6, 0xd6, 0x97, 0xdd, 0xc2, 0xe8, 0x6b, 0xda, 0x85, 0xa3,
    0x3a, 0xda, 0xc9, 0x20, 0xf1, 0xbf, 0x18, 0xe1, 0xb0, 0xc6, 0xd1, 0x66, 0xa5, 0xce, 0xcd, 0xaf,
];
const MASK_N: [u8; 32] = [
    0xd3, 0xbf, 0xb5, 0x18, 0xf4, 0x4f, 0x34, 0x30, 0xf2, 0x9d, 0x0c, 0x92, 0xaf, 0x50, 0x38, 0x65,
    0xa1, 0xed, 0x32, 0x81, 0xdc, 0x69, 0xb3, 0x5d, 0xd8, 0x68, 0xba, 0x85, 0xf8, 0x86, 0xc4, 0xab,
];

pub(super) struct Responder {
    pub(super) pb: [u8; 32],
    pub(super) confirm_a: [u8; 32],
    pub(super) confirm_b: [u8; 32],
    pub(super) k_wrap: Zeroizing<[u8; 32]>,
}

pub(super) fn random_scalar() -> Option<Scalar> {
    let wide = crate::service::enrollment::random::<64>().ok()?;
    Some(Scalar::from_bytes_mod_order_wide(&wide))
}

pub(super) fn password_scalar(pin: &str, device_id: &[u8; 16], server_id: &str) -> Scalar {
    let mut input =
        Vec::with_capacity(PASSWORD_LABEL.len() + 24 + pin.len() + 16 + server_id.len());
    input.extend_from_slice(PASSWORD_LABEL);
    push_prefixed(&mut input, pin.as_bytes());
    push_prefixed(&mut input, device_id);
    push_prefixed(&mut input, server_id.as_bytes());
    let digest = Sha512::digest(&input);
    let mut wide = [0u8; 64];
    wide.copy_from_slice(&digest);
    Scalar::from_bytes_mod_order_wide(&wide)
}

pub(super) fn responder_share(w: &Scalar, y: &Scalar) -> [u8; 32] {
    let share = ED25519_BASEPOINT_POINT * y + mask(&MASK_N) * w;
    share.compress().to_bytes()
}

pub(super) fn accept_share(bytes: &[u8; 32]) -> Option<EdwardsPoint> {
    let point = CompressedEdwardsY(*bytes).decompress()?;
    if point.is_small_order() {
        return None;
    }
    Some(point)
}

pub(super) fn shared_point(y: &Scalar, w: &Scalar, pa: &EdwardsPoint) -> [u8; 32] {
    let unmasked = pa - mask(&MASK_M) * w;
    (unmasked * y).mul_by_cofactor().compress().to_bytes()
}

pub(super) fn transcript(
    device_id: &[u8; 16],
    server_id: &str,
    pa: &[u8; 32],
    pb: &[u8; 32],
    shared: &[u8; 32],
    w: &Scalar,
) -> Vec<u8> {
    let mut transcript = Vec::with_capacity(8 * 6 + 16 + server_id.len() + 32 * 4);
    push_prefixed(&mut transcript, device_id);
    push_prefixed(&mut transcript, server_id.as_bytes());
    push_prefixed(&mut transcript, pa);
    push_prefixed(&mut transcript, pb);
    push_prefixed(&mut transcript, shared);
    push_prefixed(&mut transcript, &w.to_bytes());
    transcript
}

pub(super) fn session_keys(transcript: &[u8]) -> ([u8; 16], [u8; 16]) {
    let digest = Sha256::digest(transcript);
    split_halves(&digest)
}

pub(super) fn confirmation_keys(ka: &[u8; 16]) -> ([u8; 16], [u8; 16]) {
    let mut okm = [0u8; 32];
    Hkdf::<Sha256>::new(None, ka)
        .expand(CONFIRMATION_INFO, &mut okm)
        .expect("32-byte OKM is valid for HKDF-SHA256");
    split_halves(&okm)
}

pub(super) fn confirmation_tag(key: &[u8; 16], transcript: &[u8]) -> [u8; 32] {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(transcript);
    mac.finalize().into_bytes().into()
}

pub(super) fn wrap_key(ke: &[u8; 16], salt: &[u8; 16]) -> Zeroizing<[u8; 32]> {
    let mut okm = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(Some(salt), ke)
        .expand(WRAP_INFO, &mut *okm)
        .expect("32-byte OKM is valid for HKDF-SHA256");
    okm
}

pub(super) fn respond(
    y: &Scalar,
    pin: &str,
    device_id: &[u8; 16],
    server_id: &str,
    pa: &[u8; 32],
    salt: &[u8; 16],
) -> Option<Responder> {
    let pa_point = accept_share(pa)?;
    let w = password_scalar(pin, device_id, server_id);
    let pb = responder_share(&w, y);
    let shared = shared_point(y, &w, &pa_point);
    let transcript = transcript(device_id, server_id, pa, &pb, &shared, &w);
    let (ke, ka) = session_keys(&transcript);
    let (kc_a, kc_b) = confirmation_keys(&ka);
    Some(Responder {
        pb,
        confirm_a: confirmation_tag(&kc_a, &transcript),
        confirm_b: confirmation_tag(&kc_b, &transcript),
        k_wrap: wrap_key(&ke, salt),
    })
}

fn mask(encoded: &[u8; 32]) -> EdwardsPoint {
    CompressedEdwardsY(*encoded)
        .decompress()
        .expect("RFC 9382 mask constants are valid edwards25519 points")
}

fn push_prefixed(out: &mut Vec<u8>, part: &[u8]) {
    out.extend_from_slice(&(part.len() as u64).to_le_bytes());
    out.extend_from_slice(part);
}

fn split_halves(bytes: &[u8]) -> ([u8; 16], [u8; 16]) {
    let mut first = [0u8; 16];
    let mut second = [0u8; 16];
    first.copy_from_slice(&bytes[..16]);
    second.copy_from_slice(&bytes[16..32]);
    (first, second)
}

#[cfg(test)]
mod tests {
    use super::super::vector::{self, hex, hex_array};
    use super::*;

    fn y() -> Scalar {
        Scalar::from_bytes_mod_order(vector::Y)
    }

    #[test]
    fn the_mask_constants_are_the_rfc_9382_points() {
        assert_eq!(
            MASK_M.to_vec(),
            hex("d048032c6ea0b6d697ddc2e86bda85a33adac920f1bf18e1b0c6d166a5cecdaf")
        );
        assert_eq!(
            MASK_N.to_vec(),
            hex("d3bfb518f44f3430f29d0c92af503865a1ed3281dc69b35dd868ba85f886c4ab")
        );
        assert!(mask(&MASK_M).is_torsion_free());
        assert!(mask(&MASK_N).is_torsion_free());
    }

    #[test]
    fn the_desktop_role_reproduces_every_value_of_the_shared_vector() {
        let w = password_scalar(vector::PIN, &vector::DEVICE, vector::SERVER_ID);
        assert_eq!(w.to_bytes(), hex_array::<32>(vector::W));

        let pb = responder_share(&w, &y());
        assert_eq!(pb, hex_array::<32>(vector::PB));

        let pa = hex_array::<32>(vector::PA);
        let shared = shared_point(&y(), &w, &accept_share(&pa).unwrap());
        assert_eq!(shared, hex_array::<32>(vector::K));

        let transcript = transcript(&vector::DEVICE, vector::SERVER_ID, &pa, &pb, &shared, &w);
        assert_eq!(transcript, hex(vector::TT));

        let (ke, ka) = session_keys(&transcript);
        assert_eq!(ke, hex_array::<16>(vector::KE));
        assert_eq!(ka, hex_array::<16>(vector::KA));

        let (kc_a, kc_b) = confirmation_keys(&ka);
        assert_eq!(kc_a, hex_array::<16>(vector::KC_A));
        assert_eq!(kc_b, hex_array::<16>(vector::KC_B));
        assert_eq!(
            confirmation_tag(&kc_a, &transcript),
            hex_array::<32>(vector::CONFIRM_A)
        );
        assert_eq!(
            confirmation_tag(&kc_b, &transcript),
            hex_array::<32>(vector::CONFIRM_B)
        );
        assert_eq!(
            *wrap_key(&ke, &vector::SALT),
            hex_array::<32>(vector::K_WRAP)
        );
    }

    #[test]
    fn respond_composes_the_vector_values() {
        let responder = respond(
            &y(),
            vector::PIN,
            &vector::DEVICE,
            vector::SERVER_ID,
            &hex_array::<32>(vector::PA),
            &vector::SALT,
        )
        .unwrap();

        assert_eq!(responder.pb, hex_array::<32>(vector::PB));
        assert_eq!(responder.confirm_a, hex_array::<32>(vector::CONFIRM_A));
        assert_eq!(responder.confirm_b, hex_array::<32>(vector::CONFIRM_B));
        assert_eq!(*responder.k_wrap, hex_array::<32>(vector::K_WRAP));
    }

    #[test]
    fn shares_that_are_the_identity_or_small_order_are_refused() {
        let identity = [
            1u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0,
        ];
        let order_two = [
            0xecu8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0x7f,
        ];
        let order_four = [0u8; 32];
        let cases = [
            ("identity", identity),
            ("order two", order_two),
            ("order four", order_four),
        ];
        for (name, encoded) in cases {
            assert!(accept_share(&encoded).is_none(), "{name}");
        }
        assert!(accept_share(&hex_array::<32>(vector::PA)).is_some());
    }

    #[test]
    fn a_different_pin_changes_every_derived_value() {
        let right = respond(
            &y(),
            vector::PIN,
            &vector::DEVICE,
            vector::SERVER_ID,
            &hex_array::<32>(vector::PA),
            &vector::SALT,
        )
        .unwrap();
        let wrong = respond(
            &y(),
            "654321",
            &vector::DEVICE,
            vector::SERVER_ID,
            &hex_array::<32>(vector::PA),
            &vector::SALT,
        )
        .unwrap();

        assert_ne!(right.pb, wrong.pb);
        assert_ne!(right.confirm_a, wrong.confirm_a);
        assert_ne!(*right.k_wrap, *wrong.k_wrap);
    }
}
