mod platform;

use std::path::Path;

pub(super) fn codesign_debug_binaries(plugin_id: &str, plugin_path: &Path) {
    platform::codesign_debug_binaries(plugin_id, plugin_path);
}
