use super::super::super::{Binding, CaptureEvent};
use super::super::super::{OnFire, RebuildBindings};
use super::classify::{self, DeviceCapabilities, DeviceClass, SkipReason};
use super::heal;
use super::matcher::BindingMatcher;
use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender};
use evdev::raw_stream::RawDevice;
use evdev::{
    uinput::VirtualDevice, AttributeSet, AttributeSetRef, EventSummary, EventType, InputEvent,
    KeyCode, SynchronizationCode,
};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The virtual keyboard plus the set of key codes currently emitted as down
/// on it. Only the readers' graceful exit releases held keys; the dev
/// exec-restart and the abort-on-panic hook destroy the process image with
/// whatever is down at that instant still latched in the X server, which then
/// autorepeats that key forever (the desktop is unusable until reboot).
/// Tracking every emit here gives those paths one thing to flush.
pub(super) struct TrackedVirtualDevice {
    device: VirtualDevice,
    held: HashSet<u16>,
}

impl TrackedVirtualDevice {
    fn new(device: VirtualDevice) -> Self {
        Self {
            device,
            held: HashSet::new(),
        }
    }

    fn emit(&mut self, events: &[InputEvent]) -> std::io::Result<()> {
        self.device.emit(events)?;
        track_emitted(&mut self.held, events);
        Ok(())
    }

    fn release_held(&mut self) {
        let events = release_events(&self.held);
        if events.is_empty() {
            return;
        }
        if let Err(error) = self.device.emit(&events) {
            log::warn!("evdev: held-key release flush failed: {error}");
        }
        self.held.clear();
    }
}

fn track_emitted(held: &mut HashSet<u16>, events: &[InputEvent]) {
    for event in events {
        if event.event_type() != EventType::KEY {
            continue;
        }
        match event.value() {
            1 => {
                held.insert(event.code());
            }
            0 => {
                held.remove(&event.code());
            }
            _ => {}
        }
    }
}

fn release_events(held: &HashSet<u16>) -> Vec<InputEvent> {
    held.iter()
        .flat_map(|&code| {
            [
                InputEvent::new(EventType::KEY.0, code, 0),
                InputEvent::new(EventType::SYNCHRONIZATION.0, 0, 0),
            ]
        })
        .collect()
}

static FLUSH_TARGET: OnceLock<Arc<Mutex<TrackedVirtualDevice>>> = OnceLock::new();

pub(super) fn emit_key_ups(
    virtual_device: &Mutex<TrackedVirtualDevice>,
    held: impl IntoIterator<Item = u16>,
) {
    let events = release_events(&held.into_iter().collect());
    if events.is_empty() {
        return;
    }
    let mut vd = virtual_device
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Err(error) = vd.emit(&events) {
        log::warn!("evdev: seeded held-key release failed: {error}");
    }
}

/// Emit key-up for everything still held on the virtual keyboard. For the
/// ways out of the process that skip the readers' own exit release: the dev
/// exec-restart, graceful shutdown, and the abort-on-panic hook. Uses
/// try_lock so a panic on a thread that holds the emit lock cannot deadlock
/// the abort (the grabs must still be released by process death).
pub(super) fn release_held_keys() {
    let Some(target) = FLUSH_TARGET.get() else {
        return;
    };
    match target.try_lock() {
        Ok(mut vd) => vd.release_held(),
        Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner().release_held(),
        Err(std::sync::TryLockError::WouldBlock) => {
            log::warn!("evdev: virtual keyboard busy during exit flush; a held key may stick");
        }
    }
}

/// The only channel through which capture code may reach plugin actions.
///
/// Reader threads sit between the user's keyboard and the desktop: the device is
/// held with EVIOCGRAB, so every keystroke reaches the session only because a
/// reader forwarded it. Any blocking call on that thread therefore freezes all
/// typing and then replays the kernel's backlog in one burst. Handing readers a
/// `Sender` instead of a callback makes that structurally impossible - they hold
/// nothing they can call, and `send` on an unbounded channel never blocks.
type CaptureDispatch = Sender<CaptureEvent>;

fn spawn_dispatch_thread(on_fire: Arc<dyn Fn(&CaptureEvent) + Send + Sync>) -> CaptureDispatch {
    let (tx, rx) = crossbeam_channel::unbounded::<CaptureEvent>();
    std::thread::Builder::new()
        .name("hotkey-capture-dispatch".into())
        .spawn(move || {
            while let Ok(event) = rx.recv() {
                on_fire(&event);
            }
        })
        .map(|_| ())
        .unwrap_or_else(|error| {
            log::error!("failed to spawn evdev hotkey dispatch thread: {error}");
        });
    tx
}
const RESCAN_INTERVAL: Duration = Duration::from_secs(5);

