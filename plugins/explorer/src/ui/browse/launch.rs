use std::path::PathBuf;

use super::Browser;

impl Browser {
    pub fn launch_paths(&self) -> Vec<PathBuf> {
        self.operands()
    }

    pub fn cursor_file(&self) -> Option<PathBuf> {
        if self.loading() {
            return None;
        }
        self.entry(self.cursor)
            .filter(|entry| !entry.opens_as_folder)
            .map(|entry| entry.path.clone())
    }

    pub fn announce(&mut self, text: String) {
        self.failure = None;
        self.ops.notice = Some(text);
    }
}
