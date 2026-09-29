use std::time::{Duration, SystemTime};

use rcgen::{
    BasicConstraints, CertificateParams, CustomExtension, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose,
};
use rustls::pki_types::CertificateDer;
use time::OffsetDateTime;
use zeroize::Zeroizing;

use super::{raw_handshake, tls_error, Fixture};
use crate::service::{
    validate_certificate, CertificateError, Identity, PeerError, SecretKeyBytes,
    MAX_CERTIFICATE_BYTES,
};

struct Rejection {
    name: &'static str,
    certificate: Vec<u8>,
    error: CertificateError,
}

type ParameterCase = (&'static str, fn(&mut CertificateParams), CertificateError);

fn params() -> CertificateParams {
    let mut params = CertificateParams::new(vec!["qol-peer.invalid".to_owned()]).unwrap();
    let now = OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::minutes(1);
    params.not_after = now + time::Duration::hours(1);
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ServerAuth,
        ExtendedKeyUsagePurpose::ClientAuth,
    ];
    params
}

fn rejections(key: &KeyPair) -> Vec<Rejection> {
    let settings: [ParameterCase; 13] = [
        (
            "missing key usage",
            |p| p.key_usages.clear(),
            CertificateError::Usage,
        ),
        (
            "missing extended usages",
            |p| p.extended_key_usages.clear(),
            CertificateError::Usage,
        ),
        (
            "missing client usage",
            |p| p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth],
            CertificateError::Usage,
        ),
        (
            "missing server usage",
            |p| p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth],
            CertificateError::Usage,
        ),
        (
            "CA",
            |p| p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained),
            CertificateError::CertificateAuthority,
        ),
        (
            "expired",
            |p| {
                p.not_before -= time::Duration::hours(2);
                p.not_after -= time::Duration::hours(2);
            },
            CertificateError::Expired,
        ),
        (
            "future",
            |p| p.not_before += time::Duration::minutes(2),
            CertificateError::NotYetValid,
        ),
        (
            "excessive validity",
            |p| p.not_after += time::Duration::days(366),
            CertificateError::Validity,
        ),
        (
            "missing SAN",
            |p| p.subject_alt_names.clear(),
            CertificateError::Name,
        ),
        (
            "duplicate extension",
            |p| {
                p.custom_extensions.push(CustomExtension::from_oid_content(
                    &[2, 5, 29, 15],
                    vec![3, 2, 7, 128],
                ))
            },
            CertificateError::Extensions,
        ),
        (
            "malformed extension",
            |p| {
                p.key_usages.clear();
                p.custom_extensions.push(CustomExtension::from_oid_content(
                    &[2, 5, 29, 15],
                    vec![255],
                ));
            },
            CertificateError::Extensions,
        ),
        (
            "trailing extension DER",
            |p| {
                p.key_usages.clear();
                p.custom_extensions.push(CustomExtension::from_oid_content(
                    &[2, 5, 29, 15],
                    vec![3, 2, 7, 128, 0],
                ));
            },
            CertificateError::Extensions,
        ),
        (
            "unknown critical extension",
            |p| {
                let mut extension = CustomExtension::from_oid_content(&[1, 2, 3, 4], vec![5, 0]);
                extension.set_criticality(true);
                p.custom_extensions.push(extension);
            },
            CertificateError::Extensions,
        ),
    ];
    let mut cases = Vec::new();
    for (name, mutate, error) in settings {
        let mut params = params();
        mutate(&mut params);
        cases.push(Rejection {
            name,
            certificate: params.self_signed(key).unwrap().der().to_vec(),
            error,
        });
    }
    for (name, algorithm) in [
        ("P-384", &rcgen::PKCS_ECDSA_P384_SHA384),
        ("Ed25519", &rcgen::PKCS_ED25519),
    ] {
        let key = Zeroizing::new(KeyPair::generate_for(algorithm).unwrap());
        cases.push(Rejection {
            name,
            certificate: params().self_signed(&*key).unwrap().der().to_vec(),
            error: CertificateError::Algorithm,
        });
    }
    let valid = params().self_signed(key).unwrap().der().to_vec();
    let mut trailing = valid.clone();
    trailing.push(0);
    let mut signature = valid.clone();
    *signature.last_mut().unwrap() ^= 1;
    let mut point = valid.clone();
    let mut ber_boolean = valid.clone();
    let critical_key_usage = [6, 3, 0x55, 0x1d, 0x0f, 1, 1, 0xff];
    let critical_offset = ber_boolean
        .windows(critical_key_usage.len())
        .position(|window| window == critical_key_usage)
        .unwrap();
    ber_boolean[critical_offset + 7] = 1;
    let pin = validate_certificate(
        &valid,
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap();
    let offset = point
        .windows(91)
        .position(|window| window == pin.spki_der())
        .unwrap();
    point[offset + 27..offset + 91].fill(0);
    cases.extend([
        Rejection {
            name: "trailing DER",
            certificate: trailing,
            error: CertificateError::Encoding,
        },
        Rejection {
            name: "truncated DER",
            certificate: valid[..valid.len() - 1].to_vec(),
            error: CertificateError::Encoding,
        },
        Rejection {
            name: "malformed DER",
            certificate: vec![48, 255, 0],
            error: CertificateError::Encoding,
        },
        Rejection {
            name: "oversized",
            certificate: vec![0; MAX_CERTIFICATE_BYTES + 1],
            error: CertificateError::TooLarge,
        },
        Rejection {
            name: "invalid self-signature",
            certificate: signature,
            error: CertificateError::Signature,
        },
        Rejection {
            name: "off-curve point",
            certificate: point,
            error: CertificateError::Algorithm,
        },
        Rejection {
            name: "BER critical boolean",
            certificate: ber_boolean,
            error: CertificateError::Encoding,
        },
    ]);
    cases
}

