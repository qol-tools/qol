use std::path::PathBuf;

use super::Browser;
use crate::relay::Message;

impl Browser {
    pub fn cursor_file(&self) -> Option<PathBuf> {
        if self.loading() {
            return None;
        }
        self.entry(self.cursor)
            .filter(|entry| !entry.opens_as_folder)
            .map(|entry| entry.path.clone())
    }

    pub fn announce(&mut self, title: String) {
        self.failure = None;
        self.post(Message::info(title, ""));
    }
}
