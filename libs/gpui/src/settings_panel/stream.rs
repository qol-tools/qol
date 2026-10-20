use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};

enum StreamMessage {
    Frame {
        stream: String,
        interval: Duration,
        frame: String,
    },
    Close,
}

pub(super) struct StreamClient {
    url: Option<String>,
    count: AtomicUsize,
    sender: Mutex<Option<mpsc::Sender<StreamMessage>>>,
}

impl StreamClient {
    pub(super) fn new(url: Option<String>) -> Self {
        Self {
            url,
            count: AtomicUsize::new(0),
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

    pub(super) fn send(&self, stream: &str, interval: Duration, frame: String) {
        if self.count.load(Ordering::SeqCst) == 0 {
            return;
        }
        let sender = self.sender.lock().expect("stream sender lock").clone();
        if let Some(sender) = sender {
            let _ = sender.send(StreamMessage::Frame {
                stream: stream.to_string(),
                interval,
                frame,
            });
        }
    }
}

impl Drop for StreamClient {
    fn drop(&mut self) {
        self.close();
    }
}

#[derive(Default)]
struct Throttle {
    windows: HashMap<String, Window>,
}

struct Window {
    opened: Instant,
    interval: Duration,
    held: Option<String>,
}

impl Window {
    fn closes_at(&self) -> Option<Instant> {
        self.opened.checked_add(self.interval)
    }

    fn is_open_at(&self, now: Instant) -> bool {
        self.closes_at().is_none_or(|closes| now < closes)
    }
}

impl Throttle {
    fn offer(
        &mut self,
        stream: String,
        interval: Duration,
        frame: String,
        now: Instant,
    ) -> Option<String> {
        if let Some(window) = self
            .windows
            .get_mut(&stream)
            .filter(|window| window.is_open_at(now))
        {
            window.held = Some(frame);
            return None;
        }
        self.windows.insert(
            stream,
            Window {
                opened: now,
                interval,
                held: None,
            },
        );
        Some(frame)
    }

    fn next_due(&self) -> Option<Instant> {
        self.windows
            .values()
            .filter(|window| window.held.is_some())
            .filter_map(Window::closes_at)
            .min()
    }

    fn take_due(&mut self, now: Instant) -> Vec<String> {
        self.windows
            .values_mut()
            .filter(|window| !window.is_open_at(now))
            .filter_map(|window| {
                let frame = window.held.take()?;
                window.opened = now;
                Some(frame)
            })
            .collect()
    }

    fn take_held(&mut self) -> Vec<String> {
        self.windows
            .values_mut()
            .filter_map(|window| window.held.take())
            .collect()
    }
}

fn write_loop(url: String, receiver: mpsc::Receiver<StreamMessage>) {
    let Ok((mut socket, _)) = tungstenite::connect(&url) else {
        return;
    };
    let mut throttle = Throttle::default();
    loop {
        let mut frames = Vec::new();
        match next_message(&receiver, throttle.next_due()) {
            Ok(Some(StreamMessage::Frame {
                stream,
                interval,
                frame,
            })) => frames.extend(throttle.offer(stream, interval, frame, Instant::now())),
            Ok(Some(StreamMessage::Close)) => {
                write_frames(&mut socket, throttle.take_held());
                let _ = socket.close(None);
                return;
            }
            Ok(None) => {}
            Err(mpsc::RecvError) => return,
        }
        frames.extend(throttle.take_due(Instant::now()));
        if !write_frames(&mut socket, frames) {
            return;
        }
    }
}

fn next_message(
    receiver: &mpsc::Receiver<StreamMessage>,
    due: Option<Instant>,
) -> Result<Option<StreamMessage>, mpsc::RecvError> {
    let Some(due) = due else {
        return receiver.recv().map(Some);
    };
    match receiver.recv_timeout(due.saturating_duration_since(Instant::now())) {
        Ok(message) => Ok(Some(message)),
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(mpsc::RecvError),
    }
}

fn write_frames<S: std::io::Read + std::io::Write>(
    socket: &mut tungstenite::WebSocket<S>,
    frames: Vec<String>,
) -> bool {
    frames.into_iter().all(|frame| {
        socket
            .send(tungstenite::Message::Text(tungstenite::Utf8Bytes::from(
                frame,
            )))
            .is_ok()
    })
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
    use super::{brightness_frame, color_frame, StreamClient, Throttle};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    const INTERVAL: Duration = Duration::from_millis(100);

    fn stub_server() -> (u16, mpsc::Receiver<String>) {
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
            loop {
                match socket.read() {
                    Ok(tungstenite::Message::Text(text)) => {
                        let _ = sender.send(text.to_string());
                    }
                    Ok(tungstenite::Message::Close(_)) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        });
        (port, receiver)
    }

    fn frames_until_close(received: &mpsc::Receiver<String>) -> Vec<String> {
        let mut frames = Vec::new();
        while let Ok(frame) = received.recv_timeout(Duration::from_secs(5)) {
            frames.push(frame);
        }
        frames
    }

    #[test]
    fn a_frame_inside_its_streams_declared_interval_is_held() {
        let start = Instant::now();
        let cases = [
            (1000, 150, false),
            (1000, 999, false),
            (1000, 1000, true),
            (100, 99, false),
            (100, 100, true),
            (20, 19, false),
            (20, 50, true),
        ];
        for (interval_ms, since_last_ms, sent) in cases {
            let interval = Duration::from_millis(interval_ms);
            let mut throttle = Throttle::default();
            throttle.offer("live".into(), interval, "first".into(), start);
            let next = throttle.offer(
                "live".into(),
                interval,
                "next".into(),
                start + Duration::from_millis(since_last_ms),
            );
            assert_eq!(
                next.is_some(),
                sent,
                "{since_last_ms}ms into a {interval_ms}ms interval"
            );
        }
    }

    #[test]
    fn streams_on_one_source_keep_separate_windows() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let fast = Duration::from_millis(20);
        let slow = Duration::from_millis(1000);
        let mut throttle = Throttle::default();
        assert!(throttle
            .offer("fast".into(), fast, "fast-0".into(), at(0))
            .is_some());
        assert!(throttle
            .offer("slow".into(), slow, "slow-0".into(), at(50))
            .is_some());
        assert!(throttle
            .offer("fast".into(), fast, "fast-1".into(), at(60))
            .is_some());
        assert!(throttle
            .offer("slow".into(), slow, "slow-1".into(), at(100))
            .is_none());
    }

    #[test]
    fn the_newest_held_frame_goes_out_when_its_window_closes() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let interval = Duration::from_millis(1000);
        let mut throttle = Throttle::default();
        throttle.offer("live".into(), interval, "first".into(), at(0));
        throttle.offer("live".into(), interval, "older".into(), at(150));
        throttle.offer("live".into(), interval, "newest".into(), at(300));
        assert_eq!(throttle.next_due(), Some(at(1000)));
        assert!(throttle.take_due(at(999)).is_empty());
        assert_eq!(throttle.take_due(at(1000)), ["newest"]);
        assert_eq!(throttle.next_due(), None);
        assert!(throttle
            .offer("live".into(), interval, "after".into(), at(1500))
            .is_none());
        assert_eq!(throttle.take_held(), ["after"]);
    }

