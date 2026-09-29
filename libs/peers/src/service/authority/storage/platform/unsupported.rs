use std::path::Path;
use zeroize::Zeroizing;

use crate::service::authority::AuthorityError;

pub(in crate::service::authority) struct Store;

impl Store {
    pub fn create(_: &Path) -> Result<Self, AuthorityError> {
        Err(AuthorityError::UnsupportedPlatform)
    }
    pub fn open(_: &Path) -> Result<Self, AuthorityError> {
        Err(AuthorityError::UnsupportedPlatform)
    }
    pub fn read(&self) -> Result<Zeroizing<Vec<u8>>, AuthorityError> {
        Err(AuthorityError::UnsupportedPlatform)
    }
    pub fn write(&mut self, _: &[u8]) -> Result<(), AuthorityError> {
        Err(AuthorityError::UnsupportedPlatform)
    }
}
