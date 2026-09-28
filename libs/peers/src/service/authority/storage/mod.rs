mod platform;

#[cfg(all(test, target_os = "linux"))]
mod faults;

use std::path::Path;
use zeroize::Zeroizing;

use super::{snapshot, state::State, AuthorityError};
use crate::AuthorityLifetime;

pub(super) enum Storage {
    Session,
    Persistent(platform::Store),
}

impl Storage {
    pub fn create(root: &Path) -> Result<Self, AuthorityError> {
        platform::Store::create(root).map(Self::Persistent)
    }

    pub fn open(root: &Path) -> Result<Self, AuthorityError> {
        platform::Store::open(root).map(Self::Persistent)
    }

    pub fn lifetime(&self) -> AuthorityLifetime {
        match self {
            Self::Session => AuthorityLifetime::Session,
            Self::Persistent(_) => AuthorityLifetime::Persistent,
        }
    }

    pub fn read(&self) -> Result<Zeroizing<Vec<u8>>, AuthorityError> {
        match self {
            Self::Session => Err(AuthorityError::MissingStore),
            Self::Persistent(store) => store.read(),
        }
    }

    pub fn encode(&self, state: &State) -> Result<Option<Zeroizing<Vec<u8>>>, AuthorityError> {
        match self {
            Self::Session => {
                snapshot::encode(state)?;
                Ok(None)
            }
            Self::Persistent(_) => snapshot::encode(state).map(Some),
        }
    }

    pub fn publish(&mut self, bytes: Option<&[u8]>) -> Result<(), AuthorityError> {
        match (self, bytes) {
            (Self::Session, None) => Ok(()),
            (Self::Persistent(store), Some(bytes)) => store.write(bytes),
            (Self::Session, Some(_)) | (Self::Persistent(_), None) => Err(AuthorityError::Storage),
        }
    }

    pub fn commit(&mut self, state: &State) -> Result<(), AuthorityError> {
        let bytes = self.encode(state)?;
        self.publish(bytes.as_ref().map(|bytes| bytes.as_slice()))
    }
}

#[cfg(all(test, target_os = "linux"))]
pub(super) use platform::CommitFault;
