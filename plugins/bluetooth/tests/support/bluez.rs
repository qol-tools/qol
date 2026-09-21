use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dbus::arg::{PropMap, Variant};
use dbus::blocking::SyncConnection;
use dbus::channel::{MatchingReceiver, Sender};
use dbus::message::MatchRule;
use dbus::{Message, Path};
use serde_json::{json, Value};

pub const ADDRESS: &str = "00:11:22:33:44:55";
const ADAPTER: &str = "/org/bluez/hci0";
const DEVICE: &str = "/org/bluez/hci0/dev_00_11_22_33_44_55";
const DEVICE_IFACE: &str = "org.bluez.Device1";
const ADAPTER_IFACE: &str = "org.bluez.Adapter1";
const OBJECTS_IFACE: &str = "org.freedesktop.DBus.ObjectManager";
type Objects = HashMap<Path<'static>, HashMap<String, PropMap>>;

#[derive(Default)]
pub struct State {
    pub searching: bool,
    pub present: bool,
    pub paired: bool,
    pub connected: bool,
    pub audio: bool,
    pub stalled: usize,
    pub starts: usize,
    pub stops: usize,
    pub pair_calls: usize,
    pub pairable: bool,
    pub paired_while_pairable: Option<bool>,
    remove: bool,
    quit: bool,
}

impl State {
    fn properties(&self, path: &str) -> PropMap {
        let mut values = PropMap::new();
        macro_rules! put {
            ($key:literal, $value:expr) => {
                values.insert($key.into(), Variant(Box::new($value)));
            };
        }
        if path == ADAPTER {
            put!("Address", "00:00:00:00:00:01".to_string());
            put!("Powered", true);
            put!("Discovering", self.searching);
            put!("Pairable", self.pairable);
        } else {
            put!("Address", ADDRESS.to_string());
            put!("Adapter", Path::from(ADAPTER));
            put!("Alias", "Test device".to_string());
            put!("Paired", self.paired);
            put!("Trusted", true);
            put!("Connected", self.connected);
            put!("ServicesResolved", self.connected);
            put!(
                "Icon",
                if self.audio {
                    "audio-headset"
                } else {
                    "input-gaming"
                }
                .to_string()
            );
            put!("Class", if self.audio { 2393092u32 } else { 1288u32 });
            put!(
                "UUIDs",
                vec![if self.audio {
                    "0000110b-0000-1000-8000-00805f9b34fb"
                } else {
                    "00001124-0000-1000-8000-00805f9b34fb"
                }
                .to_string()]
            );
            put!("RSSI", -40i16);
        }
        values
    }

    fn objects(&self) -> Objects {
        let mut objects = Objects::from([(
            Path::from(ADAPTER),
            HashMap::from([(ADAPTER_IFACE.into(), self.properties(ADAPTER))]),
        )]);
        if self.present {
            objects.insert(
                Path::from(DEVICE),
                HashMap::from([(DEVICE_IFACE.into(), self.properties(DEVICE))]),
            );
        }
        objects
    }
}

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub struct Fixture {
    plugin: Process,
    pub state: Arc<Mutex<State>>,
    thread: Option<std::thread::JoinHandle<()>>,
    _bus: Process,
    root: tempfile::TempDir,
}

