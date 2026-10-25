use super::Browser;
use crate::props::Props;

impl Browser {
    pub fn toggle_props(&mut self) -> bool {
        if self.props.take().is_some() {
            return true;
        }
        let paths = self.operands();
        let file_kind = paths
            .first()
            .and_then(|path| self.row_of(path))
            .and_then(|row| self.entry(row))
            .map_or("File", super::super::columns::kind_label);
        self.props = Props::open(&paths, file_kind, self.waker());
        self.props.is_some()
    }

    pub fn props_open(&self) -> bool {
        self.props.is_some()
    }

    pub(crate) fn props(&self) -> Option<&Props> {
        self.props.as_ref()
    }

    pub(super) fn refresh_props(&mut self) -> bool {
        self.props.as_mut().is_some_and(Props::refresh)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::ui::browse::Browser;

    fn browser_over(names: &[&str]) -> (tempfile::TempDir, Browser) {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in names {
            fs::write(dir.path().join(name), b"data").expect("file");
        }
        let mut browser = Browser::new(dir.path().to_path_buf(), dir.path().to_path_buf(), false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while browser.row_count() < names.len() && std::time::Instant::now() < deadline {
            browser.drain(std::time::Instant::now());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        (dir, browser)
    }

    #[test]
    fn the_same_key_opens_and_closes_the_panel_for_the_cursor_item() {
        let (_dir, mut browser) = browser_over(&["a.txt", "b.txt"]);
        assert!(!browser.props_open());
        assert!(browser.toggle_props());
        assert!(browser.props_open());
        assert_eq!(browser.props().map(|props| props.title()), Some("a.txt"));
        assert!(browser.toggle_props());
        assert!(!browser.props_open());
    }

    #[test]
    fn escape_closes_the_panel_and_keeps_the_picks() {
        let (_dir, mut browser) = browser_over(&["a.txt", "b.txt"]);
        browser.pick_all();
        assert!(browser.toggle_props());
        assert_eq!(browser.props().map(|props| props.title()), Some("2 items"));
        assert!(browser.dismiss());
        assert!(!browser.props_open());
        assert!(browser.is_picked(browser.entry(0).expect("entry")));
    }

    #[test]
    fn an_empty_folder_has_nothing_to_show() {
        let (_dir, mut browser) = browser_over(&[]);
        assert!(!browser.toggle_props());
        assert!(!browser.props_open());
    }
}
