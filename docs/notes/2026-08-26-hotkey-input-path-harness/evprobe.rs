use evdev::{uinput::VirtualDeviceBuilder, AttributeSet, Device, EventType, InputEvent, KeyCode};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PROBE_NAME: &str = "qol-evdev-stall-probe";
const TRAY_VK: &str = "qol-tray-virtual-keyboard";

type Row = (&'static str, u16, i32, u128, u128); // (channel, code, value, kernel_ms, read_ms)

fn find_device(name: &str) -> Option<(PathBuf, Device)> {
    for entry in std::fs::read_dir("/dev/input").ok()? {
        let path = entry.ok()?.path();
        if !path.file_name()?.to_str()?.starts_with("event") { continue; }
        let Ok(device) = Device::open(&path) else { continue };
        if device.name().unwrap_or("") == name { return Some((path, device)); }
    }
    None
}

fn ms(t: SystemTime) -> u128 { t.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() }
fn env_u64(key: &str, default: u64) -> u64 { std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default) }

/// Re-acquiring reader: a node that vanishes (tray restart) is re-opened by name.
fn spawn_reader(name: &'static str, channel: &'static str, tx: mpsc::Sender<Row>) {
    std::thread::spawn(move || loop {
        let Some((_, mut device)) = find_device(name) else {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        };
        loop {
            match device.fetch_events() {
                Ok(events) => for event in events {
                    if event.event_type() == EventType::KEY {
                        let _ = tx.send((channel, event.code(), event.value(), ms(event.timestamp()), ms(SystemTime::now())));
                    }
                },
                Err(_) => break,
            }
        }
    });
}

