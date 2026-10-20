use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

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

    pub(super) fn send(&self, frame: String, interval: Duration) {
        if self.count.load(Ordering::SeqCst) == 0 {
            return;
        }
        let mut last = self.last_sent.lock().expect("stream throttle lock");
        if last.is_some_and(|sent| sent.elapsed() < interval) {
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

    const INTERVAL: Duration = Duration::from_millis(100);

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
        client.send("frame-0".into(), INTERVAL);
        let within_interval = Instant::now() + Duration::from_secs(60);
        *client.last_sent.lock().unwrap() = Some(within_interval);
        client.send("throttled".into(), INTERVAL);
        let past_interval = Instant::now() - INTERVAL * 2;
        *client.last_sent.lock().unwrap() = Some(past_interval);
        client.send("frame-1".into(), INTERVAL);
        client.close();
        let frames = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            frames
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>(),
            ["frame-0", "frame-1"]
        );
    }

    #[test]
    fn send_throttles_by_the_interval_the_stream_declares() {
        let (port, received) = stub_server();
        let client = StreamClient::new(Some(format!("ws://127.0.0.1:{port}")));
        client.open();
        let cases = [
            ("1000ms-after-150ms", 1000, 150, false),
            ("1000ms-after-1200ms", 1000, 1200, true),
            ("20ms-after-50ms", 20, 50, true),
        ];
        for (frame, interval_ms, since_last_ms, _) in cases {
            *client.last_sent.lock().unwrap() =
                Some(Instant::now() - Duration::from_millis(since_last_ms));
            client.send(frame.into(), Duration::from_millis(interval_ms));
        }
        client.close();
        let frames = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            frames
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>(),
            cases
                .iter()
                .filter(|(_, _, _, delivered)| *delivered)
                .map(|(frame, ..)| *frame)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn open_refcounts_one_connection_and_close_at_zero_ends_it() {
        let (port, received) = stub_server();
        let client = StreamClient::new(Some(format!("ws://127.0.0.1:{port}")));
        client.open();
        client.open();
        std::thread::sleep(Duration::from_millis(150));
        client.send("first".into(), INTERVAL);
        std::thread::sleep(Duration::from_millis(120));
        client.close();
        client.send("second".into(), INTERVAL);
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
        client.send("frame".into(), INTERVAL);
        client.close();
        client.send("after".into(), INTERVAL);
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
