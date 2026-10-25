use super::super::DetectedShadow;
use std::collections::BTreeMap;

pub(in crate::doctor::checks::hotkey_shadows) fn collect_shadows(
    _qol_index: &BTreeMap<String, String>,
) -> Vec<DetectedShadow> {
    Vec::new()
}

pub(in crate::doctor::checks::hotkey_shadows) fn shadows_under(
    _qol_index: &BTreeMap<String, String>,
    _prefixes: &[String],
) -> Option<Vec<DetectedShadow>> {
    None
}
