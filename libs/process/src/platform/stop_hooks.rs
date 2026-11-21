use qol_sync::Mutex;

pub(crate) struct StopHooks {
    hooks: Mutex<Vec<fn()>>,
}

impl StopHooks {
    pub(crate) const fn new() -> Self {
        Self {
            hooks: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn register(&self, hook: fn()) {
        self.hooks.lock().push(hook);
    }

    pub(crate) fn run(&self) {
        let hooks = std::mem::take(&mut *self.hooks.lock());
        for hook in hooks {
            hook();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StopHooks;
    use std::sync::Mutex;

    static CALLS: Mutex<Vec<&str>> = Mutex::new(Vec::new());

    fn record(name: &'static str) {
        CALLS.lock().unwrap().push(name);
    }

    fn first() {
        record("first");
    }

    fn second() {
        record("second");
    }

    fn third() {
        record("third");
    }

    #[test]
    fn hooks_run_once_in_registration_order() {
        let hooks = StopHooks::new();
        hooks.register(second);
        hooks.register(first);
        hooks.register(third);
        hooks.register(first);

        hooks.run();
        hooks.run();

        assert_eq!(
            *CALLS.lock().unwrap(),
            ["second", "first", "third", "first"]
        );
    }
}