impl Fixture {
    pub fn start(automatic: bool, audio: bool, connected: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut bus = Process(
            Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut address = String::new();
        BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim().to_string();
        let state = Arc::new(Mutex::new(State {
            present: true,
            paired: !audio,
            audio,
            connected,
            ..State::default()
        }));
        let connection = SyncConnection::new_address(&address).unwrap();
        connection
            .request_name("org.bluez", false, true, false)
            .unwrap();
        let shared = state.clone();
        connection.start_receive(
            MatchRule::new_method_call(),
            Box::new(move |message, connection| {
                handle(message, connection, &shared);
                true
            }),
        );
        let shared = state.clone();
        let thread = std::thread::spawn(move || loop {
            {
                let mut state = shared.lock().unwrap();
                if state.quit {
                    break;
                }
                if state.remove {
                    state.remove = false;
                    state.present = false;
                    connection
                        .send(
                            Message::new_signal("/", OBJECTS_IFACE, "InterfacesRemoved")
                                .unwrap()
                                .append2(Path::from(DEVICE), vec![DEVICE_IFACE.to_string()]),
                        )
                        .unwrap();
                }
            }
            connection.process(Duration::from_millis(10)).unwrap();
        });
        let config = root
            .path()
            .join("config/qol-tray/plugins/qol-bluetooth/config.json");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(config, json!({"managed_devices":if automatic {vec![ADDRESS]} else {vec![]},"auto_reconnect":automatic,"auto_reclaim_on_play":false,"set_default_output":false}).to_string()).unwrap();
        let binary = std::env::var_os("QOL_BLUETOOTH_TEST_BINARY")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_qol-bluetooth").into());
        let plugin = Process(
            Command::new(binary)
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", root.path())
                .env("XDG_CONFIG_HOME", root.path().join("config"))
                .env("XDG_DATA_HOME", root.path().join("data"))
                .env("XDG_CACHE_HOME", root.path().join("cache"))
                .env("DBUS_SYSTEM_BUS_ADDRESS", &address)
                .env("DBUS_SESSION_BUS_ADDRESS", &address)
                .env(
                    qol_conventions::ENV_DAEMON_SOCKET,
                    root.path().join("daemon.sock"),
                )
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let fixture = Self {
            plugin,
            state,
            thread: Some(thread),
            _bus: bus,
            root,
        };
        assert!(
            wait(
                || fixture.request("ping", Value::Null).is_some(),
                Duration::from_secs(5)
            ),
            "daemon did not start"
        );
        fixture
    }

    pub fn action(&self, name: &str) -> Value {
        self.request(name, json!({"address": ADDRESS}))
            .expect("daemon must answer")
    }

    pub fn remove(&self) {
        self.state.lock().unwrap().remove = true;
        assert!(wait(
            || !self.state.lock().unwrap().present,
            Duration::from_secs(1)
        ));
    }

    fn request(&self, name: &str, input: Value) -> Option<Value> {
        let mut socket = UnixStream::connect(self.root.path().join("daemon.sock")).ok()?;
        socket
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        writeln!(socket, "{}", json!({"action":name,"input":input})).ok()?;
        socket.shutdown(std::net::Shutdown::Write).ok()?;
        let mut response = String::new();
        socket.read_to_string(&mut response).ok()?;
        let response: Value = serde_json::from_str(&response).ok()?;
        assert_eq!(response["status"], "handled", "{response}");
        Some(response["data"].clone())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.plugin.0.kill();
        let _ = self.plugin.0.wait();
        self.state.lock().unwrap().quit = true;
        self.thread.take().unwrap().join().unwrap();
    }
}

pub fn wait(mut check: impl FnMut() -> bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn handle(message: Message, connection: &SyncConnection, shared: &Mutex<State>) {
    let mut state = shared.lock().unwrap();
    let member = message.member().unwrap().to_string();
    let path = message.path().unwrap().to_string();
    if path == DEVICE && !state.present {
        let name = "org.freedesktop.DBus.Error.UnknownObject".into();
        connection
            .send(message.error(&name, c"Device is absent"))
            .unwrap();
        return;
    }
    let reply = match member.as_str() {
        "GetManagedObjects" => message.method_return().append1(state.objects()),
        "GetAll" => message.method_return().append1(state.properties(&path)),
        "Get" => {
            let (_, key): (String, String) = message.read2().unwrap();
            match state.properties(&path).remove(&key) {
                Some(value) => message.method_return().append1(value),
                None => message.error(
                    &"org.freedesktop.DBus.Error.InvalidArgs".into(),
                    c"No property",
                ),
            }
        }
        "Connect" | "Disconnect" | "RemoveDevice" => {
            state.stalled += 1;
            return;
        }
        "StartDiscovery" => {
            state.searching = true;
            state.starts += 1;
            if !state.present {
                state.present = true;
                connection
                    .send(
                        Message::new_signal("/", OBJECTS_IFACE, "InterfacesAdded")
                            .unwrap()
                            .append2(
                                Path::from(DEVICE),
                                HashMap::from([(
                                    DEVICE_IFACE.to_string(),
                                    state.properties(DEVICE),
                                )]),
                            ),
                    )
                    .unwrap();
            }
            message.method_return()
        }
        "StopDiscovery" => {
            state.searching = false;
            state.stops += 1;
            message.method_return()
        }
        "Pair" => {
            state.paired = true;
            state.pair_calls += 1;
            state.paired_while_pairable = Some(state.pairable);
            message.method_return()
        }
        "Set" if path == DEVICE => {
            state.stalled += 1;
            return;
        }
        "Set" if path == ADAPTER => {
            let (_, key, value): (String, String, Variant<bool>) = message.read3().unwrap();
            if key == "Pairable" {
                state.pairable = value.0;
            }
            message.method_return()
        }
        "SetDiscoveryFilter" => message.method_return(),
        _ => message.error(
            &"org.freedesktop.DBus.Error.UnknownMethod".into(),
            c"Unsupported fixture method",
        ),
    };
    connection.send(reply).unwrap();
}
