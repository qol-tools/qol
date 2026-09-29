use x509_cert::der::{Decode, Encode};
use x509_parser::{
    extensions::{GeneralName, ParsedExtension},
    prelude::{FromDer, X509Certificate, X509Version},
};

use super::{CertificateError, PeerPin};

pub const MAX_CERTIFICATE_BYTES: usize = 4096;
pub(crate) const CERTIFICATE_LIFETIME_SECONDS: i64 = 365 * 24 * 60 * 60;
pub(crate) const PEER_NAME: &str = "qol-peer.invalid";

pub fn validate_certificate(der: &[u8], now: u64) -> Result<PeerPin, CertificateError> {
    let certificate = parse_certificate(der)?;
    validate_profile(&certificate)?;
    validate_time(&certificate, now)?;
    let pin = PeerPin::from_spki_der(certificate.public_key().raw)
        .map_err(|_| CertificateError::Algorithm)?;
    certificate
        .verify_signature(None)
        .map_err(|_| CertificateError::Signature)?;
    Ok(pin)
}

pub(crate) fn parse_certificate(der: &[u8]) -> Result<X509Certificate<'_>, CertificateError> {
    if der.len() > MAX_CERTIFICATE_BYTES {
        return Err(CertificateError::TooLarge);
    }
    canonical_der::<x509_cert::Certificate>(der).map_err(|_| CertificateError::Encoding)?;
    let (remaining, certificate) =
        X509Certificate::from_der(der).map_err(|_| CertificateError::Encoding)?;
    if !remaining.is_empty() {
        return Err(CertificateError::Encoding);
    }
    Ok(certificate)
}

fn validate_profile(certificate: &X509Certificate<'_>) -> Result<(), CertificateError> {
    if certificate.version() != X509Version::V3
        || certificate.issuer_uid.is_some()
        || certificate.subject_uid.is_some()
        || certificate.signature_value.unused_bits != 0
    {
        return Err(CertificateError::Encoding);
    }
    if certificate.signature_algorithm.algorithm.to_id_string() != "1.2.840.10045.4.3.2"
        || certificate.signature_algorithm.parameters.is_some()
        || certificate.signature_algorithm != certificate.tbs_certificate.signature
    {
        return Err(CertificateError::Algorithm);
    }
    if certificate.issuer() != certificate.subject() {
        return Err(CertificateError::Signature);
    }
    certificate
        .extensions_map()
        .map_err(|_| CertificateError::Extensions)?;
    for extension in certificate.extensions() {
        let parsed = extension.parsed_extension();
        if matches!(
            parsed,
            ParsedExtension::ParseError { .. } | ParsedExtension::Unparsed
        ) {
            return Err(CertificateError::Extensions);
        }
        let profile_extension = matches!(
            parsed,
            ParsedExtension::BasicConstraints(_)
                | ParsedExtension::KeyUsage(_)
                | ParsedExtension::ExtendedKeyUsage(_)
                | ParsedExtension::SubjectAlternativeName(_)
        );
        let identifier_extension = matches!(
            parsed,
            ParsedExtension::SubjectKeyIdentifier(_) | ParsedExtension::AuthorityKeyIdentifier(_)
        );
        if !profile_extension && (!identifier_extension || extension.critical) {
            return Err(CertificateError::Extensions);
        }
        validate_extension_der(parsed, extension.value)?;
        if matches!(parsed, ParsedExtension::KeyUsage(_)) && extension.value != [3, 2, 7, 128] {
            return Err(CertificateError::Usage);
        }
    }
    validate_usages(certificate)?;
    let san = certificate
        .subject_alternative_name()
        .map_err(|_| CertificateError::Extensions)?
        .ok_or(CertificateError::Name)?;
    if !matches!(
        san.value.general_names.as_slice(),
        [GeneralName::DNSName(PEER_NAME)]
    ) {
        return Err(CertificateError::Name);
    }
    Ok(())
}

fn validate_extension_der(
    parsed: &ParsedExtension<'_>,
    value: &[u8],
) -> Result<(), CertificateError> {
    use x509_cert::ext::pkix;

    let valid = match parsed {
        ParsedExtension::BasicConstraints(_) => canonical_der::<pkix::BasicConstraints>(value),
        ParsedExtension::KeyUsage(_) => canonical_der::<pkix::KeyUsage>(value),
        ParsedExtension::ExtendedKeyUsage(_) => canonical_der::<pkix::ExtendedKeyUsage>(value),
        ParsedExtension::SubjectAlternativeName(_) => canonical_der::<pkix::SubjectAltName>(value),
        ParsedExtension::SubjectKeyIdentifier(_) => {
            canonical_der::<pkix::SubjectKeyIdentifier>(value)
        }
        ParsedExtension::AuthorityKeyIdentifier(_) => {
            canonical_der::<pkix::AuthorityKeyIdentifier>(value)
        }
        _ => Err(()),
    };
    valid.map_err(|_| CertificateError::Extensions)
}

fn canonical_der<'a, T: Decode<'a> + Encode>(bytes: &'a [u8]) -> Result<(), ()> {
    let value = T::from_der(bytes).map_err(|_| ())?;
    if value.to_der().map_err(|_| ())? != bytes {
        return Err(());
    }
    Ok(())
}

fn validate_usages(certificate: &X509Certificate<'_>) -> Result<(), CertificateError> {
    if let Some(constraints) = certificate
        .basic_constraints()
        .map_err(|_| CertificateError::Extensions)?
    {
        if constraints.value.ca || constraints.value.path_len_constraint.is_some() {
            return Err(CertificateError::CertificateAuthority);
        }
    }
    let usage = certificate
        .key_usage()
        .map_err(|_| CertificateError::Extensions)?
        .ok_or(CertificateError::Usage)?;
    if !usage.value.digital_signature() || usage.value.key_cert_sign() || usage.value.crl_sign() {
        return Err(CertificateError::Usage);
    }
    let extended = certificate
        .extended_key_usage()
        .map_err(|_| CertificateError::Extensions)?
        .ok_or(CertificateError::Usage)?;
    let usage = extended.value;
    if !usage.server_auth
        || !usage.client_auth
        || usage.any
        || usage.code_signing
        || usage.email_protection
        || usage.time_stamping
        || usage.ocsp_signing
        || !usage.other.is_empty()
    {
        return Err(CertificateError::Usage);
    }
    Ok(())
}

fn validate_time(certificate: &X509Certificate<'_>, now: u64) -> Result<(), CertificateError> {
    let now = i64::try_from(now).map_err(|_| CertificateError::Validity)?;
    let start = certificate.validity().not_before.timestamp();
    let end = certificate.validity().not_after.timestamp();
    let duration = end.checked_sub(start).ok_or(CertificateError::Validity)?;
    if start < 0 || duration <= 0 || duration > CERTIFICATE_LIFETIME_SECONDS {
        return Err(CertificateError::Validity);
    }
    if now < start {
        return Err(CertificateError::NotYetValid);
    }
    if now >= end {
        return Err(CertificateError::Expired);
    }
    Ok(())
}
