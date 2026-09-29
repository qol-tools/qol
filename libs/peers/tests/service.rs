use std::time::{Duration, SystemTime};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use qol_peers::service::{
    validate_certificate, CertificateError, Identity, PeerError, PeerPin, SecretKeyBytes,
};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[test]
fn generation_restore_and_renewal_preserve_spki_identity() {
    let now = SystemTime::now();
    let identity = Identity::generate(now).unwrap();
    let restored = Identity::restore(
        identity.certificate_der().to_vec(),
        identity.export_secret(),
        now,
    )
    .unwrap();
    let renewal_time = now + Duration::from_secs(336 * 24 * 60 * 60);
    assert!(!identity.renewal_due(now).unwrap());
    assert!(identity.renewal_due(renewal_time).unwrap());
    let renewed = restored.renew(renewal_time).unwrap();
    assert_eq!(identity.pin(), restored.pin());
    assert_eq!(identity.pin(), renewed.pin());
    assert_ne!(identity.certificate_der(), renewed.certificate_der());
    assert!(!renewed.renewal_due(renewal_time).unwrap());
    let seconds = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_eq!(
        validate_certificate(identity.certificate_der(), seconds).unwrap(),
        *identity.pin()
    );
    let digest = Sha256::digest(identity.pin().spki_der());
    assert_eq!(
        identity.pin().peer_id().to_string(),
        URL_SAFE_NO_PAD.encode(digest)
    );
    assert_ne!(identity.pin(), Identity::generate(now).unwrap().pin());
}

#[test]
fn pin_rejects_noncanonical_der_and_invalid_curve_points() {
    let identity = Identity::generate(SystemTime::now()).unwrap();
    let spki = identity.pin().spki_der();
    assert_eq!(PeerPin::from_spki_der(spki).unwrap(), *identity.pin());
    let mut trailing = spki.to_vec();
    trailing.push(0);
    let mut curve = spki.to_vec();
    curve[22] ^= 1;
    let mut compressed = spki.to_vec();
    compressed[26] = 2;
    let mut point = spki.to_vec();
    point[27..].fill(0);
    for (name, der) in [
        ("empty", vec![]),
        ("truncated", spki[..90].to_vec()),
        ("trailing", trailing),
        ("curve", curve),
        ("compressed", compressed),
        ("point", point),
    ] {
        assert!(
            matches!(PeerPin::from_spki_der(&der), Err(PeerError::InvalidPin)),
            "case: {name}"
        );
    }
}

#[test]
fn restoration_rejects_key_mismatch_and_redacts_secret_material() {
    let now = SystemTime::now();
    let first = Identity::generate(now).unwrap();
    let second = Identity::generate(now).unwrap();
    let secret = second.export_secret();
    let decimal_secret = format!("{:?}", secret.expose_pkcs8());
    let encoded_secret = URL_SAFE_NO_PAD.encode(secret.expose_pkcs8());
    let debug = format!("{second:?} {secret:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains(&decimal_secret));
    assert!(!debug.contains(&encoded_secret));
    let error = Identity::restore(first.certificate_der().to_vec(), secret, now).unwrap_err();
    assert!(matches!(error, PeerError::KeyMismatch));
    let errors = format!("{error:?} {error}");
    assert!(!errors.contains(&decimal_secret));
    assert!(!errors.contains(&encoded_secret));
    let marker = b"private-material-must-never-appear";
    let invalid = SecretKeyBytes::from_pkcs8(Zeroizing::new(marker.to_vec()));
    let error = Identity::restore(first.certificate_der().to_vec(), invalid, now).unwrap_err();
    assert!(matches!(error, PeerError::PrivateKey));
    assert!(!format!("{error:?} {error}").contains(std::str::from_utf8(marker).unwrap()));
}

#[test]
fn clocks_before_epoch_fail_without_panicking() {
    let before_epoch = SystemTime::UNIX_EPOCH - Duration::from_secs(1);
    assert!(matches!(
        Identity::generate(before_epoch),
        Err(PeerError::Clock)
    ));
    let identity = Identity::generate(SystemTime::now()).unwrap();
    assert!(matches!(
        identity.renew(before_epoch),
        Err(PeerError::Clock)
    ));
    assert!(matches!(
        identity.renewal_due(before_epoch),
        Err(PeerError::Clock)
    ));
}

#[test]
fn certificate_validity_and_renewal_thresholds_are_explicit() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let identity = Identity::generate(now).unwrap();
    let start = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let end = start + 365 * 24 * 60 * 60;
    for (seconds, error) in [
        (start - 1, Some(CertificateError::NotYetValid)),
        (start, None),
        (end - 1, None),
        (end, Some(CertificateError::Expired)),
        (u64::MAX, Some(CertificateError::Validity)),
    ] {
        assert_eq!(
            validate_certificate(identity.certificate_der(), seconds).err(),
            error,
            "seconds: {seconds}"
        );
    }
    let threshold = now + Duration::from_secs(335 * 24 * 60 * 60);
    assert!(!identity
        .renewal_due(threshold - Duration::from_secs(1))
        .unwrap());
    assert!(identity.renewal_due(threshold).unwrap());
}

