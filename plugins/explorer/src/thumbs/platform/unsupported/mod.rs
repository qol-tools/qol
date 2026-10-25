use std::path::{Path, PathBuf};

use super::super::{Flavor, ThumbError};

pub(in crate::thumbs) fn thumbnail(_path: &Path, _flavor: Flavor) -> Result<PathBuf, ThumbError> {
    Err(ThumbError::Unsupported)
}
