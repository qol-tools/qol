use std::time::Duration;

use crate::client::PlatformStateClient;

const RECONNECT_BACKOFF: Duration = Duration::from_millis(250);

const ENV_PLUGIN_ID: &str = "QOL_TRAY_PLUGIN_ID";

pub fn spawn_host_death_watchdog() {
    if std::env::var_os("QOL_TRAY_STATE_SOCKET").is_none() {
        return;
    }
    let client = PlatformStateClient::from_env();
    let plugin_id = std::env::var(ENV_PLUGIN_ID).unwrap_or_else(|_| "unknown".to_string());
    let _ = std::thread::Builder::new()
        .name("qol-host-watchdog".into())
        .spawn(move || watch_host(&client, &plugin_id, is_orphaned, || std::process::exit(0)));
}

fn is_orphaned() -> bool {
    unsafe { libc::getppid() == 1 }
}

fn watch_host(
    client: &PlatformStateClient,
    plugin_id: &str,
    orphaned: impl Fn() -> bool,
    on_host_death: impl FnOnce(),
) {
    loop {
        if let Some(lifeline) = client.lifeline(plugin_id) {
            for _ in lifeline.events() {}
            break;
        }
        if orphaned() {
            break;
        }
        std::thread::sleep(RECONNECT_BACKOFF);
    }
    on_host_death();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::SubscribeAck;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;
    use std::time::Duration;

    fn temp_socket_path(tag: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        let pid = std::process::id();
        path.push(format!("qol-watchdog-test-{tag}-{pid}.sock"));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn ack_line() -> String {
        let mut s = serde_json::to_string(&SubscribeAck::Subscribed).expect("serialize ack");
        s.push('\n');
        s
    }

    #[test]
    fn fires_on_host_death_after_a_successful_connect() {
        let path = temp_socket_path("eof");
        let listener = UnixListener::bind(&path).expect("bind fake host");

        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            let mut writer = stream;
            writer.write_all(ack_line().as_bytes()).expect("write ack");
            writer.flush().expect("flush");
            // Host stays up briefly, then dies: dropping the stream + listener
            // closes the socket, which is the EOF the daemon must detect.
            std::thread::sleep(Duration::from_millis(100));
            drop(writer);
            drop(listener);
        });

        let client = PlatformStateClient::new(path.clone());
        let (tx, rx) = mpsc::channel();
        let watcher = std::thread::spawn(move || {
            watch_host(
                &client,
                "test-plugin",
                || false,
                move || tx.send(()).unwrap(),
            );
        });

        let died = rx.recv_timeout(Duration::from_secs(5));
        assert!(
            died.is_ok(),
            "watchdog must fire when the host socket closes"
        );

        server.join().expect("server thread");
        watcher.join().expect("watcher thread");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fires_when_orphaned_before_ever_connecting() {
        // No server is listening, so subscribe() always fails. An orphaned
        // daemon (host died during our startup race) must still exit instead
        // of spinning forever.
        let path = temp_socket_path("orphan");
        let client = PlatformStateClient::new(path);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            watch_host(
                &client,
                "test-plugin",
                || true,
                move || tx.send(()).unwrap(),
            );
        });

        let died = rx.recv_timeout(Duration::from_secs(2));
        assert!(
            died.is_ok(),
            "watchdog must exit when orphaned even if it never reached the host",
        );
    }

    #[test]
    fn keeps_retrying_while_host_absent_but_not_orphaned() {
        // Host not up yet (subscribe fails) and we are not orphaned: the
        // watchdog must NOT fire - it has to wait for the host to appear.
        let path = temp_socket_path("retry");
        let client = PlatformStateClient::new(path);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            watch_host(
                &client,
                "test-plugin",
                || false,
                move || tx.send(()).unwrap(),
            );
        });

        let fired = rx.recv_timeout(Duration::from_millis(600));
        assert!(
            fired.is_err(),
            "watchdog must keep waiting while the host is merely not-yet-up",
        );
    }
}
