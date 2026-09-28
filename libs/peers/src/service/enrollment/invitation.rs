use std::{fmt, net::SocketAddr};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use super::{random, EnrollmentError};
use crate::enrollment::{
    EnrollmentVersion, ExportedInvitation, InvitationId, MAX_INVITATION_BYTES as MAX_EXPORT_BYTES,
};
use crate::{service::PeerPin, AuthorityLifetime};

const PREFIX: &str = "qol-link:";

pub struct Invitation {
    pub(crate) document: Document,
    pub(crate) pin: PeerPin,
}

impl fmt::Debug for Invitation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Invitation([REDACTED])")
    }
}

impl Invitation {
    pub fn import(exported: &str) -> Result<Self, EnrollmentError> {
        if exported.len() > MAX_EXPORT_BYTES {
            return Err(EnrollmentError::InvalidInvitation);
        }
        let encoded = exported
            .strip_prefix(PREFIX)
            .ok_or(EnrollmentError::InvalidInvitation)?;
        let mut bytes = Zeroizing::new(vec![0; encoded.len()]);
        let size = URL_SAFE_NO_PAD
            .decode_slice(encoded, &mut bytes)
            .map_err(|_| EnrollmentError::InvalidInvitation)?;
        bytes.truncate(size);
        let canonical = Zeroizing::new(URL_SAFE_NO_PAD.encode(&bytes));
        if canonical.as_str() != encoded {
            return Err(EnrollmentError::InvalidInvitation);
        }
        let document: Document =
            serde_json::from_slice(&bytes).map_err(|_| EnrollmentError::InvalidInvitation)?;
        let mut canonical_json = Zeroizing::new(Vec::with_capacity(MAX_EXPORT_BYTES));
        serde_json::to_writer(&mut *canonical_json, &document)
            .map_err(|_| EnrollmentError::InvalidInvitation)?;
        if *canonical_json != *bytes {
            return Err(EnrollmentError::InvalidInvitation);
        }
        document.validate()?;
        let pin = document.inviter.to_pin()?;
        Ok(Self { document, pin })
    }

    pub fn export(&self) -> Result<ExportedInvitation, EnrollmentError> {
        let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_EXPORT_BYTES));
        serde_json::to_writer(&mut *bytes, &self.document)
            .map_err(|_| EnrollmentError::InvalidInvitation)?;
        let mut output = Zeroizing::new(String::with_capacity(MAX_EXPORT_BYTES));
        output.push_str(PREFIX);
        URL_SAFE_NO_PAD.encode_string(&bytes, &mut output);
        if output.len() > MAX_EXPORT_BYTES {
            return Err(EnrollmentError::InvalidInvitation);
        }
        ExportedInvitation::from_owned(output).map_err(|_| EnrollmentError::InvalidInvitation)
    }

    pub fn id(&self) -> InvitationId {
        self.document.invitation
    }

    pub fn inviter_pin(&self) -> &PeerPin {
        &self.pin
    }

    pub fn lifetime(&self) -> AuthorityLifetime {
        self.document.lifetime
    }

    pub fn endpoints(&self) -> &[SocketAddr] {
        &self.document.endpoints
    }

    pub(crate) fn create(
        pin: PeerPin,
        lifetime: AuthorityLifetime,
        endpoints: Vec<SocketAddr>,
    ) -> Result<Self, EnrollmentError> {
        let document = Document {
            version: EnrollmentVersion::V1,
            invitation: InvitationId::from_random(random()?),
            secret: Token(Zeroizing::new(random()?)),
            inviter: WirePin::from_pin(&pin),
            endpoints,
            lifetime,
        };
        document.validate()?;
        Ok(Self { document, pin })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Document {
    version: EnrollmentVersion,
    pub invitation: InvitationId,
    pub secret: Token,
    inviter: WirePin,
    endpoints: Vec<SocketAddr>,
    pub lifetime: AuthorityLifetime,
}

impl Document {
    fn validate(&self) -> Result<(), EnrollmentError> {
        if self.endpoints.len() > 8 || self.endpoints.iter().any(|endpoint| endpoint.port() == 0) {
            return Err(EnrollmentError::InvalidInvitation);
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct Token(Zeroizing<[u8; 32]>);

impl Token {
    pub fn matches(&self, other: &Self) -> bool {
        bool::from(self.0.as_slice().ct_eq(other.0.as_slice()))
    }
}

impl Serialize for Token {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let value = Zeroizing::new(URL_SAFE_NO_PAD.encode(self.0.as_slice()));
        serializer.serialize_str(&value)
    }
}

impl<'de> Deserialize<'de> for Token {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TokenVisitor;
        impl serde::de::Visitor<'_> for TokenVisitor {
            type Value = Token;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a canonical invitation secret")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Token, E> {
                let mut bytes = Zeroizing::new([0; 32]);
                if value.len() != 43
                    || URL_SAFE_NO_PAD
                        .decode_slice(value, bytes.as_mut_slice())
                        .ok()
                        != Some(32)
                {
                    return Err(E::custom("invalid invitation secret"));
                }
                let canonical = Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes.as_slice()));
                if canonical.as_str() != value {
                    return Err(E::custom("invalid invitation secret"));
                }
                Ok(Token(bytes))
            }
        }
        deserializer.deserialize_str(TokenVisitor)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct WirePin(String);

impl WirePin {
    pub fn from_pin(pin: &PeerPin) -> Self {
        Self(URL_SAFE_NO_PAD.encode(pin.spki_der()))
    }

    pub fn to_pin(&self) -> Result<PeerPin, EnrollmentError> {
        if self.0.len() != 122 {
            return Err(EnrollmentError::InvalidInvitation);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(&self.0)
            .map_err(|_| EnrollmentError::InvalidInvitation)?;
        if URL_SAFE_NO_PAD.encode(&bytes) != self.0 {
            return Err(EnrollmentError::InvalidInvitation);
        }
        PeerPin::from_spki_der(&bytes).map_err(|_| EnrollmentError::InvalidInvitation)
    }
}
