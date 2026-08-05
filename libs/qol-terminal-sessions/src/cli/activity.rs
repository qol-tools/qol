use std::time::{Duration, SystemTime};

pub(super) const ACTIVITY_WINDOW: Duration = Duration::from_secs(120);

pub(super) fn recently_active(modified: Option<SystemTime>) -> Option<bool> {
    let modified = modified?;
    let age = SystemTime::now().duration_since(modified).ok();
    Some(age.is_none_or(|age| age <= ACTIVITY_WINDOW))
}

pub(super) fn file_activity(modified: Option<SystemTime>, worked: bool) -> Option<bool> {
    recently_active(modified).map(|recent| recent && worked)
}
