use super::super::super::binding::{Binding, CaptureEvent, Phase, HEARTBEAT_INTERVAL};
use super::super::key_matcher::{self, KeyCombo, KeyMatcher, KeyTransition};
use super::{parse_mac_combo, recorder};
use core_graphics::event::CGEventType;
use qol_hotkeys::grammar::Modifier as Mod;
use qol_runtime::hotkey_relay::{
    self, HotkeyCombo, HotkeyLease, HotkeyModifier, HotkeyPress, RelayMessage,
};
use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};
use std::io::ErrorKind;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::{Duration, Instant};

const DATAGRAM_BYTES: usize = 1024;
const SOCKET_CHECK: Duration = Duration::from_secs(1);

struct Announced {
    hotkeys: Vec<HotkeyCombo>,
    relay: Option<PathBuf>,
}

static ANNOUNCED: Mutex<Announced> = Mutex::new(Announced {
    hotkeys: Vec::new(),
    relay: None,
});

struct Lease {
    combos: Vec<KeyCombo>,
    renewed: Option<Instant>,
}

static LEASE: Mutex<Lease> = Mutex::new(Lease {
    combos: Vec::new(),
    renewed: None,
});

fn lease() -> MutexGuard<'static, Lease> {
    LEASE.lock().unwrap_or_else(PoisonError::into_inner)
}

fn renew(lease: &HotkeyLease, now: Instant) {
    let mut held = self::lease();
    held.combos = lease.lease.iter().map(key_combo).collect();
    held.renewed = Some(now);
}

pub(super) fn leased(observed: &KeyCombo) -> bool {
    leased_at(observed, Instant::now())
}

fn leased_at(observed: &KeyCombo, now: Instant) -> bool {
    let lease = lease();
    lease
        .renewed
        .is_some_and(|renewed| now.saturating_duration_since(renewed) < hotkey_relay::LEASE_TTL)
        && lease.combos.contains(observed)
}

fn announced() -> MutexGuard<'static, Announced> {
    ANNOUNCED.lock().unwrap_or_else(PoisonError::into_inner)
}

fn current() -> RuntimeEvent {
    let announced = announced();
    RuntimeEvent::HotkeysChanged {
        hotkeys: announced.hotkeys.clone(),
        relay: announced.relay.clone(),
    }
}

pub(super) fn start(matcher: Arc<RwLock<KeyMatcher>>, fire_tx: Sender<CaptureEvent>) {
    crate::runtime::replay_on_subscribe(RuntimeEventKind::HotkeysChanged, current);
    let Some(path) = hotkey_relay::socket_path() else {
        log::warn!("no runtime directory for the hotkey relay socket");
        return;
    };
    let socket = match bind(&path) {
        Ok(socket) => socket,
        Err(error) => {
            log::warn!(
                "hotkey relay socket {} unavailable: {error}",
                path.display()
            );
            return;
        }
    };
    let relay_path = path.clone();
    let spawned = std::thread::Builder::new()
        .name("hotkey-relay".into())
        .spawn(move || serve(socket, &relay_path, &matcher, &fire_tx));
    match spawned {
        Ok(_) => announced().relay = Some(path),
        Err(error) => log::warn!("hotkey relay thread did not start: {error}"),
    }
}

pub(super) fn announce(bindings: &[Binding]) {
    let hotkeys = relayed(bindings);
    {
        let mut announced = announced();
        if announced.hotkeys == hotkeys {
            return;
        }
        announced.hotkeys = hotkeys;
    }
    crate::runtime::publish(&[current()]);
}

fn relayed(bindings: &[Binding]) -> Vec<HotkeyCombo> {
    let mut hotkeys: Vec<HotkeyCombo> = Vec::new();
    for combo in bindings.iter().filter_map(parse_mac_combo) {
        let combo = HotkeyCombo {
            mods: combo
                .mods
                .iter()
                .map(|&modifier| relay_mod(modifier))
                .collect(),
            mac_keycode: combo.key,
        };
        if !hotkeys.contains(&combo) {
            hotkeys.push(combo);
        }
    }
    hotkeys
}

fn bind(path: &Path) -> std::io::Result<UnixDatagram> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(path);
    let socket = UnixDatagram::bind(path)?;
    socket.set_read_timeout(Some(HEARTBEAT_INTERVAL))?;
    Ok(socket)
}

