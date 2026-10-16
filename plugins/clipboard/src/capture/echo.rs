use sha2::{Digest, Sha256};

use super::{Capture, Content};

// A desktop clipboard keeper (Cinnamon's, for one) takes the selection over when the app that
// copied a secret exits, and offers the same text without the password manager's marker.
// Only a digest of the last secret is held, and only in memory.
#[derive(Default)]
pub struct SecretEcho {
    last_secret: Option<[u8; 32]>,
}

impl SecretEcho {
    /// Whether this capture is the last secret offered again without its marker.
    pub fn repeats_secret(&mut self, capture: &Capture) -> bool {
        let digest = fingerprint(&capture.content);
        if capture.sensitive {
            self.last_secret = Some(digest);
            return false;
        }
        if self.last_secret == Some(digest) {
            return true;
        }
        self.last_secret = None;
        false
    }
}

fn fingerprint(content: &Content) -> [u8; 32] {
    let mut hasher = Sha256::new();
    match content {
        Content::Text(text) => {
            hasher.update(b"text\0");
            hasher.update(text.as_bytes());
        }
        Content::Files(paths) => {
            hasher.update(b"files\0");
            for path in paths {
                hasher.update(path.as_bytes());
                hasher.update(b"\0");
            }
        }
        Content::Png(png) => {
            hasher.update(b"png\0");
            hasher.update(png);
        }
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str, sensitive: bool) -> Capture {
        Capture {
            content: Content::Text(value.to_string()),
            app: None,
            sensitive,
        }
    }

    #[test]
    fn a_secret_offered_again_without_its_marker_is_an_echo_until_other_content_arrives() {
        let mut echo = SecretEcho::default();
        let steps = [
            (text("hunter2", false), false),
            (text("hunter2", true), false),
            (text("hunter2", false), true),
            (text("hunter2", false), true),
            (text("plain", false), false),
            (text("hunter2", false), false),
        ];
        for (index, (capture, repeats)) in steps.into_iter().enumerate() {
            assert_eq!(echo.repeats_secret(&capture), repeats, "step {index}");
        }
    }

    #[test]
    fn files_and_text_with_the_same_bytes_are_different_content() {
        let mut echo = SecretEcho::default();
        assert!(!echo.repeats_secret(&text("/tmp/key", true)));
        let files = Capture {
            content: Content::Files(vec!["/tmp/key".to_string()]),
            app: None,
            sensitive: false,
        };
        assert!(!echo.repeats_secret(&files));
    }
}