pub(crate) fn keycode_name(code: u16) -> &'static str {
    const ESC: u16 = KeyCode::KEY_ESC.0;
    const ENTER: u16 = KeyCode::KEY_ENTER.0;
    const SPACE: u16 = KeyCode::KEY_SPACE.0;
    const TAB: u16 = KeyCode::KEY_TAB.0;
    const UP: u16 = KeyCode::KEY_UP.0;
    const DOWN: u16 = KeyCode::KEY_DOWN.0;
    const LEFT: u16 = KeyCode::KEY_LEFT.0;
    const RIGHT: u16 = KeyCode::KEY_RIGHT.0;
    const LEFTCTRL: u16 = KeyCode::KEY_LEFTCTRL.0;
    const RIGHTCTRL: u16 = KeyCode::KEY_RIGHTCTRL.0;
    const LEFTALT: u16 = KeyCode::KEY_LEFTALT.0;
    const RIGHTALT: u16 = KeyCode::KEY_RIGHTALT.0;
    const LEFTSHIFT: u16 = KeyCode::KEY_LEFTSHIFT.0;
    const RIGHTSHIFT: u16 = KeyCode::KEY_RIGHTSHIFT.0;
    const LEFTMETA: u16 = KeyCode::KEY_LEFTMETA.0;
    const RIGHTMETA: u16 = KeyCode::KEY_RIGHTMETA.0;
    const A: u16 = KeyCode::KEY_A.0;
    const Z: u16 = KeyCode::KEY_Z.0;
    match code {
        ESC => "esc",
        ENTER => "enter",
        SPACE => "space",
        TAB => "tab",
        UP => "up",
        DOWN => "down",
        LEFT => "left",
        RIGHT => "right",
        LEFTCTRL => "ctrl",
        RIGHTCTRL => "rctrl",
        LEFTALT => "alt",
        RIGHTALT => "ralt",
        LEFTSHIFT => "shift",
        RIGHTSHIFT => "rshift",
        LEFTMETA => "super",
        RIGHTMETA => "rsuper",
        _ => {
            if (A..=Z).contains(&code) {
                static LETTERS: [&str; 26] = [
                    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p",
                    "q", "r", "s", "t", "u", "v", "w", "x", "y", "z",
                ];
                LETTERS[(code - A) as usize]
            } else {
                "key"
            }
        }
    }
}

fn key_list(keys: &AttributeSet<KeyCode>) -> String {
    keys.iter()
        .map(|key| keycode_name(key.0))
        .collect::<Vec<_>>()
        .join(",")
}

fn event_before(event: &InputEvent, grab_time: SystemTime) -> bool {
    match event.timestamp().duration_since(UNIX_EPOCH) {
        Ok(event_since_epoch) => timestamp_before_ms(
            event_since_epoch.as_millis(),
            grab_time
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(u128::MAX),
        ),
        Err(_) => false,
    }
}

fn timestamp_before_ms(event_ms: u128, grab_ms: u128) -> bool {
    event_ms < grab_ms.saturating_sub(1)
}

#[cfg(debug_assertions)]
fn trace_capture_key(device: &str, keycode: u16, value: i32) {
    qol_runtime::probe!(
        "HOTKEY_CAPTURE",
        "event=key dev={} code={} value={}",
        device,
        keycode_name(keycode),
        value
    );
}

#[cfg(not(debug_assertions))]
fn trace_capture_key(_device: &str, _keycode: u16, _value: i32) {}

#[cfg(debug_assertions)]
fn trace_capture_emit(keycode: u16, value: i32) {
    qol_runtime::probe!(
        "HOTKEY_CAPTURE",
        "event=emit code={} value={}",
        keycode_name(keycode),
        value
    );
}

#[cfg(not(debug_assertions))]
fn trace_capture_emit(_keycode: u16, _value: i32) {}

#[cfg(debug_assertions)]
fn trace_capture_batch(device: &str, count: usize) {
    if count == 0 {
        qol_runtime::probe!("HOTKEY_CAPTURE", "event=syn_dropped dev={}", device);
    }
}

#[cfg(not(debug_assertions))]
fn trace_capture_batch(_device: &str, _count: usize) {}

#[cfg(debug_assertions)]
fn trace_capture_drop(device: &str, keycode: u16, value: i32, reason: &str) {
    qol_runtime::probe!(
        "HOTKEY_CAPTURE",
        "event=drop dev={} code={} value={} reason={}",
        device,
        keycode_name(keycode),
        value,
        reason
    );
}

#[cfg(not(debug_assertions))]
fn trace_capture_drop(_device: &str, _keycode: u16, _value: i32, _reason: &str) {}

#[cfg(debug_assertions)]
fn trace_capture_replay(device: &str, keycode: u16, value: i32) {
    qol_runtime::probe!(
        "HOTKEY_CAPTURE",
        "event=state_replay dev={} code={} value={}",
        device,
        keycode_name(keycode),
        value
    );
}

#[cfg(not(debug_assertions))]
fn trace_capture_replay(_device: &str, _keycode: u16, _value: i32) {}

/// Install a process-wide panic hook that aborts after logging. This ensures a
/// panic in any thread terminates the process, closes every grabbed-device fd,
/// and the kernel releases EVIOCGRAB. Without this, a panic in (say) the tray
/// thread would leave reader threads holding exclusive grabs and freeze the
/// keyboard until the process is killed externally. Idempotent.
fn install_panic_safety_hook() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let prior = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log::error!(
                "evdev capture active: panic detected; aborting to release keyboard grabs ({info})"
            );
            release_held_keys();
            prior(info);
            std::process::abort();
        }));
    });
}

