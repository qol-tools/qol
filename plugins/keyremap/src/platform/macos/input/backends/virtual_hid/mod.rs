pub(crate) mod fn_keys;
pub(crate) mod keyboard;

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};

use crate::platform::macos::app::remap::ResolvedConfig;
use crate::platform::macos::hid_helper::protocol::{
    self, Role, ToDaemon, ToHelper, PROTOCOL_VERSION, SOCKET_PATH,
};
use crate::platform::macos::input::{InputState, Strategy};
use crate::platform::macos::layout::LayoutStore;
use keyboard::{KeyContext, KeyboardState, Output};

const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(50);
const RETRY_INTERVAL: Duration = Duration::from_secs(2);
const CAPS_LOCK_SETTLE: Duration = Duration::from_millis(100);
const WRITE_TIMEOUT: Duration = Duration::from_millis(200);
const HID_SYSTEM_STATE: i32 = 1;
const ALPHA_SHIFT: u64 = 0x0001_0000;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFPreferencesAnyApplication: CFStringRef;
    fn CFPreferencesCopyAppValue(key: CFStringRef, application: CFStringRef) -> CFTypeRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceFlagsState(state: i32) -> u64;
}

pub(crate) trait KeyEnvironment: Send + Sync {
    fn config(&self) -> Arc<ResolvedConfig>;
    fn key_target_bundle_id(&self) -> String;
    fn input(&self) -> &Arc<InputState>;
}

pub(crate) fn start(environment: Arc<impl KeyEnvironment + 'static>, layouts: Arc<LayoutStore>) {
    std::thread::Builder::new()
        .name("keyremap-vhid-session".into())
        .spawn(move || {
            let mut last_error = String::new();
            loop {
                let outcome = UnixStream::connect(SOCKET_PATH)
                    .context("the keyboard helper is not running")
                    .and_then(|stream| run_session(stream, environment.as_ref(), &layouts));
                match outcome {
                    Ok(()) => log::info!("the keyboard helper session ended"),
                    Err(error) => {
                        let message = format!("{error:#}");
                        if message != last_error {
                            log::info!("using the event tap: {message}");
                            last_error = message;
                        }
                    }
                }
                std::thread::sleep(RETRY_INTERVAL);
            }
        })
        .expect("failed to spawn the virtual HID session thread");
}

pub(crate) fn run_session(
    stream: UnixStream,
    environment: &impl KeyEnvironment,
    layouts: &LayoutStore,
) -> Result<()> {
    let result = session(stream, environment, layouts);
    environment.input().markers.clear();
    if environment.input().strategy.set(Strategy::EventTap) == Strategy::VirtualHid {
        log::info!("key input is back on the event tap");
    }
    result
}

fn session(
    stream: UnixStream,
    environment: &impl KeyEnvironment,
    layouts: &LayoutStore,
) -> Result<()> {
    let mut writer = stream.try_clone()?;
    writer.set_write_timeout(Some(WRITE_TIMEOUT))?;
    protocol::write_message(
        &mut writer,
        &ToHelper::Hello {
            protocol: PROTOCOL_VERSION,
            role: Role::Session,
        },
    )?;
    let mut lines = BufReader::new(stream).lines();
    let greeting = lines
        .next()
        .context("the keyboard helper closed the connection")??;
    match protocol::parse_message::<ToDaemon>(&greeting)? {
        ToDaemon::Welcome { .. } => log::info!("connected to the keyboard helper"),
        ToDaemon::Refused { protocol, reason } => bail!(
            "the keyboard helper speaks protocol {protocol} and keyremap speaks {PROTOCOL_VERSION} ({reason}); run sudo qol-keyremap install-hid-helper"
        ),
        other => bail!("unexpected greeting from the keyboard helper: {other:?}"),
    }

    let writer = Arc::new(Mutex::new(writer));
    let caps_due = Arc::new(Mutex::new(None::<Instant>));
    let stop = Arc::new(AtomicBool::new(false));
    std::thread::scope(|scope| {
        scope.spawn(|| heartbeat_loop(environment, &writer, &caps_due, &stop));
        let result = read_keys(lines, environment, layouts, &writer, &caps_due);
        stop.store(true, Ordering::SeqCst);
        result
    })
}

fn heartbeat_loop(
    environment: &impl KeyEnvironment,
    writer: &Mutex<UnixStream>,
    caps_due: &Mutex<Option<Instant>>,
    stop: &AtomicBool,
) {
    while !stop.load(Ordering::SeqCst) {
        if environment.config().enabled && send(writer, &ToHelper::Heartbeat).is_err() {
            return;
        }
        let due = caps_due
            .lock()
            .map(|mut due| due.take_if(|at| Instant::now() >= *at))
            .unwrap_or_default();
        if due.is_some() {
            let _ = send(writer, &ToHelper::CapsLockLight { on: caps_lock_on() });
        }
        std::thread::sleep(HEARTBEAT_INTERVAL);
    }
}