fn serve(
    mut socket: UnixDatagram,
    path: &Path,
    matcher: &RwLock<KeyMatcher>,
    fire_tx: &Sender<CaptureEvent>,
) {
    while run(&socket, path, matcher, fire_tx) == Stopped::SocketGone {
        log::error!(
            "hotkey relay socket {} was removed; binding it again",
            path.display()
        );
        match bind(path) {
            Ok(rebound) => socket = rebound,
            Err(error) => {
                log::error!(
                    "hotkey relay stopped: could not bind {}: {error}",
                    path.display()
                );
                return;
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Stopped {
    SocketGone,
    Failed,
}

fn run(
    socket: &UnixDatagram,
    path: &Path,
    matcher: &RwLock<KeyMatcher>,
    fire_tx: &Sender<CaptureEvent>,
) -> Stopped {
    let mut held: Vec<KeyCombo> = Vec::new();
    let mut datagram = [0u8; DATAGRAM_BYTES];
    let mut last_check = Instant::now();
    loop {
        if last_check.elapsed() >= SOCKET_CHECK {
            last_check = Instant::now();
            if !path.exists() {
                return Stopped::SocketGone;
            }
        }
        match socket.recv(&mut datagram) {
            Ok(len) => match serde_json::from_slice::<RelayMessage>(&datagram[..len]) {
                Ok(RelayMessage::Press(press)) => handle(matcher, fire_tx, &mut held, &press),
                Ok(RelayMessage::Lease(lease)) => renew(&lease, Instant::now()),
                Err(error) => log::warn!("hotkey relay sent an unreadable message: {error}"),
            },
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => {
                log::error!("hotkey relay stopped: {error}");
                return Stopped::Failed;
            }
        }
        for combo in &held {
            let outcome = key_matcher::match_under_lock(matcher, KeyTransition::Repeat, combo);
            fire(fire_tx, outcome.fired);
        }
    }
}

fn handle(
    matcher: &RwLock<KeyMatcher>,
    fire_tx: &Sender<CaptureEvent>,
    held: &mut Vec<KeyCombo>,
    press: &HotkeyPress,
) {
    let combo = key_combo(&press.combo);
    held.retain(|other| other.key != combo.key);
    if press.pressed && recorder::global().recording() {
        recorder::handle_event(CGEventType::KeyDown, &combo);
        return;
    }
    let transition = if press.pressed {
        held.push(combo.clone());
        KeyTransition::Press
    } else {
        KeyTransition::Release
    };
    let outcome = key_matcher::match_under_lock(matcher, transition, &combo);
    fire(fire_tx, outcome.fired);
}

fn fire(fire_tx: &Sender<CaptureEvent>, fired: Option<(Binding, Phase)>) {
    if let Some((binding, phase)) = fired {
        let _ = fire_tx.send(CaptureEvent { binding, phase });
    }
}

fn key_combo(combo: &HotkeyCombo) -> KeyCombo {
    KeyCombo {
        mods: combo
            .mods
            .iter()
            .map(|&modifier| grammar_mod(modifier))
            .collect(),
        key: combo.mac_keycode,
    }
}

fn relay_mod(modifier: Mod) -> HotkeyModifier {
    match modifier {
        Mod::Shift => HotkeyModifier::Shift,
        Mod::Ctrl => HotkeyModifier::Ctrl,
        Mod::Alt => HotkeyModifier::Alt,
        Mod::Super => HotkeyModifier::Super,
    }
}

fn grammar_mod(modifier: HotkeyModifier) -> Mod {
    match modifier {
        HotkeyModifier::Shift => Mod::Shift,
        HotkeyModifier::Ctrl => Mod::Ctrl,
        HotkeyModifier::Alt => Mod::Alt,
        HotkeyModifier::Super => Mod::Super,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::capture::parse_combo;
    use qol_hotkeys::macos_keycode::{LEFT_ARROW, SPACE, TAB};
    use std::sync::mpsc;

    const WAIT: Duration = Duration::from_secs(5);

    fn binding(key: &str, continuous: bool) -> Binding {
        Binding {
            combo: parse_combo(key),
            plugin_uid: crate::plugins::PluginUid::new("plugin"),
            action: "open".into(),
            raw_key: key.into(),
            continuous,
            hold: None,
        }
    }

    fn press(modifier: HotkeyModifier, mac_keycode: u16, pressed: bool) -> HotkeyPress {
        HotkeyPress {
            combo: HotkeyCombo {
                mods: vec![modifier],
                mac_keycode,
            },
            pressed,
        }
    }

    fn fired(event: &CaptureEvent) -> (&str, Phase) {
        (event.binding.raw_key.as_str(), event.phase)
    }

    #[test]
    fn every_bound_combo_is_relayed_once_and_unresolvable_keys_are_left_out() {
        let bindings = [
            binding("Alt+Tab", false),
            binding("Alt+Tab", false),
            binding("Super+Space", false),
            binding("Super+Nope", false),
        ];
        assert_eq!(
            relayed(&bindings),
            vec![
                HotkeyCombo {
                    mods: vec![HotkeyModifier::Alt],
                    mac_keycode: TAB,
                },
                HotkeyCombo {
                    mods: vec![HotkeyModifier::Super],
                    mac_keycode: SPACE,
                },
            ]
        );
    }

    #[test]
    fn a_relayed_press_fires_its_binding_through_the_same_matcher_as_the_tap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("relay.sock");
        let socket = bind(&path).unwrap();
        let matcher = RwLock::new(KeyMatcher::new(
            vec![binding("Alt+Tab", false), binding("Super+Left", true)],
            parse_mac_combo,
        ));
        let (fire_tx, fire_rx) = mpsc::channel();
        let relay_path = path.clone();
        std::thread::spawn(move || serve(socket, &relay_path, &matcher, &fire_tx));
        let sender = UnixDatagram::unbound().unwrap();
        let send = |press: HotkeyPress| {
            sender
                .send_to(&serde_json::to_vec(&press).unwrap(), &path)
                .unwrap();
        };
        let next = || fire_rx.recv_timeout(WAIT).unwrap();

        send(press(HotkeyModifier::Alt, TAB, true));
        assert_eq!(fired(&next()), ("Alt+Tab", Phase::START));

        send(press(HotkeyModifier::Alt, TAB, false));
        send(press(HotkeyModifier::Super, LEFT_ARROW, true));
        assert_eq!(fired(&next()), ("Super+Left", Phase::START));
        assert_eq!(
            fired(&next()),
            ("Super+Left", Phase::HEARTBEAT),
            "a held continuous hotkey keeps beating although Karabiner-Elements sends no repeats"
        );
        send(press(HotkeyModifier::Super, LEFT_ARROW, false));
        let stop = std::iter::repeat_with(next)
            .find(|event| event.phase != Phase::HEARTBEAT)
            .unwrap();
        assert_eq!(fired(&stop), ("Super+Left", Phase::STOP));
    }

    #[test]
    fn a_relay_socket_someone_deletes_is_bound_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("relay.sock");
        let socket = bind(&path).unwrap();
        let matcher = RwLock::new(KeyMatcher::new(
            vec![binding("Alt+Tab", false)],
            parse_mac_combo,
        ));
        let (fire_tx, fire_rx) = mpsc::channel();
        let relay_path = path.clone();
        std::thread::spawn(move || serve(socket, &relay_path, &matcher, &fire_tx));

        std::fs::remove_file(&path).unwrap();
        let sender = UnixDatagram::unbound().unwrap();
        let wire = serde_json::to_vec(&press(HotkeyModifier::Alt, TAB, true)).unwrap();
        let started = Instant::now();
        while sender.send_to(&wire, &path).is_err() {
            assert!(
                started.elapsed() < WAIT,
                "the relay never bound its socket again"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(
            fired(&fire_rx.recv_timeout(WAIT).unwrap()),
            ("Alt+Tab", Phase::START)
        );
    }

    #[test]
    fn a_leased_combo_belongs_to_the_relay_until_the_lease_lapses() {
        let shift_super_r = HotkeyCombo {
            mods: vec![HotkeyModifier::Shift, HotkeyModifier::Super],
            mac_keycode: 15,
        };
        let observed = key_combo(&shift_super_r);
        let ctrl_shift_r = key_combo(&HotkeyCombo {
            mods: vec![HotkeyModifier::Ctrl, HotkeyModifier::Shift],
            mac_keycode: 15,
        });
        let start = Instant::now();
        renew(
            &HotkeyLease {
                lease: vec![shift_super_r],
            },
            start,
        );
        assert!(leased_at(&observed, start + Duration::from_secs(2)));
        assert!(!leased_at(&ctrl_shift_r, start));
        assert!(!leased_at(&observed, start + hotkey_relay::LEASE_TTL));
        renew(&HotkeyLease { lease: Vec::new() }, start);
        assert!(!leased_at(&observed, start));
    }
}