pub(super) fn install(
    bindings: Vec<Binding>,
    on_fire: OnFire,
    reload_rx: Receiver<()>,
    rebuild: RebuildBindings,
) -> Result<()> {
    install_panic_safety_hook();

    let matcher = Arc::new(Mutex::new(BindingMatcher::new(bindings)));
    let grabbed = Arc::new(Mutex::new(HashSet::<PathBuf>::new()));
    let quarantined = Arc::new(Mutex::new(HashSet::<PathBuf>::new()));
    let keyboards = open_keyboards(true)?;
    if keyboards.is_empty() {
        anyhow::bail!("no keyboard input devices found under /dev/input");
    }
    let keyboard_count = keyboards.len();
    // The virtual keyboard has to exist before the first EVIOCGRAB. From the
    // instant a real keyboard is grabbed the session stops seeing it, so a key
    // held across that moment can only be released through the virtual device;
    // building it afterwards left a window whose stuck key autorepeated
    // forever. Capabilities come from the opened devices, which is why the
    // grab loop consumes paths and this reads the devices first.
    let key_caps = Arc::new(Mutex::new(merged_key_capabilities(
        keyboards
            .iter()
            .filter_map(|(_, device)| device.supported_keys()),
    )));
    let virtual_device = Arc::new(Mutex::new(TrackedVirtualDevice::new(build_virtual_device(
        &key_caps
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )?)));
    let _ = FLUSH_TARGET.set(virtual_device.clone());
    let physical_down = heal::physical_down_union(&keyboards);
    let to_grab = {
        let guard = grabbed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let quarantine = quarantined
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        devices_to_grab(
            &guard,
            &quarantine,
            keyboards.into_iter().map(|(path, _)| path).collect(),
        )
    };
    let mut grabbed_keyboards = Vec::new();
    let mut held_at_grab: Vec<u16> = Vec::new();
    for path in to_grab {
        let Some(keyboard) = grab_keyboard(path, &grabbed, &virtual_device) else {
            continue;
        };
        for code in keyboard.held.iter() {
            held_at_grab.push(code.0);
        }
        grabbed_keyboards.push(keyboard);
    }

    heal::heal_stuck_keys(
        &virtual_device,
        &physical_down,
        &key_caps
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );

    if grabbed_keyboards.is_empty() {
        anyhow::bail!(
            "evdev: found {keyboard_count} keyboard(s) but grabbed none (EVIOCGRAB denied; check input-group / udev permissions)"
        );
    }

    {
        let Ok(mut guard) = matcher.lock() else {
            anyhow::bail!("evdev: matcher lock poisoned during startup seeding");
        };
        guard.seed_held(held_at_grab);
    }

    let on_fire: Arc<dyn Fn(&CaptureEvent) + Send + Sync> = Arc::from(on_fire);
    let dispatch = spawn_dispatch_thread(on_fire);

    for keyboard in grabbed_keyboards {
        let matcher = matcher.clone();
        let virtual_device = virtual_device.clone();
        let dispatch = dispatch.clone();
        let grabbed = grabbed.clone();
        let quarantined = quarantined.clone();

        std::thread::spawn(move || {
            run_reader(
                keyboard.path,
                keyboard.device,
                keyboard.grab_time,
                keyboard.held,
                matcher,
                virtual_device,
                dispatch,
                grabbed,
                quarantined,
            );
        });
    }
    #[cfg(debug_assertions)]
    {
        let names: Vec<String> = open_keyboards(false)
            .map(|keyboards| {
                keyboards
                    .into_iter()
                    .map(|(path, device)| {
                        format!(
                            "{}={}",
                            path.file_name().unwrap_or_default().to_string_lossy(),
                            device.name().unwrap_or("unknown")
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        qol_runtime::probe!(
            "HOTKEY_CAPTURE",
            "event=install devices={}",
            names.join(",")
        );
    }

    spawn_reload_thread(matcher.clone(), reload_rx, rebuild, dispatch.clone());
    spawn_rescan_thread(
        grabbed,
        quarantined,
        matcher,
        virtual_device,
        key_caps,
        dispatch,
    );

    Ok(())
}

fn devices_to_grab(
    grabbed: &HashSet<PathBuf>,
    quarantined: &HashSet<PathBuf>,
    discovered: Vec<PathBuf>,
) -> Vec<PathBuf> {
    discovered
        .into_iter()
        .filter(|path| !grabbed.contains(path) && !quarantined.contains(path))
        .collect()
}

struct GrabbedKeyboard {
    path: PathBuf,
    device: RawDevice,
    grab_time: SystemTime,
    held: AttributeSet<KeyCode>,
}

/// Taking the virtual device by reference is the structural guarantee that no
/// keyboard can be grabbed before there is somewhere to release its held keys:
/// a grab is what stops the session seeing a device, so the release channel has
/// to already exist. Do not weaken this parameter to make a call site simpler.
fn grab_keyboard(
    path: PathBuf,
    grabbed: &Arc<Mutex<HashSet<PathBuf>>>,
    virtual_device: &Mutex<TrackedVirtualDevice>,
) -> Option<GrabbedKeyboard> {
    let Ok(mut device) = RawDevice::open(&path) else {
        log::warn!("evdev: failed to open {} (unplugged?)", path.display());
        return None;
    };
    let grab_time = SystemTime::now();
    if let Err(error) = device.grab() {
        log::warn!("evdev: failed to grab {}: {error}", path.display());
        return None;
    }
    log::info!("evdev: grabbed {}", path.display());
    let held = device.get_key_state().unwrap_or_default();
    #[cfg(debug_assertions)]
    {
        let device_name = device.name().unwrap_or("unknown").to_owned();
        if held.iter().next().is_some() {
            qol_runtime::probe!(
                "HOTKEY_CAPTURE",
                "event=held_at_grab dev={} keys={}",
                device_name,
                key_list(&held)
            );
        }
    }
    {
        let mut guard = grabbed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !guard.insert(path.clone()) {
            let _ = device.ungrab();
            return None;
        }
    }
    // The session saw these as presses it will never see released, because the
    // release now lands on the grabbed device. Cancel them or it repeats them
    // forever.
    emit_key_ups(virtual_device, held.iter().map(|key| key.0));
    Some(GrabbedKeyboard {
        path,
        device,
        grab_time,
        held,
    })
}

fn spawn_rescan_thread(
    grabbed: Arc<Mutex<HashSet<PathBuf>>>,
    quarantined: Arc<Mutex<HashSet<PathBuf>>>,
    matcher: Arc<Mutex<BindingMatcher>>,
    virtual_device: Arc<Mutex<TrackedVirtualDevice>>,
    key_caps: Arc<Mutex<AttributeSet<KeyCode>>>,
    dispatch: CaptureDispatch,
) {
    std::thread::Builder::new()
        .name("hotkey-capture-linux-rescan".into())
        .spawn(move || loop {
            std::thread::sleep(RESCAN_INTERVAL);
            let Ok(keyboards) = open_keyboards(false) else {
                log::warn!("evdev: rescan discovery failed");
                continue;
            };
            let to_grab = {
                let guard = grabbed
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let quarantine = quarantined
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                devices_to_grab(
                    &guard,
                    &quarantine,
                    keyboards.into_iter().map(|(path, _)| path).collect(),
                )
            };
            for path in to_grab {
                let Some(keyboard) = grab_keyboard(path, &grabbed, &virtual_device) else {
                    continue;
                };
                log::info!("evdev: rescan grabbed {}", keyboard.path.display());
                if let Ok(mut guard) = matcher.lock() {
                    guard.seed_held(keyboard.held.iter().map(|key| key.0));
                }
                merge_capabilities(&keyboard, &key_caps, &virtual_device, &keyboard.path);
                let matcher = matcher.clone();
                let virtual_device = virtual_device.clone();
                let dispatch = dispatch.clone();
                let grabbed = grabbed.clone();
                let quarantined = quarantined.clone();
                let path = keyboard.path;
                let path_display = path.display().to_string();
                let spawn_result = std::thread::Builder::new()
                    .name("hotkey-capture-linux-reader".into())
                    .spawn(move || {
                        run_reader(
                            path,
                            keyboard.device,
                            keyboard.grab_time,
                            keyboard.held,
                            matcher,
                            virtual_device,
                            dispatch,
                            grabbed,
                            quarantined,
                        );
                    });
                if let Err(error) = spawn_result {
                    log::error!("evdev: failed to spawn reader thread for {path_display}: {error}");
                }
            }
        })
        .map(|_| ())
        .unwrap_or_else(|error| {
            log::error!("failed to spawn evdev hotkey rescan thread: {error}");
        });
}

fn merge_capabilities(
    keyboard: &GrabbedKeyboard,
    key_caps: &Mutex<AttributeSet<KeyCode>>,
    virtual_device: &Mutex<TrackedVirtualDevice>,
    path: &std::path::Path,
) {
    let Ok(mut caps) = key_caps.lock() else {
        log::error!("evdev: key caps lock poisoned; skipping capability merge");
        return;
    };
    let before = caps.iter().count();
    if let Some(keys) = keyboard.device.supported_keys() {
        for code in keys.iter() {
            caps.insert(code);
        }
    }
    if caps.iter().count() == before {
        return;
    }
    match build_virtual_device(&caps) {
        Ok(new_device) => {
            if let Ok(mut vd) = virtual_device.lock() {
                vd.device = new_device;
                log::info!(
                    "evdev: virtual device rebuilt with {} key capabilities after grabbing {}",
                    caps.iter().count(),
                    path.display()
                );
            }
        }
        Err(error) => {
            log::error!(
                "evdev: virtual device rebuild failed after grabbing {}: {error}",
                path.display()
            );
        }
    }
}

fn spawn_reload_thread(
    matcher: Arc<Mutex<BindingMatcher>>,
    reload_rx: Receiver<()>,
    rebuild: RebuildBindings,
    dispatch: CaptureDispatch,
) {
    std::thread::Builder::new()
        .name("hotkey-capture-linux-reload".into())
        .spawn(move || {
            while reload_rx.recv().is_ok() {
                while reload_rx.try_recv().is_ok() {}
                let bindings = match rebuild() {
                    Ok(bindings) => bindings,
                    Err(error) => {
                        log::error!(
                            "linux evdev hotkey reload skipped; keeping current bindings: {error:#}"
                        );
                        continue;
                    }
                };
                let stopped = match matcher.lock() {
                    Ok(mut guard) => {
                        let stopped = guard.reload(bindings);
                        log::info!("linux evdev hotkey capture: bindings reloaded");
                        stopped
                    }
                    Err(poisoned) => {
                        log::error!(
                            "linux evdev matcher lock poisoned during reload; recovering: {poisoned}"
                        );
                        let mut guard = poisoned.into_inner();
                        guard.reload(bindings)
                    }
                };
                for event in stopped {
                    let _ = dispatch.send(event);
                }
            }
        })
        .map(|_| ())
        .unwrap_or_else(|error| {
            log::error!("failed to spawn evdev hotkey reload thread: {error}");
        });
}

#[allow(clippy::too_many_arguments)]
fn run_reader(
    path: PathBuf,
    mut device: RawDevice,
    grab_time: SystemTime,
    initial_held: AttributeSet<KeyCode>,
    matcher: Arc<Mutex<BindingMatcher>>,
    virtual_device: Arc<Mutex<TrackedVirtualDevice>>,
    dispatch: CaptureDispatch,
    grabbed: Arc<Mutex<HashSet<PathBuf>>>,
    quarantined: Arc<Mutex<HashSet<PathBuf>>>,
) {
    let device_name = device.name().unwrap_or("unknown").to_owned();
    let mut reader_held = ReaderHeldKeys::seeded(initial_held.iter().map(|key| key.0));
    'read_loop: loop {
        let events = match device.fetch_events() {
            Ok(events) => events,
            Err(error) => {
                log::warn!("evdev: read error on {}: {error}", path.display());
                break;
            }
        };
        let batch: Vec<InputEvent> = events.collect();
        trace_capture_batch(&device_name, batch.len());
        for event in batch {
            if releases_grab(&event) {
                log::warn!(
                    "evdev: {device_name} ({}) emitted {:?} while grabbed; releasing it",
                    path.display(),
                    event.event_type()
                );
                quarantined
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(path.clone());
                break 'read_loop;
            }
            let EventSummary::Synchronization(_, SynchronizationCode::SYN_DROPPED, _) =
                event.destructure()
            else {
                process_event(
                    event,
                    &matcher,
                    &virtual_device,
                    &dispatch,
                    &device_name,
                    grab_time,
                    &mut reader_held,
                );
                continue;
            };
            match device.get_key_state() {
                Ok(state) => {
                    let current: HashSet<u16> = state.iter().map(|key| key.0).collect();
                    let (became_down, became_up) = reader_held.resync(current);
                    for code in &became_down {
                        replay_state(&virtual_device, &matcher, &dispatch, &device_name, *code, 1);
                    }
                    for code in &became_up {
                        replay_state(&virtual_device, &matcher, &dispatch, &device_name, *code, 0);
                    }
                }
                Err(error) => {
                    log::warn!("evdev: state resync failed on {}: {error}", path.display());
                }
            }
        }
    }
    if let Err(error) = device.ungrab() {
        log::warn!("evdev: ungrab on reader exit failed: {error}");
    }
    grabbed
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&path);
    for code in reader_held.exit_release_codes() {
        replay_state(&virtual_device, &matcher, &dispatch, &device_name, code, 0);
    }
}

/// `pressed` is what this reader is responsible for releasing when it exits:
/// keys it saw go down itself. `mirror` is the device's raw key state, kept
/// only to diff against after a SYN_DROPPED. Keys already held at grab time are
/// deliberately absent from `pressed`, because `release_seeded_keys` has
/// already cancelled them in the session.
#[derive(Default)]
struct ReaderHeldKeys {
    pressed: HashSet<u16>,
    mirror: HashSet<u16>,
}

impl ReaderHeldKeys {
    fn seeded(initial: impl IntoIterator<Item = u16>) -> Self {
        Self {
            pressed: HashSet::new(),
            mirror: initial.into_iter().collect(),
        }
    }

    fn observe(&mut self, code: u16, value: i32) {
        if value == 0 {
            self.pressed.remove(&code);
            self.mirror.remove(&code);
        } else {
            self.mirror.insert(code);
            if value == 1 {
                self.pressed.insert(code);
            }
        }
    }

    /// Returns the (became_down, became_up) diff against the mirror and adopts
    /// `current` as the new mirror.
    fn resync(&mut self, current: HashSet<u16>) -> (Vec<u16>, Vec<u16>) {
        let became_down: Vec<u16> = current.difference(&self.mirror).copied().collect();
        let became_up: Vec<u16> = self.mirror.difference(&current).copied().collect();
        for code in &became_down {
            self.pressed.insert(*code);
        }
        for code in &became_up {
            self.pressed.remove(code);
        }
        self.mirror = current;
        (became_down, became_up)
    }

    fn exit_release_codes(&self) -> Vec<u16> {
        let mut codes: Vec<u16> = self.pressed.iter().copied().collect();
        codes.sort_unstable();
        codes
    }
}

fn replay_state(
    virtual_device: &Mutex<TrackedVirtualDevice>,
    matcher: &Mutex<BindingMatcher>,
    dispatch: &CaptureDispatch,
    device: &str,
    code: u16,
    value: i32,
) {
    trace_capture_replay(device, code, value);
    let events = match matcher.lock() {
        Ok(mut guard) => guard.reconcile(code, value),
        Err(_) => Vec::new(),
    };
    for capture_event in events {
        let _ = dispatch.send(capture_event);
    }
    let key_event = InputEvent::new(EventType::KEY.0, code, value);
    let sync_event = InputEvent::new(EventType::SYNCHRONIZATION.0, 0, 0);
    if let Ok(mut vd) = virtual_device.lock() {
        if let Err(error) = vd.emit(&[key_event, sync_event]) {
            log::warn!("evdev: virtual emit failed: {error}");
        }
    }
}

fn process_event(
    event: InputEvent,
    matcher: &Mutex<BindingMatcher>,
    virtual_device: &Mutex<TrackedVirtualDevice>,
    dispatch: &CaptureDispatch,
    device: &str,
    grab_time: SystemTime,
    reader_held: &mut ReaderHeldKeys,
) {
    let EventSummary::Key(_, key_code, value) = event.destructure() else {
        forward(event, virtual_device);
        return;
    };
    if event_before(&event, grab_time) {
        trace_capture_drop(device, key_code.0, value, "pre_grab");
        return;
    }
    trace_capture_key(device, key_code.0, value);
    reader_held.observe(key_code.0, value);
    let decision = match matcher.lock() {
        Ok(mut m) => m.observe(key_code.0, value),
        Err(_) => {
            forward(event, virtual_device);
            return;
        }
    };
    if decision.forward {
        forward(event, virtual_device);
        trace_capture_emit(key_code.0, value);
    }
    for capture_event in decision.events {
        log::info!(
            "evdev: hotkey phase {:?} {} -> {}::{}",
            capture_event.phase,
            capture_event.binding.raw_key,
            capture_event.binding.plugin_uid.as_str(),
            capture_event.binding.action
        );
        // never call plugin code from the reader thread; see CaptureDispatch.
        let _ = dispatch.send(capture_event);
    }
}

fn forward(event: InputEvent, virtual_device: &Mutex<TrackedVirtualDevice>) {
    if let Ok(mut vd) = virtual_device.lock() {
        if let Err(error) = vd.emit(&[event]) {
            log::warn!("evdev: virtual emit failed: {error}");
        }
    }
}

fn releases_grab(event: &InputEvent) -> bool {
    matches!(
        event.event_type(),
        EventType::RELATIVE | EventType::ABSOLUTE
    )
}

fn open_keyboards(log_skips: bool) -> Result<Vec<(PathBuf, RawDevice)>> {
    let mut keyboards = Vec::new();
    for (path, device) in evdev::enumerate() {
        let caps = DeviceCapabilities::of(&device);
        if let DeviceClass::Skipped(reason) = classify::classify(&caps) {
            if !matches!(
                reason,
                SkipReason::NoKeyboardKeys | SkipReason::VirtualKeyboard
            ) {
                if log_skips {
                    log::info!(
                        "evdev: skipped {} ({}): {reason}",
                        caps.name,
                        path.display()
                    );
                } else {
                    log::debug!(
                        "evdev: skipped {} ({}): {reason}",
                        caps.name,
                        path.display()
                    );
                }
            }
            continue;
        }
        let Ok(device) = RawDevice::open(&path) else {
            log::warn!(
                "evdev: failed to open {} during discovery (unplugged?)",
                path.display()
            );
            continue;
        };
        keyboards.push((path, device));
    }
    Ok(keyboards)
}

fn merged_key_capabilities<'a>(
    sources: impl IntoIterator<Item = &'a AttributeSetRef<KeyCode>>,
) -> AttributeSet<KeyCode> {
    sources
        .into_iter()
        .flat_map(AttributeSetRef::iter)
        .collect()
}

fn build_virtual_device(keys: &AttributeSet<KeyCode>) -> Result<VirtualDevice> {
    build_virtual_device_named(keys, classify::VIRTUAL_KEYBOARD_NAME)
}

fn build_virtual_device_named(keys: &AttributeSet<KeyCode>, name: &str) -> Result<VirtualDevice> {
    let mut device = VirtualDevice::builder()
        .context("creating uinput builder (is /dev/uinput accessible?)")?
        .name(name)
        .with_keys(keys)
        .context("declaring key capabilities on uinput device")?
        .build()
        .context("registering uinput virtual device")?;
    for path in device
        .enumerate_dev_nodes_blocking()
        .into_iter()
        .flatten()
        .flatten()
    {
        log::info!("evdev: virtual device created at {}", path.display());
    }
    Ok(device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_keyboard_supports_non_us_and_lock_keys() {
        let primary = AttributeSet::from_iter([
            KeyCode::KEY_ESC,
            KeyCode::KEY_A,
            KeyCode::KEY_CAPSLOCK,
            KeyCode::KEY_102ND,
        ]);
        let secondary = AttributeSet::from_iter([
            KeyCode::KEY_LEFTCTRL,
            KeyCode::KEY_RIGHTCTRL,
            KeyCode::KEY_KATAKANAHIRAGANA,
        ]);
        let sources: [&AttributeSetRef<KeyCode>; 2] = [&primary, &secondary];
        let keys = merged_key_capabilities(sources);

        for key in [
            KeyCode::KEY_CAPSLOCK,
            KeyCode::KEY_102ND,
            KeyCode::KEY_LEFTCTRL,
            KeyCode::KEY_RIGHTCTRL,
            KeyCode::KEY_KATAKANAHIRAGANA,
        ] {
            assert!(keys.contains(key), "missing virtual keyboard key {key:?}");
        }
    }

    #[test]
    fn devices_to_grab_returns_replugged_nodes_only() {
        let grabbed: HashSet<PathBuf> = HashSet::from([PathBuf::from("/dev/input/event3")]);
        let quarantined: HashSet<PathBuf> = HashSet::from([PathBuf::from("/dev/input/event5")]);
        let discovered = vec![
            PathBuf::from("/dev/input/event3"),
            PathBuf::from("/dev/input/event5"),
            PathBuf::from("/dev/input/event30"),
        ];
        assert_eq!(
            devices_to_grab(&grabbed, &quarantined, discovered),
            vec![PathBuf::from("/dev/input/event30")]
        );
    }

    #[test]
    fn devices_to_grab_is_idempotent() {
        let grabbed: HashSet<PathBuf> = HashSet::from([PathBuf::from("/dev/input/event3")]);
        let quarantined: HashSet<PathBuf> = HashSet::new();
        let discovered = vec![PathBuf::from("/dev/input/event3")];
        assert!(devices_to_grab(&grabbed, &quarantined, discovered).is_empty());
    }

    #[test]
    fn releases_grab_for_relative_and_absolute_events_only() {
        let relative = InputEvent::new(EventType::RELATIVE.0, 0, 1);
        let absolute = InputEvent::new(EventType::ABSOLUTE.0, 0, 1);
        let key = InputEvent::new(EventType::KEY.0, 38, 1);
        let sync = InputEvent::new(EventType::SYNCHRONIZATION.0, 0, 0);
        assert!(releases_grab(&relative));
        assert!(releases_grab(&absolute));
        assert!(!releases_grab(&key));
        assert!(!releases_grab(&sync));
    }

    fn capture_event(action: &str) -> CaptureEvent {
        CaptureEvent {
            binding: Binding {
                combo: None,
                plugin_uid: crate::plugins::PluginUid::new("test-plugin"),
                action: action.to_owned(),
                raw_key: "F12".to_owned(),
                continuous: false,
            },
            phase: super::super::super::super::Phase::START,
        }
    }

    #[test]
    fn tracked_held_set_follows_emitted_downs_and_ups() {
        let mut held = HashSet::new();
        let down = |code: u16| InputEvent::new(EventType::KEY.0, code, 1);
        let up = |code: u16| InputEvent::new(EventType::KEY.0, code, 0);
        let syn = InputEvent::new(EventType::SYNCHRONIZATION.0, 0, 0);

        track_emitted(&mut held, &[down(38), syn, down(42), syn]);
        assert_eq!(held, HashSet::from([38, 42]));
        track_emitted(&mut held, &[InputEvent::new(EventType::KEY.0, 38, 2), syn]);
        assert_eq!(
            held,
            HashSet::from([38, 42]),
            "autorepeat does not change held state"
        );
        track_emitted(&mut held, &[up(38), syn]);
        assert_eq!(held, HashSet::from([42]));
    }

    /// The letter latched by an exec-restart mid-keypress: the flush must
    /// emit an up (with a SYN) for every key still down on the virtual
    /// keyboard, else X autorepeats it until the machine is power-cycled.
    #[test]
    fn release_events_emit_an_up_and_a_syn_for_every_held_key() {
        let held = HashSet::from([38u16]);
        let events = release_events(&held);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type(), EventType::KEY);
        assert_eq!((events[0].code(), events[0].value()), (38, 0));
        assert_eq!(events[1].event_type(), EventType::SYNCHRONIZATION);
        assert!(release_events(&HashSet::new()).is_empty());
    }

    /// A slow plugin action must never hold up the reader thread. Before the
    /// dispatch channel existed, `on_fire` ran inline in the read loop, so a
    /// cold-daemon action froze every grabbed keyboard for its full duration and
    /// then flushed the kernel's backlog into the desktop in one burst.
    #[test]
    fn dispatch_returns_immediately_while_the_handler_is_still_blocked() {
        let entered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handler_entered = entered.clone();
        let dispatch = spawn_dispatch_thread(Arc::new(move |_: &CaptureEvent| {
            handler_entered.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(500));
        }));

        let started = std::time::Instant::now();
        for index in 0..10 {
            dispatch
                .send(capture_event(&format!("action-{index}")))
                .expect("dispatch must accept the event");
        }
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_millis(100),
            "sending 10 events took {elapsed:?}; the reader thread was blocked by the handler"
        );
        assert!(
            entered.load(std::sync::atomic::Ordering::SeqCst) <= 1,
            "the handler should still be working on the first event"
        );
    }

    #[test]
    fn reader_exit_releases_only_keys_its_own_stream_pressed() {
        let shift = KeyCode::KEY_LEFTSHIFT.0;
        let mut external = ReaderHeldKeys::default();
        let builtin = ReaderHeldKeys::default();

        external.observe(shift, 1);

        assert_eq!(external.exit_release_codes(), vec![shift]);
        assert!(
            builtin.exit_release_codes().is_empty(),
            "a reader whose device only had the key down at grab must not release it"
        );
    }

    #[test]
    fn seeded_key_repressed_by_its_device_is_released_at_exit() {
        let shift = KeyCode::KEY_LEFTSHIFT.0;
        let mut reader = ReaderHeldKeys::default();

        reader.observe(shift, 1);
        reader.observe(shift, 0);
        reader.observe(shift, 1);

        assert_eq!(reader.exit_release_codes(), vec![shift]);
    }

    #[test]
    fn resync_updates_exit_responsibility_in_both_directions() {
        let mut reader = ReaderHeldKeys::default();
        reader.observe(30, 1);

        let (became_down, became_up) = reader.resync(HashSet::from([31]));

        assert_eq!(became_down, vec![31]);
        assert_eq!(became_up, vec![30]);
        assert_eq!(reader.exit_release_codes(), vec![31]);
    }

    #[test]
    fn resync_diffs_against_the_state_present_at_grab() {
        let mut reader = ReaderHeldKeys::seeded([30]);

        let (became_down, became_up) = reader.resync(HashSet::from([30]));

        assert!(became_down.is_empty(), "a key held since grab is not new");
        assert!(became_up.is_empty());
        assert!(
            reader.exit_release_codes().is_empty(),
            "a key held since grab was already cancelled by release_seeded_keys"
        );
    }

    #[test]
    fn timestamp_before_drops_clearly_older_events_and_keeps_recent_and_newer() {
        assert!(timestamp_before_ms(1000, 2000));
        assert!(!timestamp_before_ms(1999, 2000));
        assert!(!timestamp_before_ms(2000, 2000));
        assert!(!timestamp_before_ms(2500, 2000));
        assert!(!timestamp_before_ms(0, 0));
        assert!(!timestamp_before_ms(0, 1));
    }

    #[test]
    fn stuck_candidates_are_x_down_minus_physical_down_sorted() {
        let empty: HashSet<u16> = HashSet::new();
        assert!(super::heal::stuck_candidates(&empty, &empty).is_empty());

        let x_down = HashSet::from([30u16, 31, 32]);
        assert_eq!(
            super::heal::stuck_candidates(&x_down, &empty),
            vec![30, 31, 32]
        );
        assert!(super::heal::stuck_candidates(&empty, &x_down).is_empty());

        let physical = HashSet::from([31u16]);
        assert_eq!(
            super::heal::stuck_candidates(&x_down, &physical),
            vec![30, 32]
        );

        let disjoint_x = HashSet::from([45u16, 10]);
        let disjoint_physical = HashSet::from([46u16]);
        assert_eq!(
            super::heal::stuck_candidates(&disjoint_x, &disjoint_physical),
            vec![10, 45]
        );
    }

    #[test]
    fn loopback_virtual_device_relays_the_heal_key_up() {
        let keys = AttributeSet::from_iter([KeyCode::KEY_DOWN, KeyCode::KEY_A]);
        let mut device = match build_virtual_device_named(&keys, "qol-tray-loopback-test") {
            Ok(device) => device,
            Err(error) => {
                eprintln!("skipping loopback stuck-key test: {error}");
                return;
            }
        };
        let Some(node_path) = device
            .enumerate_dev_nodes_blocking()
            .into_iter()
            .flatten()
            .flatten()
            .next()
        else {
            eprintln!("skipping loopback stuck-key test: no dev node enumerated");
            return;
        };
        use std::os::unix::fs::OpenOptionsExt;
        let open_deadline = std::time::Instant::now() + Duration::from_secs(3);
        let node_file = loop {
            let attempt = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&node_path)
                .or_else(|_| {
                    std::fs::OpenOptions::new()
                        .read(true)
                        .custom_flags(libc::O_NONBLOCK)
                        .open(&node_path)
                });
            match attempt {
                Ok(file) => break file,
                Err(error) => {
                    if std::time::Instant::now() >= open_deadline {
                        eprintln!("skipping loopback stuck-key test: {error}");
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        };
        let mut node = RawDevice::from_fd(node_file.into()).expect("open loopback uinput node");
        node.grab().expect("grab loopback uinput node");

        device
            .emit(&[
                InputEvent::new(EventType::KEY.0, KeyCode::KEY_DOWN.0, 1),
                InputEvent::new(EventType::SYNCHRONIZATION.0, 0, 0),
            ])
            .expect("virtual emit on loopback test device");

        let state = node.get_key_state().expect("loopback key state");
        assert!(
            state.contains(KeyCode::KEY_DOWN),
            "loopback node must report KEY_DOWN after the virtual press"
        );

        let tracked = Mutex::new(TrackedVirtualDevice::new(device));
        tracked
            .lock()
            .expect("lock loopback tracked device")
            .held
            .insert(KeyCode::KEY_DOWN.0);
        emit_key_ups(&tracked, [KeyCode::KEY_DOWN.0]);

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(mut events) = node.fetch_events() {
                let found = events.any(|event| {
                    event.event_type() == EventType::KEY
                        && event.code() == KeyCode::KEY_DOWN.0
                        && event.value() == 0
                });
                if found {
                    break;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "loopback node never received the healed KEY_DOWN key-up"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        let _ = node.ungrab();
        drop(node);
    }
}