fn read_keys(
    lines: std::io::Lines<BufReader<UnixStream>>,
    environment: &impl KeyEnvironment,
    layouts: &LayoutStore,
    writer: &Mutex<UnixStream>,
    caps_due: &Mutex<Option<Instant>>,
) -> Result<()> {
    let mut keyboard = KeyboardState::default();
    let mut warned = HashSet::new();
    for line in lines {
        match protocol::parse_message::<ToDaemon>(&line?)? {
            ToDaemon::Key {
                usage_page,
                usage,
                pressed,
                apple,
            } => {
                let config = environment.config();
                let layout = layouts.get();
                let bundle_id = environment.key_target_bundle_id();
                let context = KeyContext {
                    config: &config,
                    table: &layout.table,
                    physical: layout.physical,
                    bundle_id: &bundle_id,
                    fn_state: apple && fn_keys_are_standard(),
                };
                for output in keyboard.handle(usage_page, usage, pressed, apple, &context) {
                    apply(output, environment.input(), writer, caps_due, &mut warned)?;
                }
            }
            ToDaemon::Seized { active } => {
                if !active {
                    keyboard.reset();
                    environment.input().markers.clear();
                }
                let strategy = if active {
                    Strategy::VirtualHid
                } else {
                    Strategy::EventTap
                };
                if environment.input().strategy.set(strategy) != strategy {
                    log::info!("key input strategy: {strategy:?}");
                }
            }
            ToDaemon::Welcome { .. } | ToDaemon::Refused { .. } | ToDaemon::Status(_) => {}
        }
    }
    Ok(())
}

fn apply(
    output: Output,
    input: &InputState,
    writer: &Mutex<UnixStream>,
    caps_due: &Mutex<Option<Instant>>,
    warned: &mut HashSet<String>,
) -> Result<()> {
    match output {
        Output::Emit(emit) => send(
            writer,
            &ToHelper::Emit {
                usage_page: emit.page,
                usage: emit.usage,
                pressed: emit.pressed,
            },
        )?,
        Output::Mark { keycode, marker } => input.markers.insert(keycode, marker),
        Output::Tap { keycode, marker } => input.markers.tap(keycode, marker, Instant::now()),
        Output::Unmark { keycode } => input.markers.release(keycode, Instant::now()),
        Output::MissingChar(text) => {
            if warned.insert(text.clone()) {
                log::warn!(
                    "no key sequence types {text:?} in the current keyboard layout; skipping it"
                );
            }
        }
        Output::CapsLock => {
            if let Ok(mut due) = caps_due.lock() {
                *due = Some(Instant::now() + CAPS_LOCK_SETTLE);
            }
        }
    }
    Ok(())
}

fn send(writer: &Mutex<UnixStream>, message: &ToHelper) -> std::io::Result<()> {
    let mut stream = writer
        .lock()
        .map_err(|_| std::io::Error::other("helper writer lock poisoned"))?;
    protocol::write_message(&mut *stream, message)
}

fn fn_keys_are_standard() -> bool {
    let key = CFString::from_static_string("com.apple.keyboard.fnState");
    let value = unsafe {
        CFPreferencesCopyAppValue(key.as_concrete_TypeRef(), kCFPreferencesAnyApplication)
    };
    if value.is_null() {
        return false;
    }
    let value = unsafe { CFType::wrap_under_create_rule(value) };
    if let Some(flag) = value.downcast::<CFBoolean>() {
        return bool::from(flag);
    }
    value
        .downcast::<CFNumber>()
        .and_then(|number| number.to_i64())
        .is_some_and(|number| number != 0)
}

