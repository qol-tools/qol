use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::Sender;
use std::time::Duration;

const SOCKET_PATH: &str = "/tmp/qol-launcher.sock";

pub enum Command {
    Show,
    Kill,
}

pub fn send_show() -> bool {
    send_raw(b"show")
}

pub fn send_kill() -> bool {
    send_raw(b"kill")
}

pub fn start_listener(tx: Sender<Command>) -> bool {
    if send_show() {
        return false;
    }

    let _ = fs::remove_file(SOCKET_PATH);
    let Ok(listener) = UnixListener::bind(SOCKET_PATH) else {
        return false;
    };
    listener.set_nonblocking(true).ok();

    std::thread::spawn(move || {
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if let Some(cmd) = read_command(&mut stream) {
                        if tx.send(cmd).is_err() {
                            break;
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => break,
            }
        }
        let _ = fs::remove_file(SOCKET_PATH);
    });

    true
}

pub fn cleanup() {
    let _ = fs::remove_file(SOCKET_PATH);
}

fn send_raw(msg: &[u8]) -> bool {
    let Ok(mut stream) = UnixStream::connect(SOCKET_PATH) else {
        return false;
    };
    stream.write_all(msg).is_ok()
}

fn read_command(stream: &mut UnixStream) -> Option<Command> {
    let mut buf = [0u8; 16];
    let n = stream.read(&mut buf).ok()?;
    match &buf[..n] {
        b"show" => Some(Command::Show),
        b"kill" => Some(Command::Kill),
        _ => None,
    }
}
