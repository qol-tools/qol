use std::fmt::{Display, Formatter};
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

use crate::cli::{CliLaunchProgram, CliToolId};
use crate::{IdentityError, SessionId, TerminalError};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SpawnKey(String);

impl SpawnKey {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if crate::model::valid_component(&value) {
            return Ok(Self(value));
        }
        Err(IdentityError::component("spawn key", value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for SpawnKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        SpawnKey::new(value).map_err(serde::de::Error::custom)
    }
}

impl Display for SpawnKey {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnSurface {
    Tab,
    OsWindow,
}

impl Display for SpawnSurface {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tab => formatter.write_str("tab"),
            Self::OsWindow => formatter.write_str("os window"),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SpawnIdentity {
    pub key: SpawnKey,
    pub tool: CliToolId,
    pub surface: SpawnSurface,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SpawnRequest {
    pub identity: SpawnIdentity,
    pub launch: CliLaunchProgram,
    pub cwd: PathBuf,
    pub title: Option<String>,
}

pub trait SessionSpawner: Send + Sync {
    fn supports(&self, surface: SpawnSurface) -> bool;

    fn spawn(&self, request: &SpawnRequest) -> Result<SessionId, TerminalError>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::cli::{CliLaunchProgram, CliToolId};
    use crate::{
        BackendId, DeliveryMode, ScreenReader, SessionBinding, SessionFocus, SessionId,
        SessionInventory, SessionSpawner, SpawnIdentity, SpawnKey, SpawnRequest, SpawnSurface,
        TerminalBackend, TerminalError, TerminalSessionService, TerminalSnapshot, TextInput,
    };

    fn request(key: &str, surface: SpawnSurface) -> SpawnRequest {
        SpawnRequest {
            identity: SpawnIdentity {
                key: SpawnKey::new(key).unwrap(),
                tool: CliToolId::new("codex").unwrap(),
                surface,
            },
            launch: CliLaunchProgram::new("codex"),
            cwd: std::path::PathBuf::from("/work/project"),
            title: None,
        }
    }

    fn identity(key: &str) -> SpawnIdentity {
        SpawnIdentity {
            key: SpawnKey::new(key).unwrap(),
            tool: CliToolId::new("codex").unwrap(),
            surface: SpawnSurface::Tab,
        }
    }

    #[test]
    fn spawn_key_accepts_safe_components_and_rejects_everything_else() {
        let accepted = ["voice-42", "qol.voice", "a_b", "0"];
        for value in accepted {
            assert!(SpawnKey::new(value).is_ok(), "value: {value}");
        }
        let rejected = [
            "",
            "has space",
            "colon:separated",
            "slash/separated",
            "semi;separated",
            "at@sign",
            "braces{}",
            "ümlaut",
        ];
        for value in rejected {
            assert!(SpawnKey::new(value).is_err(), "value: {value}");
        }
        let key = SpawnKey::new("voice-42").unwrap();
        assert_eq!(key.as_str(), "voice-42");
        assert_eq!(key.to_string(), "voice-42");
    }

    #[test]
    fn spawn_key_serde_round_trips() {
        let key = SpawnKey::new("voice-42").unwrap();
        let encoded = serde_json::to_string(&key).unwrap();
        assert_eq!(serde_json::from_str::<SpawnKey>(&encoded).unwrap(), key);
    }

    #[test]
    fn spawn_key_deserialization_keeps_validation() {
        let invalid = serde_json::from_str::<SpawnKey>("\"has space\"")
            .expect_err("invalid keys must be rejected on deserialization");
        assert!(invalid.to_string().contains("unsupported characters"));
        assert!(serde_json::from_str::<SpawnKey>("\"voice-42\"").is_ok());
    }

    #[test]
    fn spawn_surface_serde_round_trips_every_variant() {
        let cases = [SpawnSurface::Tab, SpawnSurface::OsWindow];
        for surface in cases {
            let encoded = serde_json::to_string(&surface).unwrap();
            assert_eq!(
                serde_json::from_str::<SpawnSurface>(&encoded).unwrap(),
                surface
            );
        }
        assert_eq!(
            serde_json::to_string(&SpawnSurface::OsWindow).unwrap(),
            "\"os_window\""
        );
        assert_eq!(SpawnSurface::Tab.to_string(), "tab");
        assert_eq!(SpawnSurface::OsWindow.to_string(), "os window");
    }

    #[test]
    fn spawn_request_round_trips_with_structured_identity_and_launch_program() {
        let request = SpawnRequest {
            identity: identity("voice-42"),
            launch: CliLaunchProgram {
                program: "codex".to_owned(),
                args: vec!["--dangerously-skip-permissions".to_owned()],
                env: Vec::new(),
            },
            cwd: std::path::PathBuf::from("/work/project"),
            title: Some("Codex".to_owned()),
        };
        let encoded = serde_json::to_string(&request).unwrap();
        assert_eq!(
            serde_json::from_str::<SpawnRequest>(&encoded).unwrap(),
            request
        );
    }

    #[test]
    fn interpreter_launch_for_resolves_the_program_carried_by_the_request() {
        let interpreter = crate::cli::CliSessionInterpreter::system();
        let launch = interpreter
            .launch_for(&CliToolId::new("codex").unwrap())
            .expect("codex must resolve to a launch program");
        let request = SpawnRequest {
            identity: identity("voice-42"),
            launch,
            cwd: std::path::PathBuf::from("/work/project"),
            title: None,
        };
        assert_eq!(request.launch.program, "codex");
        assert!(request.launch.args.is_empty());
    }

    struct Backend {
        id: BackendId,
        supported: Vec<SpawnSurface>,
        has_spawner: bool,
        spawned: std::sync::atomic::AtomicUsize,
        discover_identity: Option<SpawnIdentity>,
        spawned_backend: Option<BackendId>,
    }

    impl Backend {
        fn spawner_error(&self, request: &SpawnRequest) -> TerminalError {
            TerminalError::SpawnFailed {
                backend: self.id.clone(),
                message: format!("refused {}", request.identity.key),
            }
        }
    }

    impl SessionInventory for Backend {
        fn discover(&self) -> Result<Vec<crate::SessionFacts>, TerminalError> {
            Ok(vec![crate::SessionFacts {
                id: SessionId::new(self.id.clone(), "1").unwrap(),
                root_pid: 10,
                cwd: "/work/project".to_owned(),
                title: "Terminal".to_owned(),
                at_prompt: true,
                reported_cmd: None,
                foreground_basenames: Vec::new(),
                foreground_pids: Vec::new(),
                capabilities: crate::SessionCapabilities::ALL,
                spawn_identity: self.discover_identity.clone(),
            }])
        }
    }

    impl ScreenReader for Backend {
        fn read_screen(&self, _target: &SessionBinding) -> Result<String, TerminalError> {
            Err(TerminalError::Unsupported {
                target: _target.session_id().clone(),
                capability: "screen reading",
            })
        }
    }

    impl SessionFocus for Backend {
        fn focus(&self, _target: &SessionBinding) -> Result<(), TerminalError> {
            Ok(())
        }
    }

    impl TextInput for Backend {
        fn send_text(
            &self,
            _target: &SessionBinding,
            _text: &str,
            _mode: DeliveryMode,
        ) -> Result<(), TerminalError> {
            Ok(())
        }

        fn send_key(&self, _target: &SessionBinding, _key: &str) -> Result<(), TerminalError> {
            Ok(())
        }
    }

    impl TerminalBackend for Backend {
        fn read_screen_from_snapshot(
            &self,
            _snapshot: &TerminalSnapshot,
            target: &SessionBinding,
        ) -> Result<String, TerminalError> {
            Err(TerminalError::Unsupported {
                target: target.session_id().clone(),
                capability: "screen reading",
            })
        }

        fn id(&self) -> &BackendId {
            &self.id
        }

        fn spawner(&self) -> Option<&dyn SessionSpawner> {
            self.has_spawner.then_some(self as &dyn SessionSpawner)
        }
    }

    impl SessionSpawner for Backend {
        fn supports(&self, surface: SpawnSurface) -> bool {
            self.supported.contains(&surface)
        }

        fn spawn(&self, request: &SpawnRequest) -> Result<SessionId, TerminalError> {
            self.spawned
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if !self.supported.contains(&request.identity.surface) {
                return Err(self.spawner_error(request));
            }
            let backend = self
                .spawned_backend
                .clone()
                .unwrap_or_else(|| self.id.clone());
            SessionId::new(backend, format!("spawn-{}", request.identity.key)).map_err(|error| {
                TerminalError::SpawnFailed {
                    backend: self.id.clone(),
                    message: error.to_string(),
                }
            })
        }
    }

    #[test]
    fn spawn_on_uses_the_optional_capability_of_the_same_registered_backend() {
        let backend = Arc::new(Backend {
            id: BackendId::new("kitty").unwrap(),
            supported: vec![SpawnSurface::Tab],
            has_spawner: true,
            spawned: std::sync::atomic::AtomicUsize::new(0),
            discover_identity: None,
            spawned_backend: None,
        });
        let service = TerminalSessionService::from_backends([
            Arc::clone(&backend) as Arc<dyn TerminalBackend>
        ])
        .unwrap();

        let spawned = service
            .spawn_on(
                &BackendId::new("kitty").unwrap(),
                &request("lane-1", SpawnSurface::Tab),
            )
            .unwrap();
        assert_eq!(spawned.backend().to_string(), "kitty");
        assert_eq!(spawned.native(), "spawn-lane-1");
        assert_eq!(
            backend.spawned.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn spawn_on_refuses_surfaces_the_backend_capability_does_not_support() {
        let backend = Arc::new(Backend {
            id: BackendId::new("kitty").unwrap(),
            supported: vec![SpawnSurface::Tab],
            has_spawner: true,
            spawned: std::sync::atomic::AtomicUsize::new(0),
            discover_identity: None,
            spawned_backend: None,
        });
        let service = TerminalSessionService::from_backends([
            Arc::clone(&backend) as Arc<dyn TerminalBackend>
        ])
        .unwrap();

        let error = service
            .spawn_on(
                &BackendId::new("kitty").unwrap(),
                &request("lane-2", SpawnSurface::OsWindow),
            )
            .expect_err("os window spawning must be refused");
        assert!(error.to_string().contains("cannot spawn"));
        assert_eq!(
            backend.spawned.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn spawn_on_selects_the_backend_explicitly_by_id() {
        let backend = Arc::new(Backend {
            id: BackendId::new("kitty").unwrap(),
            supported: vec![SpawnSurface::Tab],
            has_spawner: true,
            spawned: std::sync::atomic::AtomicUsize::new(0),
            discover_identity: None,
            spawned_backend: None,
        });
        let service = TerminalSessionService::from_backends([
            Arc::clone(&backend) as Arc<dyn TerminalBackend>
        ])
        .unwrap();

        let error = service
            .spawn_on(
                &BackendId::new("other").unwrap(),
                &request("lane-3", SpawnSurface::Tab),
            )
            .expect_err("unregistered backends must fail");
        assert!(error.to_string().contains("not registered"));
        assert_eq!(
            backend.spawned.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn spawn_on_reports_spawn_unsupported_when_the_backend_has_no_capability() {
        let backend = Arc::new(Backend {
            id: BackendId::new("kitty").unwrap(),
            supported: vec![SpawnSurface::Tab],
            has_spawner: false,
            spawned: std::sync::atomic::AtomicUsize::new(0),
            discover_identity: None,
            spawned_backend: None,
        });
        let service = TerminalSessionService::from_backends([
            Arc::clone(&backend) as Arc<dyn TerminalBackend>
        ])
        .unwrap();

        let error = service
            .spawn_on(
                &BackendId::new("kitty").unwrap(),
                &request("lane-4", SpawnSurface::Tab),
            )
            .expect_err("backends without the capability must fail");
        assert!(error.to_string().contains("cannot spawn"));
        assert_eq!(
            backend.spawned.load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn discovery_carries_the_spawn_identity_to_consumers() {
        let backend = Arc::new(Backend {
            id: BackendId::new("kitty").unwrap(),
            supported: vec![],
            has_spawner: false,
            spawned: std::sync::atomic::AtomicUsize::new(0),
            discover_identity: Some(identity("voice-42")),
            spawned_backend: None,
        });
        let service = TerminalSessionService::from_backends([
            Arc::clone(&backend) as Arc<dyn TerminalBackend>
        ])
        .unwrap();

        let facts = service.discover().unwrap();
        assert_eq!(facts[0].spawn_identity, Some(identity("voice-42")));
    }

    #[test]
    fn spawn_on_rejects_a_session_id_owned_by_a_different_backend() {
        let backend = Arc::new(Backend {
            id: BackendId::new("kitty").unwrap(),
            supported: vec![SpawnSurface::Tab],
            has_spawner: true,
            spawned: std::sync::atomic::AtomicUsize::new(0),
            discover_identity: None,
            spawned_backend: Some(BackendId::new("other").unwrap()),
        });
        let service = TerminalSessionService::from_backends([
            Arc::clone(&backend) as Arc<dyn TerminalBackend>
        ])
        .unwrap();

        let error = service
            .spawn_on(
                &BackendId::new("kitty").unwrap(),
                &request("lane-5", SpawnSurface::Tab),
            )
            .expect_err("foreign session ids must be rejected");
        assert!(matches!(error, TerminalError::SpawnFailed { .. }));
        assert!(error.to_string().contains("different backend"));
        assert_eq!(
            backend.spawned.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }
}
