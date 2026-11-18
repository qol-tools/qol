use std::path::Path;
use std::sync::Arc;

use super::trash_reason::describe;
use crate::ops::bin::{Bin, BinError, Held};

struct SystemTrash {
    no_restore: &'static str,
}

impl Bin for SystemTrash {
    fn trash(&self, path: &Path) -> Result<Option<Held>, BinError> {
        trash::delete(path).map_err(|error| BinError::Failed(describe(&error)))?;
        Ok(None)
    }

    fn restore(&self, _held: &Held, _to: &Path) -> Result<(), BinError> {
        Err(BinError::Failed(self.no_restore.into()))
    }
}

pub(super) fn system_bin(no_restore: &'static str) -> Arc<dyn Bin> {
    Arc::new(SystemTrash { no_restore })
}
