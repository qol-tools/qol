use super::super::super::{Binding, CaptureEvent};
use super::super::super::{OnFire, RebuildBindings};
use super::matcher::BindingMatcher;
use anyhow::{Context, Result};
use crossbeam_channel::Receiver;
use evdev::{
    uinput::VirtualDevice, AttributeSet, AttributeSetRef, Device, EventSummary, InputEvent, KeyCode,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};

const VIRTUAL_KEYBOARD_NAME: &str = "qol-tray-virtual-keyboard";

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
    let keyboards = open_keyboards()?;
    if keyboards.is_empty() {
        anyhow::bail!("no keyboard input devices found under /dev/input");
    }
    let keyboard_count = keyboards.len();
    let mut grabbed_keyboards = Vec::new();
    for (path, mut device) in keyboards {
        if let Err(error) = device.grab() {
            log::warn!("evdev: failed to grab {}: {error}", path.display());
            continue;
        }
        log::info!("evdev: grabbed {}", path.display());
        grabbed_keyboards.push((path, device));
    }

    if grabbed_keyboards.is_empty() {
        anyhow::bail!(
            "evdev: found {keyboard_count} keyboard(s) but grabbed none (EVIOCGRAB denied; check input-group / udev permissions)"
        );
    }

    let key_caps = merged_key_capabilities(
        grabbed_keyboards
            .iter()
            .filter_map(|(_, device)| device.supported_keys()),
    );
    let virtual_device = Arc::new(Mutex::new(build_virtual_device(&key_caps)?));
    let on_fire: Arc<dyn Fn(&CaptureEvent) + Send + Sync> = Arc::from(on_fire);

    for (path, device) in grabbed_keyboards {
        let matcher = matcher.clone();
        let virtual_device = virtual_device.clone();
        let on_fire = on_fire.clone();

        std::thread::spawn(move || {
            run_reader(path, device, matcher, virtual_device, on_fire);
        });
    }

    spawn_reload_thread(matcher, reload_rx, rebuild, on_fire);

    Ok(())
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

fn run_reader(
    path: PathBuf,
    mut device: Device,
    matcher: Arc<Mutex<BindingMatcher>>,
    virtual_device: Arc<Mutex<VirtualDevice>>,
    on_fire: Arc<dyn Fn(&CaptureEvent) + Send + Sync>,
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
    on_fire: &dyn Fn(&CaptureEvent),
) {
    let EventSummary::Key(_, key_code, value) = event.destructure() else {
        forward(event, virtual_device);
        return;
    };
    let decision = match matcher.lock() {
        Ok(mut m) => m.observe(key_code.0, value),
        Err(_) => {
            forward(event, virtual_device);
            return;
        }
    };
    if decision.forward {
        forward(event, virtual_device);
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
}
