#[cfg(test)]
mod tests;
mod trash_dirs;
mod trash_info;
mod trasher;

use std::sync::Arc;

use crate::ops::bin::Bin;

pub(crate) fn system_bin() -> Arc<dyn Bin> {
    Arc::new(trasher::Trasher::system())
}