#[test]
fn local_restore_or_renew_preserves_persisted_identity_across_offline_intervals() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let identity = Identity::generate(now).unwrap();
    let day = 24 * 60 * 60;
    for (elapsed, expected_renewal) in [
        (0, false),
        (335 * day - 1, false),
        (335 * day, true),
        (365 * day - 1, true),
        (365 * day, true),
        (365 * day + 1, true),
        (3 * 365 * day, true),
    ] {
        let time = now + Duration::from_secs(elapsed);
        let (restored, renewed) = Identity::restore_or_renew(
            identity.certificate_der().to_vec(),
            identity.export_secret(),
            time,
        )
        .unwrap();
        assert_eq!(renewed, expected_renewal, "elapsed: {elapsed}");
        assert_eq!(restored.pin(), identity.pin(), "elapsed: {elapsed}");
        assert_eq!(
            restored.export_secret().expose_pkcs8(),
            identity.export_secret().expose_pkcs8(),
            "elapsed: {elapsed}"
        );
        assert_eq!(
            restored.certificate_der() != identity.certificate_der(),
            expected_renewal,
            "elapsed: {elapsed}"
        );
        assert!(!restored.renewal_due(time).unwrap());
        let persisted = Identity::restore(
            restored.certificate_der().to_vec(),
            restored.export_secret(),
            time,
        )
        .unwrap();
        assert_eq!(persisted.pin(), identity.pin(), "elapsed: {elapsed}");
        let (reloaded, changed) = Identity::restore_or_renew(
            persisted.certificate_der().to_vec(),
            persisted.export_secret(),
            time,
        )
        .unwrap();
        assert!(!changed, "elapsed: {elapsed}");
        assert_eq!(reloaded.certificate_der(), persisted.certificate_der());
    }
}

#[test]
fn strict_restore_still_rejects_expired_local_certificates() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let identity = Identity::generate(now).unwrap();
    for days in [365, 366, 3 * 365] {
        let time = now + Duration::from_secs(days * 24 * 60 * 60);
        assert!(matches!(
            Identity::restore(
                identity.certificate_der().to_vec(),
                identity.export_secret(),
                time,
            ),
            Err(PeerError::Certificate(CertificateError::Expired))
        ));
    }
}

#[test]
fn local_restore_or_renew_rejects_invalid_or_mismatched_private_keys() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let identity = Identity::generate(now).unwrap();
    let stranger = Identity::generate(now).unwrap();
    for days in [0, 335, 365, 3 * 365] {
        let time = now + Duration::from_secs(days * 24 * 60 * 60);
        assert!(matches!(
            Identity::restore_or_renew(
                identity.certificate_der().to_vec(),
                stranger.export_secret(),
                time,
            ),
            Err(PeerError::KeyMismatch)
        ));
        assert!(matches!(
            Identity::restore_or_renew(
                identity.certificate_der().to_vec(),
                SecretKeyBytes::from_pkcs8(Zeroizing::new(vec![0; 32])),
                time,
            ),
            Err(PeerError::PrivateKey)
        ));
    }
}

#[test]
fn local_restore_or_renew_rejects_future_certificates_and_invalid_clocks() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let identity = Identity::generate(now).unwrap();
    assert!(matches!(
        Identity::restore_or_renew(
            identity.certificate_der().to_vec(),
            identity.export_secret(),
            now - Duration::from_secs(1),
        ),
        Err(PeerError::Certificate(CertificateError::NotYetValid))
    ));
    let last_day = u64::try_from(time::Date::MAX.midnight().assume_utc().unix_timestamp()).unwrap();
    let unissuable = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(last_day));
    for time in std::iter::once(SystemTime::UNIX_EPOCH - Duration::from_secs(1)).chain(unissuable) {
        assert!(matches!(
            Identity::restore_or_renew(
                identity.certificate_der().to_vec(),
                identity.export_secret(),
                time,
            ),
            Err(PeerError::Clock)
        ));
    }
}
