mod platform;
mod policy;
#[cfg(test)]
pub(crate) mod tests;
mod trace;
mod watch;

pub(crate) use platform::Observation;
pub(crate) use policy::PolicyIntent;

pub(crate) fn observe() -> Observation {
    platform::observe()
}

pub(crate) fn policy_intent() -> PolicyIntent {
    policy::query()
}

pub fn spawn_watch() {
    watch::spawn();
}

pub fn stop_watch() {
    watch::stop_watch();
}