fn caps_lock_on() -> bool {
    let flags = unsafe { CGEventSourceFlagsState(HID_SYSTEM_STATE) };
    flags & ALPHA_SHIFT != 0
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;
    use std::time::Duration;

    use qol_hotkeys::macos_keycode::PhysicalLayout;
    use serde_json::json;

    use super::*;
    use crate::platform::macos::app::config::RemapConfig;
    use crate::platform::macos::app::remap;
    use crate::platform::macos::hid_helper::protocol::PAGE_KEYBOARD;
    use crate::platform::macos::layout::{CharTable, LayoutSnapshot};

    struct FakeEnvironment {
        config: Arc<ResolvedConfig>,
        input: Arc<InputState>,
    }

    impl KeyEnvironment for FakeEnvironment {
        fn config(&self) -> Arc<ResolvedConfig> {
            Arc::clone(&self.config)
        }

        fn key_target_bundle_id(&self) -> String {
            "com.apple.TextEdit".to_string()
        }

        fn input(&self) -> &Arc<InputState> {
            &self.input
        }
    }

    fn environment(enabled: bool) -> FakeEnvironment {
        let raw: RemapConfig = serde_json::from_value(json!({
            "enabled": enabled,
            "key_rules": [{ "from_mods": ["ctrl"], "from_key": "c", "to_mods": ["cmd"], "to_key": "c" }]
        }))
        .unwrap();
        FakeEnvironment {
            config: Arc::new(remap::resolve(&raw)),
            input: Arc::new(InputState::default()),
        }
    }

    fn layouts() -> LayoutStore {
        LayoutStore::new(LayoutSnapshot {
            id: "test".to_string(),
            table: CharTable::from_translation(|_, _, _| None),
            physical: PhysicalLayout::Ansi,
        })
    }

    struct FakeHelper {
        stream: UnixStream,
        lines: std::io::Lines<BufReader<UnixStream>>,
    }

    impl FakeHelper {
        fn say(&mut self, message: &ToDaemon) {
            protocol::write_message(&mut self.stream, message).unwrap();
        }

        fn next_non_heartbeat(&mut self) -> ToHelper {
            loop {
                let line = self.lines.next().expect("session closed").unwrap();
                let message: ToHelper = protocol::parse_message(&line).unwrap();
                if message != ToHelper::Heartbeat {
                    return message;
                }
            }
        }
    }

    fn connect(
        environment: Arc<FakeEnvironment>,
    ) -> (FakeHelper, std::thread::JoinHandle<anyhow::Result<()>>) {
        let (daemon_side, helper_side) = UnixStream::pair().unwrap();
        helper_side
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let session =
            std::thread::spawn(move || run_session(daemon_side, environment.as_ref(), &layouts()));
        let mut lines = BufReader::new(helper_side.try_clone().unwrap()).lines();
        let hello: ToHelper = protocol::parse_message(&lines.next().unwrap().unwrap()).unwrap();
        assert_eq!(
            hello,
            ToHelper::Hello {
                protocol: PROTOCOL_VERSION,
                role: Role::Session
            }
        );
        (
            FakeHelper {
                stream: helper_side,
                lines,
            },
            session,
        )
    }

    fn emit(usage: u16, pressed: bool) -> ToHelper {
        ToHelper::Emit {
            usage_page: PAGE_KEYBOARD,
            usage,
            pressed,
        }
    }

    #[test]
    fn keys_come_back_remapped_and_seized_flips_the_strategy() {
        let environment = Arc::new(environment(true));
        let (mut helper, session) = connect(Arc::clone(&environment));
        helper.say(&ToDaemon::Welcome {
            protocol: PROTOCOL_VERSION,
        });
        helper.say(&ToDaemon::Seized { active: true });
        helper.say(&ToDaemon::Key {
            usage_page: PAGE_KEYBOARD,
            usage: 0xE0,
            pressed: true,
            apple: false,
        });
        helper.say(&ToDaemon::Key {
            usage_page: PAGE_KEYBOARD,
            usage: 0x06,
            pressed: true,
            apple: false,
        });

        assert_eq!(helper.next_non_heartbeat(), emit(0xE0, true));
        assert_eq!(helper.next_non_heartbeat(), emit(0xE0, false));
        assert_eq!(helper.next_non_heartbeat(), emit(0xE3, true));
        assert_eq!(helper.next_non_heartbeat(), emit(0x06, true));
        assert_eq!(environment.input.strategy.get(), Strategy::VirtualHid);
        assert!(environment
            .input
            .markers
            .lookup(0x08, true, std::time::Instant::now())
            .is_some());

        helper.say(&ToDaemon::Seized { active: false });
        drop(helper);
        assert!(session.join().unwrap().is_ok());
        assert_eq!(environment.input.strategy.get(), Strategy::EventTap);
        assert!(environment
            .input
            .markers
            .lookup(0x08, true, std::time::Instant::now())
            .is_none());
    }

    #[test]
    fn a_refused_greeting_names_both_versions() {
        let (mut helper, session) = connect(Arc::new(environment(true)));
        helper.say(&ToDaemon::Refused {
            protocol: 9,
            reason: "old".to_string(),
        });
        let error = session.join().unwrap().unwrap_err().to_string();
        assert!(
            error.contains('9') && error.contains(&PROTOCOL_VERSION.to_string()),
            "{error}"
        );
    }

    #[test]
    fn a_disabled_config_sends_no_heartbeats() {
        let (mut helper, _session) = connect(Arc::new(environment(false)));
        helper.say(&ToDaemon::Welcome {
            protocol: PROTOCOL_VERSION,
        });
        helper
            .stream
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        assert!(helper.lines.next().is_none_or(|line| line.is_err()));
    }
}
