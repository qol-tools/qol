use std::io::{self, BufRead, BufReader};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, ensure, Context, Result};

use super::console::{self, ConsoleUser};
use super::devices;
use super::protocol::{self, Role, ToDaemon, ToHelper, PROTOCOL_VERSION, SOCKET_PATH};
use super::Shared;

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const HOUSEKEEPING_INTERVAL: Duration = Duration::from_secs(1);
const HELLO_TIMEOUT: Duration = Duration::from_secs(1);
const WRITE_TIMEOUT: Duration = Duration::from_millis(50);

extern "C" {
    fn getpeereid(socket: i32, uid: *mut u32, gid: *mut u32) -> i32;
}

pub(super) fn serve(shared: &Arc<Shared>) {
    let mut generation = 0;
    loop {
        shared.set_input_monitoring(devices::input_monitoring_granted());
        let Some(owner) = console::console_user() else {
            std::thread::sleep(HOUSEKEEPING_INTERVAL);
            continue;
        };
        match bind(owner) {
            Ok(listener) => {
                log::info!("listening on {SOCKET_PATH} for uid {}", owner.uid);
                accept_until_owner_changes(&listener, owner, shared, &mut generation);
            }
            Err(error) => {
                log::error!("cannot listen on {SOCKET_PATH}: {error}");
                std::thread::sleep(HOUSEKEEPING_INTERVAL);
            }
        }
    }
}

fn bind(owner: ConsoleUser) -> io::Result<UnixListener> {
    match std::fs::remove_file(SOCKET_PATH) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let listener = UnixListener::bind(SOCKET_PATH)?;
    std::os::unix::fs::chown(SOCKET_PATH, Some(owner.uid), Some(owner.gid))?;
    std::fs::set_permissions(SOCKET_PATH, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn accept_until_owner_changes(
    listener: &UnixListener,
    owner: ConsoleUser,
    shared: &Arc<Shared>,
    generation: &mut u64,
) {
    let mut next_housekeeping = Instant::now() + HOUSEKEEPING_INTERVAL;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) = accept(stream, owner, shared, generation) {
                    log::warn!("refused a keyremap connection: {error:#}");
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(POLL_INTERVAL)
            }
            Err(error) => {
                log::warn!("accept on {SOCKET_PATH} failed: {error}");
                std::thread::sleep(POLL_INTERVAL);
            }
        }
        if Instant::now() >= next_housekeeping {
            next_housekeeping = Instant::now() + HOUSEKEEPING_INTERVAL;
            shared.set_input_monitoring(devices::input_monitoring_granted());
            if console::console_user() != Some(owner) {
                log::info!("the console user changed; re-creating {SOCKET_PATH}");
                shared.drop_session();
                return;
            }
        }
    }
}

