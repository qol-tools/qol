use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub(super) struct ProcessIdentity {
    pub pid: i32,
    pub start_time_us: u64,
}

static KNOWN_WINDOW_IDS_BY_IDENTITY: OnceLock<Mutex<HashMap<ProcessIdentity, HashSet<u32>>>> =
    OnceLock::new();

pub(super) fn known_window_ids_by_identity(
) -> &'static Mutex<HashMap<ProcessIdentity, HashSet<u32>>> {
    KNOWN_WINDOW_IDS_BY_IDENTITY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn process_identity(pid: i32) -> Option<ProcessIdentity> {
    let start_time_us = qol_app_icon::process_start_time_us(pid)?;
    Some(ProcessIdentity { pid, start_time_us })
}

pub(super) fn cached_process_identity(
    pid: i32,
    cache: &mut HashMap<i32, Option<ProcessIdentity>>,
) -> Option<ProcessIdentity> {
    if let Some(identity) = cache.get(&pid) {
        return *identity;
    }
    let identity = process_identity(pid);
    cache.insert(pid, identity);
    identity
}

pub(super) fn is_switchable_app(pid: i32) -> bool {
    use objc2_app_kit::NSRunningApplication;

    objc2::rc::autoreleasepool(|_pool| {
        match NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
            Some(app) => policy_is_switchable(app.activationPolicy()),
            None => false,
        }
    })
}

fn policy_is_switchable(policy: objc2_app_kit::NSApplicationActivationPolicy) -> bool {
    use objc2_app_kit::NSApplicationActivationPolicy as Policy;
    policy == Policy::Regular || policy == Policy::Accessory
}

#[cfg(debug_assertions)]
pub(super) fn app_policy_debug(pid: i32) -> String {
    use objc2_app_kit::NSRunningApplication;

    objc2::rc::autoreleasepool(|_pool| {
        match NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
            None => "no-app".to_string(),
            Some(app) => format!("{:?}", app.activationPolicy()),
        }
    })
}

pub(super) fn is_app_hidden(pid: i32) -> bool {
    use objc2_app_kit::NSRunningApplication;

    objc2::rc::autoreleasepool(|_pool| {
        let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
            return false;
        };
        app.isHidden()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_app_kit::NSApplicationActivationPolicy as Policy;

    #[test]
    fn switchable_policy_admits_regular_and_accessory_only() {
        let cases = [
            (Policy::Regular, true),
            (Policy::Accessory, true),
            (Policy::Prohibited, false),
        ];
        for (policy, expected) in cases {
            assert_eq!(policy_is_switchable(policy), expected, "policy: {policy:?}");
        }
    }
}
