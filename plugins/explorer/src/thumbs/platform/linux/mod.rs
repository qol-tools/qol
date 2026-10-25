mod cache;
mod mime;
mod sandbox;
mod thumbnailers;

use std::path::{Path, PathBuf};

use super::super::{Flavor, ThumbError};
use cache::Stamp;

pub(in crate::thumbs) fn thumbnail(path: &Path, flavor: Flavor) -> Result<PathBuf, ThumbError> {
    let stamp = Stamp::of(path)?;
    if let Some(found) = cache::shared(&stamp, flavor) {
        return Ok(found);
    }
    let own = cache::own(&stamp, flavor)?;
    if own.is_file() {
        return Ok(own);
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ThumbError::Unavailable("the file name is not readable".into()))?;
    let mime =
        mime::of(name).ok_or_else(|| ThumbError::Unavailable("the file type is unknown".into()))?;
    let thumbnailer = thumbnailers::for_mime(&mime).ok_or_else(|| {
        ThumbError::Unavailable(format!("no installed thumbnailer previews {mime}"))
    })?;
    thumbnailer.run(path, &stamp, flavor, &own)?;
    cache::forget_older(&stamp, &own);
    Ok(own)
}

fn data_dirs() -> Vec<PathBuf> {
    let set = |key: &str| {
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
    };
    let home = set("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".local/share")));
    let shared = set("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home.into_iter()
        .chain(
            shared
                .split(':')
                .filter(|segment| !segment.trim().is_empty())
                .map(PathBuf::from),
        )
        .filter(|dir| dir.is_absolute())
        .collect()
}

fn on_path(program: &str) -> Option<PathBuf> {
    let path = Path::new(program);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}
