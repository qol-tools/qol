mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod linux_index;

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
}

pub trait FilesProvider: Send + Sync {
    fn load_entries(&self) -> Vec<FileEntry>;
}

pub(crate) fn watch_roots() -> Vec<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        linux::file_roots()
    }
    #[cfg(not(target_os = "linux"))]
    {
        fallback::file_roots()
    }
}

pub fn default_provider() -> Box<dyn FilesProvider> {
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxFilesProvider)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(fallback::FallbackFilesProvider)
    }
}
