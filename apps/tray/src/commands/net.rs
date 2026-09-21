use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);

pub fn wait_for_tcp_ready(addr: SocketAddr, attempts: u32, interval: Duration) -> bool {
    for attempt in 0..attempts {
        if TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).is_ok() {
            return true;
        }
        if attempt + 1 < attempts {
            std::thread::sleep(interval);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::Instant;

    #[test]
    fn returns_true_on_first_attempt_without_sleeping() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let ready = wait_for_tcp_ready(addr, 40, Duration::from_secs(3600));
        assert!(
            ready,
            "expected ready on the first attempt for a bound listener on {addr}; \
             the one-hour interval means any sleep on the ready path would hang this test"
        );
    }

    #[test]
    fn returns_false_after_exhausting_attempts_when_port_closed() {
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        let interval = Duration::from_millis(5);
        let start = Instant::now();
        let ready = wait_for_tcp_ready(addr, 3, interval);
        assert!(!ready, "expected not-ready for closed port {addr}");
        assert!(
            start.elapsed() >= interval * 2,
            "expected sleeps between the 3 attempts, took {:?}",
            start.elapsed()
        );
    }
}
