use std::{fmt, str::FromStr};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("enrollment identifier must be canonical base64url encoding of 16 bytes")]
pub struct EnrollmentIdError;

macro_rules! identifier {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub struct $name([u8; 16]);

        impl $name {
            #[cfg(feature = "service")]
            pub(crate) fn from_random(bytes: [u8; 16]) -> Self {
                Self(bytes)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&URL_SAFE_NO_PAD.encode(self.0))
            }
        }

        impl FromStr for $name {
            type Err = EnrollmentIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                if value.len() != 22 {
                    return Err(EnrollmentIdError);
                }
                let bytes: [u8; 16] = URL_SAFE_NO_PAD
                    .decode(value)
                    .map_err(|_| EnrollmentIdError)?
                    .try_into()
                    .map_err(|_| EnrollmentIdError)?;
                if URL_SAFE_NO_PAD.encode(bytes) != value {
                    return Err(EnrollmentIdError);
                }
                Ok(Self(bytes))
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer)?
                    .parse()
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}

identifier!(InvitationId);
identifier!(TransactionId);
