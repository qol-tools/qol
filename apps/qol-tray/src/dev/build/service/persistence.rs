use std::collections::HashMap;
use std::path::Path;

use crate::dev::adapters::BuildFingerprintStore;

use super::super::types::BuildRun;

pub(super) fn load_known_fingerprints(
    fingerprint_store: &dyn BuildFingerprintStore,
    config_dir: Option<&Path>,
) -> HashMap<String, String> {
    config_dir
        .map(|dir| fingerprint_store.load(dir))
        .unwrap_or_default()
}

pub(super) fn persist_build_run(
    fingerprint_store: &dyn BuildFingerprintStore,
    config_dir: Option<&Path>,
    build_run: &BuildRun,
) {
    let Some(dir) = config_dir else {
        return;
    };
    if let Err(error) = fingerprint_store.save(dir, &build_run.fingerprints) {
        log::error!("Failed to persist build fingerprints: {}", error);
    }
}
