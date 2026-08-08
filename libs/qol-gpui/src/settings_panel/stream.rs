use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

const SEND_MIN_INTERVAL: Duration = Duration::from_millis(100);

enum StreamMessage {
    Frame(String),
    Close,
}

pub(super) struct StreamClient {
    url: Option<String>,
    count: AtomicUsize,
    last_sent: Mutex<Option<Instant>>,
    sender: Mutex<Option<mpsc::Sender<StreamMessage>>>,
}

impl StreamClient {
    pub(super) fn new(url: Option<String>) -> Self {
        Self {
            url,
            count: AtomicUsize::new(0),
            last_sent: Mutex::new(None),
            sender: Mutex::new(None),
        }
    }

    pub(super) fn open(&self) {
        if self.count.fetch_add(1, Ordering::SeqCst) > 0 {
            return;
        }
        let Some(url) = self.url.clone() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        *self.sender.lock().expect("stream sender lock") = Some(sender);
        std::thread::Builder::new()
            .name("qol-settings-stream".into())
            .spawn(move || write_loop(url, receiver))
            .ok();
    }

    pub(super) fn close(&self) {
        if self.count.load(Ordering::SeqCst) == 0 {
            return;
        }
        if self.count.fetch_sub(1, Ordering::SeqCst) > 1 {
            return;
        }
        let sender = self.sender.lock().expect("stream sender lock").take();
        if let Some(sender) = sender {
            let _ = sender.send(StreamMessage::Close);
        }
    }

    pub(super) fn send(&self, frame: String) {
        if self.count.load(Ordering::SeqCst) == 0 {
            return;
        }
        let mut last = self.last_sent.lock().expect("stream throttle lock");
        if last.is_some_and(|sent| sent.elapsed() < SEND_MIN_INTERVAL) {
            return;
        }
        *last = Some(Instant::now());
        let sender = self.sender.lock().expect("stream sender lock").clone();
        if let Some(sender) = sender {
            let _ = sender.send(StreamMessage::Frame(frame));
        }
    }
}

impl Drop for StreamClient {
    fn drop(&mut self) {
        self.close();
    }
}

fn write_loop(url: String, receiver: mpsc::Receiver<StreamMessage>) {
    let Ok((mut socket, _)) = tungstenite::connect(&url) else {
        return;
    };
    while let Ok(message) = receiver.recv() {
        match message {
            StreamMessage::Frame(frame) => {
                if socket
                    .send(tungstenite::Message::Text(tungstenite::Utf8Bytes::from(
                        frame,
                    )))
                    .is_err()
                {
                    return;
                }
            }
            StreamMessage::Close => {
                let _ = socket.close(None);
                return;
            }
        }
    }
}

pub(super) fn color_frame(hex: &str) -> Option<String> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if !valid_hex(hex) {
        return None;
    }
    Some(format!(r#"{{"type":"color","hex":"{hex}"}}"#))
}

pub(super) fn brightness_frame(level: u8, hex: &str) -> Option<String> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if !valid_hex(hex) {
        return None;
    }
    Some(format!(
        r#"{{"type":"brightness","level":{level},"hex":"{hex}"}}"#
    ))
}

fn valid_hex(hex: &str) -> bool {
    hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::{brightness_frame, color_frame, StreamClient};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn stub_server() -> (u16, mpsc::Receiver<Vec<(Instant, String)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            let Ok(mut socket) = tungstenite::accept(stream) else {
                return;
            };
            let mut frames = Vec::new();
            loop {
                match socket.read() {
                    Ok(tungstenite::Message::Text(text)) => {
                        frames.push((Instant::now(), text.to_string()));
                    }
                    Ok(tungstenite::Message::Close(_)) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            let _ = sender.send(frames);
        });
        (port, receiver)
    }

    #[test]
    fn open_send_close_delivers_throttled_frames_to_the_daemon_stub() {
        let (port, received) = stub_server();
        let client = StreamClient::new(Some(format!("ws://127.0.0.1:{port}")));
        client.open();
        for index in 0..25 {
            client.send(format!("frame-{index}"));
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(300));
        client.close();
        let frames = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            frames.len() >= 2,
            "expected throttled frames to arrive, got {frames:?}"
        );
        assert!(
            frames.len() < 20,
            "frames must be throttled below the daemon drain interval, got {}",
            frames.len()
        );
        assert_eq!(frames[0].1, "frame-0");
        for pair in frames.windows(2) {
            let gap = pair[1].0.duration_since(pair[0].0);
            assert!(
                gap >= Duration::from_millis(85),
                "frames must be spaced like the daemon drain, gap was {gap:?}"
            );
        }
    }

    #[test]
    fn open_refcounts_one_connection_and_close_at_zero_ends_it() {
        let (port, received) = stub_server();
        let client = StreamClient::new(Some(format!("ws://127.0.0.1:{port}")));
        client.open();
        client.open();
        std::thread::sleep(Duration::from_millis(150));
        client.send("first".into());
        std::thread::sleep(Duration::from_millis(120));
        client.close();
        client.send("second".into());
        std::thread::sleep(Duration::from_millis(120));
        client.close();
        let frames = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            frames
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
    }

    #[test]
    fn a_client_without_a_daemon_port_never_connects_or_panics() {
        let client = StreamClient::new(None);
        client.open();
        client.send("frame".into());
        client.close();
        client.send("after".into());
    }

    #[test]
    fn color_frames_strip_the_hash_and_reject_bad_hex() {
        assert_eq!(
            color_frame("#1a2b3c"),
            Some(r#"{"type":"color","hex":"1a2b3c"}"#.to_string())
        );
        assert_eq!(
            color_frame("1a2b3c"),
            Some(r#"{"type":"color","hex":"1a2b3c"}"#.to_string())
        );
        assert_eq!(color_frame("#12"), None);
        assert_eq!(color_frame("#gggggg"), None);
    }

    #[test]
    fn brightness_frames_carry_the_level_and_hex() {
        assert_eq!(
            brightness_frame(42, "1a2b3c"),
            Some(r#"{"type":"brightness","level":42,"hex":"1a2b3c"}"#.to_string())
        );
        assert_eq!(brightness_frame(42, "bad"), None);
    }
}