fn main() {
    let gap = env_u64("PROBE_GAP_MS", 150);
    let count = env_u64("PROBE_COUNT", 6) as usize;
    let tail_s = env_u64("PROBE_TAIL_S", 4);
    let wait_s = env_u64("PROBE_WAIT_S", 8);
    let probe_keys: Vec<KeyCode> = vec![
        KeyCode::KEY_F13, KeyCode::KEY_F14, KeyCode::KEY_F15,
        KeyCode::KEY_F16, KeyCode::KEY_F17, KeyCode::KEY_F18,
    ];
    let mut caps = AttributeSet::<KeyCode>::new();
    for key in &probe_keys { caps.insert(*key); }
    caps.insert(KeyCode::KEY_F12);
    caps.insert(KeyCode::KEY_ESC);
    caps.insert(KeyCode::KEY_A);
    let mut probe = VirtualDeviceBuilder::new().expect("uinput")
        .name(PROBE_NAME).with_keys(&caps).expect("keys").build().expect("build");
    for path in probe.enumerate_dev_nodes_blocking().expect("nodes") {
        println!("probe node: {:?}", path.expect("node"));
    }
    println!("waiting {wait_s}s for the tray rescan to grab the probe");
    std::thread::sleep(Duration::from_secs(wait_s));
    println!("tray virtual keyboard: {:?}", find_device(TRAY_VK).map(|(p, _)| p));

    let (tx, rx) = mpsc::channel::<Row>();
    spawn_reader(TRAY_VK, "tray", tx.clone());
    spawn_reader(PROBE_NAME, "direct", tx);
    std::thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}

    let wall0 = ms(SystemTime::now());
    println!("wall0={wall0}");
    if std::env::var("PROBE_TRIGGER").is_ok() {
        let n = env_u64("PROBE_TRIGGER_COUNT", 1);
        let g = env_u64("PROBE_TRIGGER_GAP_MS", 10);
        println!("firing F12 x{n} from t=0 ({g}ms apart)");
        for _ in 0..n {
            probe.emit(&[
                InputEvent::new(EventType::KEY.0, KeyCode::KEY_F12.0, 1),
                InputEvent::new(EventType::KEY.0, KeyCode::KEY_F12.0, 0),
            ]).expect("trigger emit");
            std::thread::sleep(Duration::from_millis(g));
        }
        println!("trigger burst done at t={}ms", ms(SystemTime::now()) as i128 - wall0 as i128);
    }
    let newcap_at = env_u64("PROBE_NEWCAP_AT_MS", 0);
    let mut newcap_device = None;
    let mut injected: Vec<(u16, i128)> = Vec::new();
    for index in 0..count {
        if newcap_at > 0 && newcap_device.is_none() && (index as u64) * gap >= newcap_at {
            let mut caps2 = AttributeSet::<KeyCode>::new();
            let newcap_code = env_u64("PROBE_NEWCAP_CODE", 189) as u16; caps2.insert(KeyCode(newcap_code));
            caps2.insert(KeyCode::KEY_ESC); caps2.insert(KeyCode::KEY_A);
            let dev = VirtualDeviceBuilder::new().expect("uinput").name("qol-evdev-newcap-probe")
                .with_keys(&caps2).expect("keys").build().expect("build");
            println!("newcap keyboard created at t={}ms code={}", ms(SystemTime::now()) as i128 - wall0 as i128, newcap_code);
            newcap_device = Some(dev);
        }
        let key = &probe_keys[index % probe_keys.len()];
        probe.emit(&[
            InputEvent::new(EventType::KEY.0, key.0, 1),
            InputEvent::new(EventType::KEY.0, key.0, 0),
        ]).expect("emit");
        injected.push((key.0, ms(SystemTime::now()) as i128 - wall0 as i128));
        std::thread::sleep(Duration::from_millis(gap));
    }
    println!("injected {} keys, gap={}ms, last_at={}ms", injected.len(), gap, injected.last().map(|k| k.1).unwrap_or(0));
    std::thread::sleep(Duration::from_secs(tail_s));
    drop(newcap_device);

    let mut seen: Vec<Row> = Vec::new();
    while let Ok(row) = rx.try_recv() { seen.push(row); }
    seen.sort_by_key(|r| r.3);
    let rel = |t: u128| t as i128 - wall0 as i128;

    // Per-injection fate: per code, the k-th injection matches the k-th observation in the
    // MERGED (tray + direct) stream sorted by time. A key that reached neither channel
    // shifts every later match, so true losses surface as LOST at the end; the latency
    // column shows where a shift began (it stays high instead of decaying like a burst).
    let mut merged: BTreeMap<u16, Vec<(i128, &'static str)>> = BTreeMap::new();
    for (ch, code, value, kern, _) in &seen {
        if *value == 1 { merged.entry(*code).or_default().push((rel(*kern), ch)); }
    }
    for list in merged.values_mut() { list.sort(); }
    let mut cursor: BTreeMap<u16, usize> = BTreeMap::new();
    let (mut via_tray, mut direct, mut lost) = (0, 0, 0);
    let mut worst_latency = 0i128;
    println!("--- per-key fate (t in ms since wall0) ---");
    for (code, inject_t) in &injected {
        let i = cursor.entry(*code).or_insert(0);
        let hit = merged.get(code).and_then(|q| q.get(*i).copied());
        let fate = match hit {
            Some((_, "tray")) => { via_tray += 1; "tray" }
            Some((_, _)) => { direct += 1; "direct(ungrabbed)" }
            None => { lost += 1; "LOST" }
        };
        if hit.is_some() { *i += 1; }
        let latency = hit.map(|(t, _)| t - inject_t);
        if let Some(l) = latency { worst_latency = worst_latency.max(l); }
        let show = |t: Option<i128>| t.map(|t| t.to_string()).unwrap_or_else(|| "-".into());
        println!("  code={code} inject_t={inject_t} seen_t={} latency={} {fate}", show(hit.map(|h| h.0)), show(latency));
    }
    println!("RESULT injected={} via_tray={via_tray} direct={direct} lost={lost}", injected.len());
    println!("RESULT worst_latency_ms={worst_latency}");

    // Tray stream shape: gaps and bursts.
    let tray_downs: Vec<i128> = seen.iter().filter(|r| r.0 == "tray" && r.2 == 1).map(|r| rel(r.3)).collect();
    let mut worst = 0i128; let mut worst_at = 0i128;
    for pair in tray_downs.windows(2) { if pair[1] - pair[0] > worst { worst = pair[1] - pair[0]; worst_at = pair[0]; } }
    let mut per_ms = BTreeMap::new();
    for t in &tray_downs { *per_ms.entry(*t).or_insert(0) += 1; }
    println!("RESULT tray_worst_gap_ms={worst} at_t={worst_at} nominal_gap_ms={gap}");
    println!("RESULT tray_max_downs_in_one_ms={} (burst if >1)", per_ms.values().copied().max().unwrap_or(0));

    // Stuck keys per channel: a down never followed by its up.
    for ch in ["tray", "direct"] {
        let mut open: BTreeMap<u16, i128> = BTreeMap::new();
        let mut orphan_ups = 0;
        for (c, code, value, kern, _) in &seen {
            if *c != ch { continue; }
            if *value == 1 { open.insert(*code, rel(*kern)); }
            else if *value == 0 && open.remove(code).is_none() { orphan_ups += 1; }
        }
        for (code, t) in &open { println!("RESULT STUCK_KEY channel={ch} code={code} down_at={t}ms never_released"); }
        println!("RESULT channel={ch} left_held={} orphan_ups={orphan_ups}", open.len());
    }
}
