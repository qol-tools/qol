use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const ICON_PIXELS: usize = 128;

pub(super) fn icon_path(app: &Path) -> Option<PathBuf> {
    let cached = super::cache_dir()?
        .join("qol-launcher/icons")
        .join(cache_name(app, stamp(app)));
    if cached.is_file() {
        return Some(cached);
    }
    let png = qol_app_icon::icon_png_for_path(app, ICON_PIXELS)?;
    fs::create_dir_all(cached.parent()?).ok()?;
    let staged = cached.with_extension("part");
    fs::write(&staged, png).ok()?;
    fs::rename(&staged, &cached).ok()?;
    Some(cached)
}

fn stamp(app: &Path) -> Option<SystemTime> {
    fs::metadata(app.join("Contents/Info.plist"))
        .or_else(|_| fs::metadata(app))
        .and_then(|metadata| metadata.modified())
        .ok()
}

fn cache_name(app: &Path, stamp: Option<SystemTime>) -> String {
    let mut hasher = DefaultHasher::new();
    app.hash(&mut hasher);
    stamp.hash(&mut hasher);
    format!("{:016x}.png", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn cache_names_change_when_the_bundle_changes() {
        let app = Path::new("/Applications/Foo.app");
        let before = Some(UNIX_EPOCH + Duration::from_secs(1));
        let after = Some(UNIX_EPOCH + Duration::from_secs(2));

        assert_eq!(cache_name(app, before), cache_name(app, before));
        assert_ne!(cache_name(app, before), cache_name(app, after));
        assert_ne!(
            cache_name(app, before),
            cache_name(Path::new("/Applications/Bar.app"), before)
        );
    }
}
