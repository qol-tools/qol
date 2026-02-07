mod fallback;

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
}

pub trait FilesProvider: Send + Sync {
    fn load_entries(&self) -> Vec<FileEntry>;
}

pub fn default_provider() -> Box<dyn FilesProvider> {
    Box::new(fallback::FallbackFilesProvider)
}