#[tokio::test]
async fn certificate_matrix_rejects_at_parser_and_both_handshake_boundaries() {
    let fixture = Fixture::new();
    let key = Zeroizing::new(KeyPair::generate().unwrap());
    for case in rejections(&key) {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(
            validate_certificate(&case.certificate, now),
            Err(case.error),
            "case: {}",
            case.name
        );
        let chain = vec![CertificateDer::from(case.certificate)];
        let (_, server) = raw_handshake(
            fixture.client_with_chain(chain.clone()),
            fixture.server_config.0.clone(),
        )
        .await;
        assert_certificate_error(server.expect_err(case.name), case.error, case.name);
        let (client, _) = raw_handshake(
            fixture.client_config.0.clone(),
            fixture.server_with_chain(chain),
        )
        .await;
        assert_certificate_error(client.expect_err(case.name), case.error, case.name);
    }
}

#[test]
fn local_recovery_preserves_certificate_profile_pin_and_signature_checks() {
    let key = Zeroizing::new(KeyPair::generate().unwrap());
    let now = SystemTime::now();
    let offline = now + Duration::from_secs(2 * 365 * 24 * 60 * 60);
    for case in rejections(&key) {
        for validation_time in [now, offline] {
            let result = Identity::restore_or_renew(
                case.certificate.clone(),
                SecretKeyBytes::from_pkcs8(Zeroizing::new(key.serialize_der())),
                validation_time,
            );
            if case.error == CertificateError::Expired
                || (case.error == CertificateError::NotYetValid && validation_time == offline)
            {
                let (identity, renewed) = result.expect(case.name);
                assert!(renewed, "case: {}", case.name);
                let seconds = validation_time
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                assert_eq!(
                    validate_certificate(identity.certificate_der(), seconds).unwrap(),
                    *identity.pin(),
                    "case: {}",
                    case.name
                );
                continue;
            }
            let error = result.expect_err(case.name);
            assert!(
                matches!(error, PeerError::Certificate(actual) if actual == case.error),
                "case: {}, time: {validation_time:?}, error: {error:?}",
                case.name
            );
        }
    }
}

#[tokio::test]
async fn extra_certificate_chains_are_rejected_in_both_directions() {
    let fixture = Fixture::new();
    for length in [2, 8] {
        let client_chain =
            vec![CertificateDer::from(fixture.client.certificate_der().to_vec()); length];
        let (_, server) = raw_handshake(
            fixture.client_with_chain(client_chain),
            fixture.server_config.0.clone(),
        )
        .await;
        assert_certificate_error(
            server.err().unwrap(),
            CertificateError::Chain,
            "client extra chain",
        );
        let server_chain =
            vec![CertificateDer::from(fixture.server.certificate_der().to_vec()); length];
        let (client, _) = raw_handshake(
            fixture.client_config.0.clone(),
            fixture.server_with_chain(server_chain),
        )
        .await;
        assert_certificate_error(
            client.err().unwrap(),
            CertificateError::Chain,
            "server extra chain",
        );
    }
}

fn assert_certificate_error(error: std::io::Error, expected: CertificateError, case: &str) {
    let rustls::Error::InvalidCertificate(rustls::CertificateError::Other(inner)) =
        tls_error(&error)
    else {
        panic!("case: {case}, unexpected TLS error: {error:?}");
    };
    assert_eq!(
        inner.0.downcast_ref::<CertificateError>(),
        Some(&expected),
        "case: {case}"
    );
}
