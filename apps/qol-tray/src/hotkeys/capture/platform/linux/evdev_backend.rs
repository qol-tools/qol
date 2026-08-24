use super::super::super::{Binding, CaptureEvent};
use super::super::super::{OnFire, RebuildBindings};
use super::matcher::BindingMatcher;
use anyhow::{Context, Result};
use crossbeam_channel::Receiver;
use evdev::raw_stream::RawDevice;
use evdev::{
    uinput::VirtualDevice, AttributeSet, AttributeSetRef, Device, EventSummary, EventType,
    InputEvent, KeyCode, SynchronizationCode,
};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const VIRTUAL_KEYBOARD_NAME: &str = "qol-tray-virtual-keyboard";
const RESCAN_INTERVAL: Duration = Duration::from_secs(5);

pub(super) fn keycode_name(code: u16) -> &'static str {
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
    let grabbed = Arc::new(Mutex::new(HashMap::<PathBuf, HashSet<u16>>::new()));
    let keyboards = open_keyboards()?;
    if keyboards.is_empty() {
        anyhow::bail!("no keyboard input devices found under /dev/input");
    }
    let keyboard_count = keyboards.len();
    let to_grab = {
        let guard = grabbed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let paths: HashSet<PathBuf> = guard.keys().cloned().collect();
        devices_to_grab(
            &paths,
            keyboards.into_iter().map(|(path, _)| path).collect(),
        )
    };
    let mut grabbed_keyboards = Vec::new();
    let mut held_at_grab: Vec<u16> = Vec::new();
    for path in to_grab {
        let Some(keyboard) = grab_keyboard(path, &grabbed) else {
            continue;
        };
        for code in keyboard.held.iter() {
            held_at_grab.push(code.0);
        }
        grabbed_keyboards.push(keyboard);
    }

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

    let key_caps = Arc::new(Mutex::new(merged_key_capabilities(
        grabbed_keyboards
            .iter()
            .filter_map(|keyboard| keyboard.device.supported_keys()),
    )));
    let virtual_device = Arc::new(Mutex::new(build_virtual_device(
        &key_caps
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )?));
    let on_fire: Arc<dyn Fn(&CaptureEvent) + Send + Sync> = Arc::from(on_fire);

    for keyboard in grabbed_keyboards {
        let matcher = matcher.clone();
        let virtual_device = virtual_device.clone();
        let on_fire = on_fire.clone();
        let grabbed = grabbed.clone();

        std::thread::spawn(move || {
            run_reader(
                keyboard.path,
                keyboard.device,
                keyboard.grab_time,
                keyboard.held,
                matcher,
                virtual_device,
                on_fire,
                grabbed,
            );
        });
    }
    #[cfg(debug_assertions)]
    {
        let names: Vec<String> = open_keyboards()
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

    spawn_reload_thread(matcher.clone(), reload_rx, rebuild, on_fire.clone());
    spawn_rescan_thread(grabbed, matcher, virtual_device, key_caps, on_fire);

    Ok(())
}

fn devices_to_grab(grabbed: &HashSet<PathBuf>, discovered: Vec<PathBuf>) -> Vec<PathBuf> {
    discovered
        .into_iter()
        .filter(|path| !grabbed.contains(path))
        .collect()
}

struct GrabbedKeyboard {
    path: PathBuf,
    device: RawDevice,
    grab_time: SystemTime,
    held: AttributeSet<KeyCode>,
}

fn grab_keyboard(
    path: PathBuf,
    grabbed: &Arc<Mutex<HashMap<PathBuf, HashSet<u16>>>>,
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
        let held_codes: HashSet<u16> = held.iter().map(|key| key.0).collect();
        if guard.insert(path.clone(), held_codes).is_some() {
            let _ = device.ungrab();
            return None;
        }
    }
    Some(GrabbedKeyboard {
        path,
        device,
        grab_time,
        held,
    })
}