fn accept(
    stream: UnixStream,
    owner: ConsoleUser,
    shared: &Arc<Shared>,
    generation: &mut u64,
) -> Result<()> {
    stream.set_nonblocking(false)?;
    let peer = peer_uid(&stream)?;
    ensure!(
        peer == owner.uid,
        "peer uid {peer} is not the console user {}",
        owner.uid
    );
    stream.set_read_timeout(Some(HELLO_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let mut writer = stream.try_clone()?;
    let mut lines = BufReader::new(stream).lines();
    let hello = lines
        .next()
        .context("the client closed before saying hello")??;
    let ToHelper::Hello { protocol, role } = protocol::parse_message(&hello)? else {
        bail!("the first message was not hello");
    };
    if protocol != PROTOCOL_VERSION {
        protocol::write_message(
            &mut writer,
            &ToDaemon::Refused {
                protocol: PROTOCOL_VERSION,
                reason: format!("the keyboard helper speaks protocol {PROTOCOL_VERSION}, keyremap speaks {protocol}"),
            },
        )?;
        return Ok(());
    }
    match role {
        Role::Status => protocol::write_message(&mut writer, &ToDaemon::Status(shared.status()))?,
        Role::Session => {
            *generation += 1;
            let session = *generation;
            protocol::write_message(
                &mut writer,
                &ToDaemon::Welcome {
                    protocol: PROTOCOL_VERSION,
                },
            )?;
            writer.set_read_timeout(None)?;
            shared.open_session(session, writer);
            let shared = Arc::clone(shared);
            std::thread::Builder::new()
                .name(format!("keyremap-helper-session-{session}"))
                .spawn(move || read_session(session, lines, &shared))?;
        }
    }
    Ok(())
}

fn read_session(generation: u64, lines: impl Iterator<Item = io::Result<String>>, shared: &Shared) {
    for line in lines {
        let Ok(line) = line else {
            break;
        };
        match protocol::parse_message::<ToHelper>(&line) {
            Ok(ToHelper::Heartbeat) => shared.heartbeat(generation),
            Ok(ToHelper::Emit {
                usage_page,
                usage,
                pressed,
            }) => {
                shared.emit(generation, usage_page, usage, pressed);
            }
            Ok(ToHelper::CapsLockLight { on }) => shared.set_caps_light(generation, on),
            Ok(ToHelper::Hello { .. }) => {}
            Err(error) => log::warn!("ignoring a bad message from keyremap: {error}"),
        }
    }
    shared.close_session(generation);
}

fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let (mut uid, mut gid) = (0, 0);
    if unsafe { getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(uid)
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    use super::*;

    extern "C" {
        fn getuid() -> u32;
        fn getgid() -> u32;
    }

    fn me() -> ConsoleUser {
        ConsoleUser {
            uid: unsafe { getuid() },
            gid: unsafe { getgid() },
        }
    }

    fn say(client: &mut UnixStream, message: &ToHelper) {
        protocol::write_message(client, message).unwrap();
    }

    fn hear(client: &UnixStream) -> ToDaemon {
        let mut line = String::new();
        BufReader::new(client).read_line(&mut line).unwrap();
        protocol::parse_message(&line).unwrap()
    }

    #[test]
    fn status_connections_get_the_status() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let shared = Arc::new(Shared::default());
        say(
            &mut client,
            &ToHelper::Hello {
                protocol: PROTOCOL_VERSION,
                role: Role::Status,
            },
        );
        accept(server, me(), &shared, &mut 0).unwrap();
        let ToDaemon::Status(status) = hear(&client) else {
            panic!("expected status")
        };
        assert_eq!(status.protocol, PROTOCOL_VERSION);
        assert!(!status.virtual_keyboard_ready);
    }

    #[test]
    fn another_protocol_is_refused_with_the_helper_version() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let shared = Arc::new(Shared::default());
        say(
            &mut client,
            &ToHelper::Hello {
                protocol: PROTOCOL_VERSION + 1,
                role: Role::Session,
            },
        );
        accept(server, me(), &shared, &mut 0).unwrap();
        assert!(matches!(
            hear(&client),
            ToDaemon::Refused { protocol, .. } if protocol == PROTOCOL_VERSION
        ));
    }

    #[test]
    fn a_session_is_welcomed_and_its_heartbeats_reach_the_watchdog() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let shared = Arc::new(Shared::default());
        let mut generation = 0;
        say(
            &mut client,
            &ToHelper::Hello {
                protocol: PROTOCOL_VERSION,
                role: Role::Session,
            },
        );
        accept(server, me(), &shared, &mut generation).unwrap();
        assert!(matches!(hear(&client), ToDaemon::Welcome { .. }));
        assert_eq!(generation, 1);
        assert!(shared.lock().watchdog.is_current(1));

        drop(client);
        for _ in 0..50 {
            if !shared.lock().watchdog.is_current(1) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("closing the session did not reach the watchdog");
    }

    #[test]
    fn a_peer_from_another_user_is_refused() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let shared = Arc::new(Shared::default());
        say(
            &mut client,
            &ToHelper::Hello {
                protocol: PROTOCOL_VERSION,
                role: Role::Status,
            },
        );
        let stranger = ConsoleUser {
            uid: me().uid + 1,
            gid: me().gid,
        };
        assert!(accept(server, stranger, &shared, &mut 0).is_err());
        let _ = client.flush();
    }
}
