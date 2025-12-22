use std::sync::RwLock;

use super::DiscoveredPluginInfo;

#[derive(Debug, Clone, PartialEq)]
pub enum DiscoveryStatus {
    Idle,
    Discovering,
    Complete,
}

#[derive(Debug, Clone)]
pub struct DiscoveryState {
    pub status: DiscoveryStatus,
    pub plugins: Vec<DiscoveredPluginInfo>,
}

impl Default for DiscoveryState {
    fn default() -> Self {
        Self {
            status: DiscoveryStatus::Idle,
            plugins: vec![],
        }
    }
}

pub struct DaemonState {
    pub discovery: RwLock<DiscoveryState>,
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            discovery: RwLock::new(DiscoveryState::default()),
        }
    }
}

impl Default for DaemonState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn discovery_status_equality() {
        let cases = [
            (DiscoveryStatus::Idle, DiscoveryStatus::Idle, true),
            (DiscoveryStatus::Discovering, DiscoveryStatus::Discovering, true),
            (DiscoveryStatus::Complete, DiscoveryStatus::Complete, true),
            (DiscoveryStatus::Idle, DiscoveryStatus::Discovering, false),
            (DiscoveryStatus::Idle, DiscoveryStatus::Complete, false),
            (DiscoveryStatus::Discovering, DiscoveryStatus::Complete, false),
        ];

        for (a, b, expected) in cases {
            assert_eq!(a == b, expected, "{:?} == {:?} should be {}", a, b, expected);
        }
    }

    #[test]
    fn discovery_state_defaults() {
        let state = DiscoveryState::default();
        assert_eq!(state.status, DiscoveryStatus::Idle);
        assert!(state.plugins.is_empty());
    }

    #[test]
    fn daemon_state_initializes_correctly() {
        let state = DaemonState::new();
        let discovery = state.discovery.read().unwrap();
        assert_eq!(discovery.status, DiscoveryStatus::Idle);
        assert!(discovery.plugins.is_empty());
    }

    #[test]
    fn multiple_readers_can_access_simultaneously() {
        let state = Arc::new(DaemonState::new());
        let readers: Vec<_> = (0..10)
            .map(|_| {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    let _guard = state.discovery.read().unwrap();
                    thread::sleep(std::time::Duration::from_millis(10));
                })
            })
            .collect();

        for reader in readers {
            reader.join().unwrap();
        }
    }

    #[test]
    fn writer_blocks_readers_then_readers_resume() {
        let state = Arc::new(DaemonState::new());

        {
            let mut guard = state.discovery.write().unwrap();
            guard.status = DiscoveryStatus::Discovering;
        }

        let guard = state.discovery.read().unwrap();
        assert_eq!(guard.status, DiscoveryStatus::Discovering);
    }

    #[test]
    fn state_transitions_are_visible_to_readers() {
        let state = Arc::new(DaemonState::new());
        let state_clone = Arc::clone(&state);

        let writer = thread::spawn(move || {
            let mut guard = state_clone.discovery.write().unwrap();
            guard.status = DiscoveryStatus::Discovering;
            guard.plugins.push(DiscoveredPluginInfo {
                id: "test".into(),
                name: "Test".into(),
                path: "/test".into(),
            });
        });

        writer.join().unwrap();

        let guard = state.discovery.read().unwrap();
        assert_eq!(guard.status, DiscoveryStatus::Discovering);
        assert_eq!(guard.plugins.len(), 1);
    }
}
