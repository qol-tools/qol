use std::{
    fmt,
    sync::Arc,
    time::{Duration, SystemTime},
};

use rcgen::{
    CertificateParams, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
};
use rustls::{
    crypto::ring,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    sign::CertifiedKey,
};
use time::OffsetDateTime;
use zeroize::Zeroizing;

use super::{
    certificate::{parse_certificate, CERTIFICATE_LIFETIME_SECONDS, PEER_NAME},
    validate_certificate, CertificateError, PeerError, PeerPin,
};

pub struct SecretKeyBytes(Zeroizing<Vec<u8>>);

impl SecretKeyBytes {
    pub fn from_pkcs8(bytes: Zeroizing<Vec<u8>>) -> Self {
        Self(bytes)
    }

    pub fn expose_pkcs8(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SecretKeyBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKeyBytes([REDACTED])")
    }
}

pub struct Identity {
    certificate: CertificateDer<'static>,
    secret: SecretKeyBytes,
    pin: PeerPin,
}

impl Identity {
    pub fn generate(now: SystemTime) -> Result<Self, PeerError> {
        let key = Zeroizing::new(
            KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(|_| PeerError::KeyGeneration)?,
        );
        Self::issue(&key, now)
    }

    pub fn restore(
        certificate: Vec<u8>,
        secret: SecretKeyBytes,
        now: SystemTime,
    ) -> Result<Self, PeerError> {
        let pin = validate_certificate(&certificate, unix_seconds(now)?)?;
        let key = parse_key(&secret)?;
        if key.public_key_raw() != &pin.spki_der()[26..] {
            return Err(PeerError::KeyMismatch);
        }
        let identity = Self {
            certificate: CertificateDer::from(certificate),
            secret,
            pin,
        };
        identity.certified_key()?;
        Ok(identity)
    }

    pub fn restore_or_renew(
        certificate: Vec<u8>,
        secret: SecretKeyBytes,
        now: SystemTime,
    ) -> Result<(Self, bool), PeerError> {
        let seconds = i64::try_from(unix_seconds(now)?).map_err(|_| PeerError::Clock)?;
        let parsed = parse_certificate(&certificate)?;
        let validation_time = if seconds >= parsed.validity().not_after.timestamp() {
            let start = u64::try_from(parsed.validity().not_before.timestamp())
                .map_err(|_| CertificateError::Validity)?;
            SystemTime::UNIX_EPOCH
                .checked_add(Duration::from_secs(start))
                .ok_or(PeerError::Clock)?
        } else {
            now
        };
        let identity = Self::restore(certificate, secret, validation_time)?;
        if !identity.renewal_due(now)? {
            return Ok((identity, false));
        }
        identity.renew(now).map(|renewed| (renewed, true))
    }

    pub fn renew(&self, now: SystemTime) -> Result<Self, PeerError> {
        let key = parse_key(&self.secret)?;
        Self::issue(&key, now)
    }

    pub fn renewal_due(&self, now: SystemTime) -> Result<bool, PeerError> {
        let now = i64::try_from(unix_seconds(now)?).map_err(|_| PeerError::Clock)?;
        let certificate = parse_certificate(self.certificate.as_ref())?;
        Ok(now >= certificate.validity().not_after.timestamp() - 30 * 24 * 60 * 60)
    }

    pub fn pin(&self) -> &PeerPin {
        &self.pin
    }

    pub fn certificate_der(&self) -> &[u8] {
        self.certificate.as_ref()
    }

    pub fn export_secret(&self) -> SecretKeyBytes {
        SecretKeyBytes(Zeroizing::new(self.secret.expose_pkcs8().to_vec()))
    }

    pub(crate) fn certified_key(&self) -> Result<Arc<CertifiedKey>, PeerError> {
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.secret.expose_pkcs8()));
        let signing_key =
            ring::sign::any_supported_type(&key).map_err(|_| PeerError::PrivateKey)?;
        let certified = CertifiedKey::new(vec![self.certificate.clone()], signing_key);
        certified.keys_match().map_err(|_| PeerError::KeyMismatch)?;
        Ok(Arc::new(certified))
    }

    fn issue(key: &KeyPair, now: SystemTime) -> Result<Self, PeerError> {
        let seconds = i64::try_from(unix_seconds(now)?).map_err(|_| PeerError::Clock)?;
        let end = seconds
            .checked_add(CERTIFICATE_LIFETIME_SECONDS)
            .ok_or(PeerError::Clock)?;
        let mut params = CertificateParams::new(vec![PEER_NAME.to_owned()])
            .map_err(|_| PeerError::CertificateGeneration)?;
        params.not_before =
            OffsetDateTime::from_unix_timestamp(seconds).map_err(|_| PeerError::Clock)?;
        params.not_after =
            OffsetDateTime::from_unix_timestamp(end).map_err(|_| PeerError::Clock)?;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![
            ExtendedKeyUsagePurpose::ServerAuth,
            ExtendedKeyUsagePurpose::ClientAuth,
        ];
        let certificate = params
            .self_signed(key)
            .map_err(|_| PeerError::CertificateGeneration)?;
        Self::restore(
            certificate.der().to_vec(),
            SecretKeyBytes(Zeroizing::new(key.serialize_der())),
            now,
        )
    }
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity")
            .field("peer_id", &self.pin.peer_id())
            .field("secret", &self.secret)
            .finish_non_exhaustive()
    }
}

fn parse_key(secret: &SecretKeyBytes) -> Result<Zeroizing<KeyPair>, PeerError> {
    if secret.expose_pkcs8().len() > 4096 {
        return Err(PeerError::PrivateKey);
    }
    KeyPair::from_pkcs8_der_and_sign_algo(
        &PrivatePkcs8KeyDer::from(secret.expose_pkcs8()),
        &PKCS_ECDSA_P256_SHA256,
    )
    .map(Zeroizing::new)
    .map_err(|_| PeerError::PrivateKey)
}

pub(crate) fn unix_seconds(now: SystemTime) -> Result<u64, PeerError> {
    now.duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| PeerError::Clock)
}
