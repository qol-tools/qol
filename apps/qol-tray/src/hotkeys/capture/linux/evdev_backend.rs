//! evdev + uinput implementation. Only compiled with `linux_evdev` feature.

use super::super::{keycodes, Binding, BindingMatcher, CaptureDecision};
use anyhow::{Context, Result};
use evdev::{uinput::VirtualDevice, AttributeSet, Device, EventSummary, InputEvent, KeyCode};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};

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

/// Open every keyboard device, grab it, build the shared virtual device,
/// and spawn one reader thread per grabbed device. Reader threads are
/// detached on spawn — they own their `Device` and live until the process
/// dies (which closes the fd and releases EVIOCGRAB).
pub(super) fn install(
    bindings: Vec<Binding>,
    on_fire: Box<dyn Fn(&Binding) + Send + Sync>,
) -> Result<()> {
    install_panic_safety_hook();

    let matcher = Arc::new(Mutex::new(BindingMatcher::new(bindings)));
    let key_caps = matcher_keycodes_as_attribute_set(&matcher.lock().unwrap());
    let virtual_device = build_virtual_device(&key_caps)?;
    let virtual_device = Arc::new(Mutex::new(virtual_device));

    let keyboards = open_keyboards()?;
    if keyboards.is_empty() {
        anyhow::bail!("no keyboard input devices found under /dev/input");
    }

    let on_fire: Arc<dyn Fn(&Binding) + Send + Sync> = Arc::from(on_fire);

    for (path, mut device) in keyboards {
        if let Err(error) = device.grab() {
            log::warn!("evdev: failed to grab {}: {error}", path.display());
            continue;
        }
        log::info!("evdev: grabbed {}", path.display());

        let matcher = matcher.clone();
        let virtual_device = virtual_device.clone();
        let on_fire = on_fire.clone();

        std::thread::spawn(move || {
            run_reader(path, device, matcher, virtual_device, on_fire);
        });
    }

    Ok(())
}

fn run_reader(
    path: PathBuf,
    mut device: Device,
    matcher: Arc<Mutex<BindingMatcher>>,
    virtual_device: Arc<Mutex<VirtualDevice>>,
    on_fire: Arc<dyn Fn(&Binding) + Send + Sync>,
) {
    loop {
        let events = match device.fetch_events() {
            Ok(events) => events,
            Err(error) => {
                log::warn!("evdev: read error on {}: {error}", path.display());
                break;
            }
        };
        for event in events {
            process_event(event, &matcher, &virtual_device, on_fire.as_ref());
        }
    }
    if let Err(error) = device.ungrab() {
        log::warn!("evdev: ungrab on reader exit failed: {error}");
    }
}

fn process_event(
    event: InputEvent,
    matcher: &Mutex<BindingMatcher>,
    virtual_device: &Mutex<VirtualDevice>,
    on_fire: &dyn Fn(&Binding),
) {
    let EventSummary::Key(_, key_code, value) = event.destructure() else {
        forward(event, virtual_device);
        return;
    };
    let decision = match matcher.lock() {
        Ok(mut m) => m.observe(key_code.0, value),
        Err(_) => CaptureDecision::Forward,
    };
    match decision {
        CaptureDecision::Forward => forward(event, virtual_device),
        CaptureDecision::Fire(binding) => {
            log::info!(
                "evdev: hotkey fired {} -> {}::{}",
                binding.raw_key,
                binding.plugin_id,
                binding.action
            );
            on_fire(&binding);
        }
    }
}

fn forward(event: InputEvent, virtual_device: &Mutex<VirtualDevice>) {
    if let Ok(mut vd) = virtual_device.lock() {
        if let Err(error) = vd.emit(&[event]) {
            log::warn!("evdev: virtual emit failed: {error}");
        }
    }
}

fn open_keyboards() -> Result<Vec<(PathBuf, Device)>> {
    let mut keyboards = Vec::new();
    for (path, device) in evdev::enumerate() {
        if !is_keyboard(&device) {
            continue;
        }
        keyboards.push((path, device));
    }
    Ok(keyboards)
}

fn is_keyboard(device: &Device) -> bool {
    let Some(keys) = device.supported_keys() else {
        return false;
    };
    // Heuristic: a keyboard supports the basic alpha and ESC keys. Mice and
    // touchpads expose only BTN_* codes.
    keys.contains(KeyCode::KEY_ESC) && keys.contains(KeyCode::KEY_A)
}

fn matcher_keycodes_as_attribute_set(matcher: &BindingMatcher) -> AttributeSet<KeyCode> {
    let mut set = AttributeSet::<KeyCode>::new();
    // Always include common modifiers so the virtual device can re-emit them
    // even if the configured bindings don't reference every modifier.
    for code in [
        keycodes::KEY_LEFTSHIFT,
        keycodes::KEY_RIGHTSHIFT,
        keycodes::KEY_LEFTCTRL,
        keycodes::KEY_RIGHTCTRL,
        keycodes::KEY_LEFTALT,
        keycodes::KEY_RIGHTALT,
        keycodes::KEY_LEFTMETA,
        keycodes::KEY_RIGHTMETA,
    ] {
        set.insert(KeyCode(code));
    }
    for code in matcher.referenced_keycodes() {
        set.insert(KeyCode(code));
    }
    // The forwarded stream needs all the keys the user types, not just the
    // configured combos. Expose A-Z, 0-9, F1-F12, and the symbol keys so the
    // virtual device can re-emit them.
    for raw in PASSTHROUGH_KEYS {
        set.insert(KeyCode(*raw));
    }
    set
}

const PASSTHROUGH_KEYS: &[u16] = &[
    1, 14, 15, 28, 57, 99, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 119,
    // Letters a-z
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 43,
    44, 45, 46, 47, 48, 49, 50, 51, 52, 53, // Digits 1-9, 0
    2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, // F1-F12
    59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 87, 88,
];

fn build_virtual_device(keys: &AttributeSet<KeyCode>) -> Result<VirtualDevice> {
    let mut device = VirtualDevice::builder()
        .context("creating uinput builder (is /dev/uinput accessible?)")?
        .name("qol-tray-virtual-keyboard")
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