    #[test]
    fn closing_delivers_the_held_last_frame_and_drops_the_superseded_one() {
        let (port, received) = stub_server();
        let client = StreamClient::new(Some(format!("ws://127.0.0.1:{port}")));
        let interval = Duration::from_secs(60);
        client.open();
        client.send("live", interval, "frame-0".into());
        client.send("live", interval, "superseded".into());
        client.send("live", interval, "frame-1".into());
        client.close();
        assert_eq!(frames_until_close(&received), ["frame-0", "frame-1"]);
    }

    #[test]
    fn a_held_frame_reaches_the_daemon_when_its_window_closes() {
        let (port, received) = stub_server();
        let client = StreamClient::new(Some(format!("ws://127.0.0.1:{port}")));
        client.open();
        client.send("live", INTERVAL, "first".into());
        client.send("live", INTERVAL, "last".into());
        let before_close: Vec<String> = (0..2)
            .filter_map(|_| received.recv_timeout(Duration::from_secs(5)).ok())
            .collect();
        client.close();
        assert_eq!(before_close, ["first", "last"]);
        assert!(frames_until_close(&received).is_empty());
    }

    #[test]
    fn open_refcounts_one_connection_and_close_at_zero_ends_it() {
        let (port, received) = stub_server();
        let client = StreamClient::new(Some(format!("ws://127.0.0.1:{port}")));
        client.open();
        client.open();
        client.send("live", INTERVAL, "first".into());
        client.close();
        client.send("live", INTERVAL, "second".into());
        client.close();
        client.send("live", INTERVAL, "after-close".into());
        assert_eq!(frames_until_close(&received), ["first", "second"]);
    }

    #[test]
    fn a_client_without_a_daemon_port_never_connects_or_panics() {
        let client = StreamClient::new(None);
        client.open();
        client.send("live", INTERVAL, "frame".into());
        client.close();
        client.send("live", INTERVAL, "after".into());
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
