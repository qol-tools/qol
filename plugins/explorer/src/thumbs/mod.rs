mod platform;

use std::fmt;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Flavor {
    Normal,
    Large,
}

impl Flavor {
    pub fn for_edge(edge: f32) -> Self {
        if edge <= Self::Normal.px() as f32 {
            Self::Normal
        } else {
            Self::Large
        }
    }

    pub fn px(self) -> u32 {
        match self {
            Self::Normal => 128,
            Self::Large => 256,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Large => "large",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ThumbError {
    Unsupported,
    Unavailable(String),
}

impl fmt::Display for ThumbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => formatter.write_str("previews are not available on this system"),
            Self::Unavailable(reason) => formatter.write_str(reason),
        }
    }
}

impl std::error::Error for ThumbError {}

pub fn thumbnail(path: &Path, flavor: Flavor) -> Result<PathBuf, ThumbError> {
    platform::thumbnail(path, flavor)
}

pub fn image_size(image: &Path) -> Option<(u32, u32)> {
    let file = File::open(image).ok()?;
    let reader = png::Decoder::new(BufReader::new(file)).read_info().ok()?;
    let info = reader.info();
    (info.width > 0 && info.height > 0).then_some((info.width, info.height))
}
