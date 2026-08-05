use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static STATUS: Mutex<Published> = Mutex::new(Published {
    pin: None,
    expires_at_ms: 0,
});

struct Published {
    pin: Option<String>,
    expires_at_ms: u64,
}

pub struct Snapshot {
    pub pairing_open: bool,
    pub pin: Option<String>,
    pub seconds_remaining: u64,
}

pub fn open(pin: String, expires_at_ms: u64) {
    if let Ok(mut status) = STATUS.lock() {
        status.pin = Some(pin);
        status.expires_at_ms = expires_at_ms;
    }
}

pub fn close() {
    if let Ok(mut status) = STATUS.lock() {
        status.pin = None;
        status.expires_at_ms = 0;
    }
}

pub fn current() -> Snapshot {
    let Ok(status) = STATUS.lock() else {
        return Snapshot {
            pairing_open: false,
            pin: None,
            seconds_remaining: 0,
        };
    };
    let now = now_ms();
    if status.pin.is_none() || now >= status.expires_at_ms {
        return Snapshot {
            pairing_open: false,
            pin: None,
            seconds_remaining: 0,
        };
    }
    Snapshot {
        pairing_open: true,
        pin: status.pin.clone(),
        seconds_remaining: (status.expires_at_ms - now) / 1000,
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