fn spawn_rescan_thread(
    grabbed: Arc<Mutex<HashMap<PathBuf, HashSet<u16>>>>,
    matcher: Arc<Mutex<BindingMatcher>>,
    virtual_device: Arc<Mutex<VirtualDevice>>,
    key_caps: Arc<Mutex<AttributeSet<KeyCode>>>,
    on_fire: Arc<dyn Fn(&CaptureEvent) + Send + Sync>,
) {
    std::thread::Builder::new()
        .name("hotkey-capture-linux-rescan".into())
        .spawn(move || loop {
            std::thread::sleep(RESCAN_INTERVAL);
            let Ok(keyboards) = open_keyboards() else {
                log::warn!("evdev: rescan discovery failed");
                continue;
            };
            let to_grab = {
                let guard = grabbed
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let paths: HashSet<PathBuf> = guard.keys().cloned().collect();
                devices_to_grab(
                    &paths,
                    keyboards.into_iter().map(|(path, _)| path).collect(),
                )
            };
            for path in to_grab {
                let Some(keyboard) = grab_keyboard(path, &grabbed) else {
                    continue;
                };
                log::info!("evdev: rescan grabbed {}", keyboard.path.display());
                if let Ok(mut guard) = matcher.lock() {
                    guard.seed_held(keyboard.held.iter().map(|key| key.0));
                }
                merge_capabilities(&keyboard, &key_caps, &virtual_device, &keyboard.path);
                let matcher = matcher.clone();
                let virtual_device = virtual_device.clone();
                let on_fire = on_fire.clone();
                let grabbed = grabbed.clone();
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
                            on_fire,
                            grabbed,
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
    virtual_device: &Mutex<VirtualDevice>,
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
                *vd = new_device;
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
    on_fire: Arc<dyn Fn(&CaptureEvent) + Send + Sync>,
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
                    on_fire(&event);
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
    virtual_device: Arc<Mutex<VirtualDevice>>,
    on_fire: Arc<dyn Fn(&CaptureEvent) + Send + Sync>,
    grabbed: Arc<Mutex<HashMap<PathBuf, HashSet<u16>>>>,
) {
    let device_name = device.name().unwrap_or("unknown").to_owned();
    let mut known_held: HashSet<u16> = initial_held.iter().map(|key| key.0).collect();
    loop {
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
            let EventSummary::Synchronization(_, SynchronizationCode::SYN_DROPPED, _) =
                event.destructure()
            else {
                process_event(
                    event,
                    &matcher,
                    &virtual_device,
                    on_fire.as_ref(),
                    &device_name,
                    grab_time,
                    &mut known_held,
                );
                continue;
            };
            match device.get_key_state() {
                Ok(state) => {
                    let current: HashSet<u16> = state.iter().map(|key| key.0).collect();
                    for code in current.difference(&known_held).copied() {
                        replay_state(&virtual_device, &matcher, &device_name, code, 1);
                    }
                    for code in known_held.difference(&current).copied() {
                        replay_state(&virtual_device, &matcher, &device_name, code, 0);
                    }
                    known_held = current;
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
    let to_release: Vec<u16> = {
        let mut guard = grabbed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let re_seeded: HashSet<u16> = guard.get(&path).cloned().unwrap_or_default();
        let to_release: Vec<u16> = known_held.difference(&re_seeded).copied().collect();
        guard.remove(&path);
        to_release
    };
    for code in to_release {
        replay_state(&virtual_device, &matcher, &device_name, code, 0);
    }
}

fn replay_state(
    virtual_device: &Mutex<VirtualDevice>,
    matcher: &Mutex<BindingMatcher>,
    device: &str,
    code: u16,
    value: i32,
) {
    trace_capture_replay(device, code, value);
    if let Ok(mut guard) = matcher.lock() {
        guard.reconcile(code, value);
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
    virtual_device: &Mutex<VirtualDevice>,
    on_fire: &dyn Fn(&CaptureEvent),
    device: &str,
    grab_time: SystemTime,
    known_held: &mut HashSet<u16>,
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
    if value == 0 {
        known_held.remove(&key_code.0);
    } else if value == 1 {
        known_held.insert(key_code.0);
    }
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
        on_fire(&capture_event);
    }
}

fn forward(event: InputEvent, virtual_device: &Mutex<VirtualDevice>) {
    if let Ok(mut vd) = virtual_device.lock() {
        if let Err(error) = vd.emit(&[event]) {
            log::warn!("evdev: virtual emit failed: {error}");
        }
    }
}

fn open_keyboards() -> Result<Vec<(PathBuf, RawDevice)>> {
    let mut keyboards = Vec::new();
    for (path, device) in evdev::enumerate() {
        if !is_keyboard(&device) {
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

fn is_keyboard(device: &Device) -> bool {
    if device.name() == Some(VIRTUAL_KEYBOARD_NAME) {
        return false;
    }
    let Some(keys) = device.supported_keys() else {
        return false;
    };
    keys.contains(KeyCode::KEY_ESC) && keys.contains(KeyCode::KEY_A)
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
    let mut device = VirtualDevice::builder()
        .context("creating uinput builder (is /dev/uinput accessible?)")?
        .name(VIRTUAL_KEYBOARD_NAME)
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
        let discovered = vec![
            PathBuf::from("/dev/input/event3"),
            PathBuf::from("/dev/input/event30"),
        ];
        assert_eq!(
            devices_to_grab(&grabbed, discovered),
            vec![PathBuf::from("/dev/input/event30")]
        );
    }

    #[test]
    fn devices_to_grab_is_idempotent() {
        let grabbed: HashSet<PathBuf> = HashSet::from([PathBuf::from("/dev/input/event3")]);
        let discovered = vec![PathBuf::from("/dev/input/event3")];
        assert!(devices_to_grab(&grabbed, discovered).is_empty());
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
}
