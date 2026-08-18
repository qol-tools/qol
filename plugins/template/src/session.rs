use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationLifetime {
    PortableSession,
    ResidentPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreMode {
    CleanExit,
    Recovery,
}

pub struct HostSession {
    prior: BTreeMap<String, Option<String>>,
    changed: BTreeMap<String, String>,
}

impl HostSession {
    pub fn new() -> Self {
        Self {
            prior: BTreeMap::new(),
            changed: BTreeMap::new(),
        }
    }

    pub fn mutate(&mut self, key: &str, value: &str, _lifetime: MutationLifetime) {
        self.prior
            .entry(key.to_string())
            .or_insert_with(|| read_host(key));
        write_host(key, value);
        self.changed.insert(key.to_string(), value.to_string());
    }

    pub fn restore(&mut self, _mode: RestoreMode) -> usize {
        let keys: Vec<String> = self.changed.keys().cloned().collect();
        for key in &keys {
            match self.prior.get(key) {
                Some(Some(prior)) => write_host(key, prior),
                _ => remove_host(key),
            }
        }
        let restored = keys.len();
        self.changed.clear();
        restored
    }
}

impl Default for HostSession {
    fn default() -> Self {
        Self::new()
    }
}

fn read_host(key: &str) -> Option<String> {
    std::env::var(format!("QOL_HOST_MUTATION_{}", key.to_uppercase())).ok()
}

fn write_host(key: &str, value: &str) {
    std::env::set_var(format!("QOL_HOST_MUTATION_{}", key.to_uppercase()), value);
}

fn remove_host(key: &str) {
    std::env::remove_var(format!("QOL_HOST_MUTATION_{}", key.to_uppercase()));
}

#[cfg(test)]
mod tests {
    use super::{HostSession, MutationLifetime, RestoreMode};

    #[test]
    fn snapshot_before_mutate_restores_the_prior_host_value() {
        std::env::remove_var("QOL_HOST_MUTATION_FOO");
        let mut session = HostSession::new();
        session.mutate("foo", "first", MutationLifetime::PortableSession);
        assert_eq!(
            std::env::var("QOL_HOST_MUTATION_FOO").as_deref(),
            Ok("first")
        );
        session.mutate("foo", "second", MutationLifetime::PortableSession);
        assert_eq!(
            std::env::var("QOL_HOST_MUTATION_FOO").as_deref(),
            Ok("second")
        );
        assert_eq!(session.restore(RestoreMode::CleanExit), 1);
        assert_eq!(
            std::env::var("QOL_HOST_MUTATION_FOO"),
            Err(std::env::VarError::NotPresent)
        );
    }
}
