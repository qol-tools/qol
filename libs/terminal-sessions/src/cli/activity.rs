use std::time::{Duration, SystemTime};

pub(super) const ACTIVITY_WINDOW: Duration = Duration::from_secs(120);

pub(super) fn recently_active(modified: Option<SystemTime>) -> Option<bool> {
    let modified = modified?;
    let age = SystemTime::now().duration_since(modified).ok();
    Some(age.is_none_or(|age| age <= ACTIVITY_WINDOW))
}

pub(super) fn quiet_secs(modified: Option<SystemTime>) -> Option<u64> {
    let age = SystemTime::now().duration_since(modified?);
    Some(age.map_or(0, |age| age.as_secs()))
}
