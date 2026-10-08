use super::peer_test_support::{action, call, finished, pending, prepare, redeem, start};
use crate::features::linked_devices::tests::enrollment::Fixture;
use qol_peers::admin::{
    AttemptState, EnrollmentRequest, Request as AdminRequest, Response as AdminResponse,
};
use qol_peers::operations::{Failure, OperationBody, Outcome, Request, Response};
use qol_peers::service::network::discovery::{
    Advertisement, DiscoveryEvent, DiscoveryFactory, DiscoveryFuture,
};
use qol_plugin_api::operations::{OperationKey, OperationKind};
use qol_runtime::PlatformStateClient;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};

struct Mesh(watch::Sender<Vec<Advertisement>>);

impl Mesh {
    fn new() -> Self {
        Self(watch::channel(Vec::new()).0)
    }
}

impl DiscoveryFactory for Mesh {
    fn run(
        &self,
        local: Advertisement,
        events: mpsc::Sender<DiscoveryEvent>,
        mut stop: watch::Receiver<bool>,
    ) -> DiscoveryFuture {
        let local_peer = local.peer;
        self.0.send_modify(|peers| {
            peers.retain(|peer| peer.peer != local_peer);
            peers.push(local);
        });
        let mut peers = self.0.subscribe();
        Box::pin(async move {
            events.send(DiscoveryEvent::Ready).await.unwrap();
            loop {
                let hints = peers.borrow_and_update().clone();
                for peer in hints.into_iter().filter(|peer| peer.peer != local_peer) {
                    events
                        .send(DiscoveryEvent::Resolved {
                            source: peer.peer.to_string(),
                            peer: peer.peer,
                            endpoints: vec![std::net::SocketAddrV4::new(peer.bind, peer.port)],
                            claim: None,
                        })
                        .await
                        .unwrap();
                }
                tokio::select! {
                    result = stop.changed() => { if result.is_err() || *stop.borrow() { return Ok(()); } }
                    result = peers.changed() => { if result.is_err() { return Ok(()); } }
                }
            }
        })
    }
}

async fn connect(mesh: &Mesh, receiver: &Fixture, sender: &Fixture) {
    mesh.0.send_modify(|_| {});
    tokio::time::timeout(Duration::from_secs(5), async {
        while !receiver.operations_ready(sender.expected().authority_id)
            || !sender.operations_ready(receiver.expected().authority_id)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

fn key() -> OperationKey {
    OperationKey::new(
        qol_conventions::plugin_id::PluginUid::new("fixture-operation-uid"),
        OperationKind::Action,
        "count",
    )
}

fn body(arguments: &str) -> String {
    serde_json::to_string(&OperationBody {
        version: 1,
        key: key(),
        arguments: arguments.into(),
        timeout_ms: 10_000,
    })
    .unwrap()
}

async fn operation(
    fixture: &Fixture,
    request: Request,
    lose_local_reply: bool,
) -> Result<Response, qol_runtime::protocol::PeerAdminClientError> {
    operation_shared(fixture.shared.clone(), request, lose_local_reply).await
}

async fn operation_shared(
    shared: Arc<crate::runtime::SharedState>,
    request: Request,
    lose_local_reply: bool,
) -> Result<Response, qol_runtime::protocol::PeerAdminClientError> {
    tokio::task::spawn_blocking(move || {
        let temporary = tempfile::tempdir().unwrap();
        let socket = temporary.path().join("local.sock");
        let listener = qol_runtime::local_ipc::bind_listener(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            qol_runtime::local_ipc::authorize_peer(&stream).unwrap();
            if lose_local_reply {
                let request =
                    qol_runtime::local_ipc::read_secret_line(&mut std::io::BufReader::new(&stream))
                        .unwrap()
                        .unwrap();
                let (mut sink, _reply) = std::os::unix::net::UnixStream::pair().unwrap();
                super::super::handle_request(&request, &mut sink, &shared);
                return;
            }
            super::super::super::handle_connection(stream, &shared);
        });
        let result = PlatformStateClient::new(socket).peer_operation(request);
        server.join().unwrap();
        result
    })
    .await
    .unwrap()
}

async fn invoke(sender: &Fixture, receiver: &Fixture, arguments: &str) -> Response {
    operation(
        sender,
        Request::Invoke {
            expected: sender.expected(),
            peer: receiver.expected().authority_id,
            body: body(arguments),
        },
        false,
    )
    .await
    .unwrap()
}

async fn requests(
    sender: &Fixture,
    receiver: &Fixture,
) -> Vec<qol_peers::operations::RequestStatus> {
    let Response::Requests { requests } = operation(
        sender,
        Request::Requests {
            expected: sender.expected(),
            peer: receiver.expected().authority_id,
        },
        false,
    )
    .await
    .unwrap() else {
        panic!("requests");
    };
    requests
}

async fn cancel_pending(sender: &Fixture, receiver: &Fixture) {
    let handle = requests(sender, receiver)
        .await
        .last()
        .unwrap()
        .handle
        .clone();
    assert!(
        matches!(operation(sender, Request::Cancel { expected: sender.expected(), handle }, false).await.unwrap(), Response::Status { status } if status.outcome == Outcome::CancelledBeforeDispatch)
    );
}

fn install_daemon(fixture: &Fixture, root: &Path) -> std::path::PathBuf {
    std::fs::create_dir_all(root).unwrap();
    let executable = root.join("fixture-daemon");
    let current_exe = std::env::current_exe().unwrap();
    std::fs::hard_link(&current_exe, &executable)
        .or_else(|_| std::fs::copy(&current_exe, &executable).map(|_| ()))
        .unwrap();
    let socket = crate::dev_generation::daemon_socket_path("operation-fixture.sock");
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let count = root.join("count.json");
    std::fs::write(&count, "0").unwrap();
    let token = "0123456789abcdef0123456789abcdef";
    let manifest: crate::plugins::PluginManifest = toml::from_str(&format!(
        r#"
[plugin]
id = "qol-operation-fixture"
uid = "fixture-operation-uid"
name = "Operation fixture"
description = ""
version = "1.0.0"
[menu]
label = "Fixture"
items = []
[daemon]
enabled = true
command = "fixture-daemon"
socket = {:?}
[action.count]
label = "Count"
args = ["count"]
peer = {{ replay = "idempotent" }}
"#,
        socket.to_str().unwrap()
    ))
    .unwrap();
    let mut plugin = crate::plugins::Plugin::new(
        crate::plugins::PluginId::new("qol-operation-fixture"),
        manifest,
        root.into(),
    );
    let child = std::process::Command::new(&executable)
        .args(["--exact", "runtime::server::socket::platform::unix::requests::peer_admin::operation_tests::operation_fixture_daemon", "--nocapture"])
        .env_remove(qol_conventions::ENV_STATE_SOCKET)
        .env_remove(qol_conventions::ENV_DAEMON_LISTENER_FD)
        .env_remove(qol_conventions::ENV_DAEMON_PORT_FD)
        .env("QOL_OPERATION_FIXTURE_COUNT", &count)
        .env(qol_conventions::ENV_DAEMON_SOCKET, &socket)
        .env(qol_conventions::ENV_DAEMON_INSTANCE, token)
        .spawn().unwrap();
    let artifact = crate::plugins::action_executor::remote::register_fixture_artifact(executable);
    crate::plugins::register_operation_fixture(&mut plugin, child, token.into(), artifact);
    fixture
        .plugins()
        .lock()
        .unwrap()
        .insert_plugin_for_test(plugin);
    count
}

async fn daemon_ready() {
    let socket = crate::dev_generation::daemon_socket_path("operation-fixture.sock");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let socket = socket.clone();
            if tokio::task::spawn_blocking(move || {
                crate::plugins::action_transport::probe_operation_instance(
                    &socket,
                    "0123456789abcdef0123456789abcdef",
                    Duration::from_millis(100),
                )
            })
            .await
            .unwrap()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[test]
fn operation_fixture_daemon() {
    let Some(count) = std::env::var_os("QOL_OPERATION_FIXTURE_COUNT") else {
        return;
    };
    let config = qol_plugin_daemon::daemon::DaemonConfig {
        socket: qol_plugin_daemon::daemon::SocketSource::EnvRequired,
        support_replace_existing: false,
    };
    qol_plugin_daemon::daemon::run_stateful_request_listener(&config, (), |_, request| {
        use qol_plugin_daemon::daemon::ReadResult;
        match request.action.as_str() {
            "kill" => ReadResult::Handled,
            "count" => {
                let previous: u64 =
                    serde_json::from_slice(&std::fs::read(&count).unwrap()).unwrap();
                let current = previous + 1;
                std::fs::write(&count, current.to_string()).unwrap();
                if request
                    .input
                    .get("barrier")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                {
                    let root = Path::new(&count).parent().unwrap();
                    std::fs::write(root.join("started"), "ready").unwrap();
                    let deadline = std::time::Instant::now() + Duration::from_secs(10);
                    while !root.join("release").exists() && std::time::Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
                if request
                    .input
                    .get("lose")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
                {
                    std::process::exit(0);
                }
                ReadResult::HandledWithData(serde_json::json!({ "count": current }))
            }
            _ => ReadResult::Fallback,
        }
    })
    .unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn peer_operation_two_hosts_real_tls_local_api_catalog_executor_and_daemon() {
    let temporary = tempfile::tempdir().unwrap();
    let _paths = crate::paths::push_test_path_root(temporary.path());
    let mesh = Arc::new(Mesh::new());
    let receiver = Fixture::with_discovery(&temporary.path().join("receiver"), mesh.clone()).await;
    let sender = Fixture::with_discovery(&temporary.path().join("sender"), mesh.clone()).await;
    start(&receiver, true, "receiver").await;
    start(&sender, true, "sender").await;
    let daemon = temporary.path().join("daemon");
    let count = install_daemon(&receiver, &daemon);
    daemon_ready().await;
    let (document, transaction) = prepare(&receiver, &sender).await;
    redeem(&sender, document, transaction).await;
    let enrollment = pending(&receiver).await;
    assert!(matches!(
        action(
            &receiver,
            EnrollmentRequest::Approve {
                expected: receiver.expected(),
                key: enrollment
            }
        )
        .await,
        AdminResponse::Changed { .. }
    ));
    assert!(matches!(
        finished(&sender, transaction).await,
        AttemptState::Completed { .. }
    ));
    connect(&mesh, &receiver, &sender).await;
    assert_eq!(
        invoke(&sender, &receiver, "{}").await,
        Response::Error {
            error: Failure::GrantRequired
        }
    );
    assert_eq!(std::fs::read_to_string(&count).unwrap(), "0");
    cancel_pending(&sender, &receiver).await;
    assert!(matches!(
        call(
            &receiver,
            AdminRequest::SetGrants {
                expected: receiver.expected(),
                peer_id: sender.expected().authority_id,
                grants: vec![key()]
            }
        )
        .await,
        AdminResponse::Changed { .. }
    ));
    assert_eq!(
        invoke(&sender, &receiver, r#"{"n":1.0}"#).await,
        Response::Error {
            error: Failure::InvalidBody
        }
    );
    assert_eq!(std::fs::read_to_string(&count).unwrap(), "0");
    let plugins = receiver.plugins();
    let original_manifest = plugins
        .lock()
        .unwrap()
        .operation_plugin_mut_for_test("qol-operation-fixture")
        .manifest
        .clone();
    for denied in ["absent", "local", "duplicate"] {
        {
            let mut manager = plugins.lock().unwrap();
            let plugin = manager.operation_plugin_mut_for_test("qol-operation-fixture");
            match denied {
                "absent" => plugin.manifest.actions["count"].peer = None,
                "local" => {
                    plugin.manifest.plugin.uid = None;
                    plugin.manifest.actions["count"].peer = None;
                }
                "duplicate" => {
                    let duplicate = crate::plugins::Plugin::new(
                        crate::plugins::PluginId::new("qol-duplicate-fixture"),
                        original_manifest.clone(),
                        daemon.clone(),
                    );
                    manager.insert_plugin_for_test(duplicate);
                }
                _ => unreachable!(),
            }
        }
        assert_eq!(
            invoke(&sender, &receiver, "{}").await,
            Response::Error {
                error: Failure::ChangedDeclaration
            }
        );
        assert_eq!(std::fs::read_to_string(&count).unwrap(), "0");
        cancel_pending(&sender, &receiver).await;
        let mut manager = plugins.lock().unwrap();
        manager
            .operation_plugin_mut_for_test("qol-operation-fixture")
            .manifest = original_manifest.clone();
        if denied == "duplicate" {
            manager
                .operation_plugin_mut_for_test("qol-duplicate-fixture")
                .manifest
                .plugin
                .uid = Some(qol_conventions::plugin_id::PluginUid::new(
                "separate-fixture-uid",
            ));
        }
    }
    {
        use crate::plugins::action_executor::remote::PreparedRemote;
        let body: OperationBody = serde_json::from_str(&body("{}")).unwrap();
        let captured = PreparedRemote::capture(&plugins.lock().unwrap()).unwrap();
        let prepared = PreparedRemote::prepare(captured, &body).unwrap();
        let mut manager = plugins.lock().unwrap();
        manager
            .operation_plugin_mut_for_test("qol-operation-fixture")
            .manifest
            .actions["count"]
            .label = "Changed".into();
        assert_eq!(prepared.recheck(&manager), Err(Failure::ChangedDeclaration));
        manager
            .operation_plugin_mut_for_test("qol-operation-fixture")
            .manifest = original_manifest.clone();
        let captured = PreparedRemote::capture(&manager).unwrap();
        drop(manager);
        let prepared = PreparedRemote::prepare(captured, &body).unwrap();
        let mut manager = plugins.lock().unwrap();
        let child = manager.operation_replace_daemon_for_test("qol-operation-fixture", None);
        assert_eq!(prepared.recheck(&manager), Err(Failure::ChangedInstance));
        manager.operation_replace_daemon_for_test("qol-operation-fixture", child);
        assert_eq!(std::fs::read_to_string(&count).unwrap(), "0");
    }
    let Response::Status { status } = invoke(&sender, &receiver, "{}").await else {
        panic!("durable status");
    };
    assert_eq!(
        status.outcome,
        Outcome::Result {
            document: r#"{"count":1}"#.into()
        }
    );
    for _ in 0..3 {
        let reply = operation(
            &sender,
            Request::Outcome {
                expected: sender.expected(),
                handle: status.handle.clone(),
            },
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            reply,
            Response::Status {
                status: status.clone()
            }
        );
    }
    let matching = qol_peers::operations::Invocation {
        handle: status.handle.clone(),
        body: body("{}"),
    };
    let incoming = sender.send_operation_fixture(matching.clone());
    assert_eq!(
        tokio::task::spawn_blocking(move || incoming.recv_timeout(Duration::from_secs(5)).unwrap())
            .await
            .unwrap(),
        Ok(status.outcome.clone())
    );
    for failure in [Failure::Conflict, Failure::OldEpoch, Failure::Gap] {
        let mut invalid = matching.clone();
        match failure {
            Failure::Conflict => {
                invalid.handle.id =
                    qol_peers::operations::RequestId("AQEBAQEBAQEBAQEBAQEBAQ".parse().unwrap())
            }
            Failure::OldEpoch => {
                invalid.handle.epoch =
                    qol_peers::operations::OperationEpoch("AgICAgICAgICAgICAgICAg".parse().unwrap())
            }
            Failure::Gap => {
                invalid.handle.sequence =
                    qol_peers::StoreRevision::new(invalid.handle.sequence.value() + 2)
            }
            _ => unreachable!(),
        }
        let incoming = sender.send_operation_fixture(invalid);
        assert_eq!(
            tokio::task::spawn_blocking(move || incoming
                .recv_timeout(Duration::from_secs(5))
                .unwrap())
            .await
            .unwrap(),
            Err(failure)
        );
    }
    assert_eq!(std::fs::read_to_string(&count).unwrap(), "1");
    let lost = operation(
        &sender,
        Request::Invoke {
            expected: sender.expected(),
            peer: receiver.expected().authority_id,
            body: body("{}"),
        },
        true,
    )
    .await;
    assert_eq!(
        lost,
        Err(qol_runtime::protocol::PeerAdminClientError::OutcomeUnknown)
    );
    let recovered = requests(&sender, &receiver).await.pop().unwrap();
    assert_eq!(
        recovered.outcome,
        Outcome::Result {
            document: r#"{"count":2}"#.into()
        }
    );
    assert_eq!(std::fs::read_to_string(&count).unwrap(), "2");
    assert!(matches!(
        call(
            &receiver,
            AdminRequest::SetGrants {
                expected: receiver.expected(),
                peer_id: sender.expected().authority_id,
                grants: vec![]
            }
        )
        .await,
        AdminResponse::Changed { .. }
    ));
    assert_eq!(
        operation(
            &sender,
            Request::Outcome {
                expected: sender.expected(),
                handle: recovered.handle.clone()
            },
            false
        )
        .await
        .unwrap(),
        Response::Status { status: recovered }
    );
    assert!(matches!(
        call(
            &receiver,
            AdminRequest::SetGrants {
                expected: receiver.expected(),
                peer_id: sender.expected().authority_id,
                grants: vec![key()]
            }
        )
        .await,
        AdminResponse::Changed { .. }
    ));
    let blocked = tokio::spawn(operation_shared(
        sender.shared.clone(),
        Request::Invoke {
            expected: sender.expected(),
            peer: receiver.expected().authority_id,
            body: body(r#"{"barrier":true}"#),
        },
        false,
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        while !daemon.join("started").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let original = requests(&sender, &receiver)
        .await
        .last()
        .unwrap()
        .handle
        .clone();
    let duplicate = qol_peers::operations::Invocation {
        handle: original.clone(),
        body: body(r#"{"barrier":true}"#),
    };
    let incoming = sender.send_operation_fixture(duplicate);
    assert_eq!(
        tokio::task::spawn_blocking(move || incoming.recv_timeout(Duration::from_secs(5)).unwrap())
            .await
            .unwrap(),
        Ok(Outcome::DispatchStarted)
    );
    assert_eq!(std::fs::read_to_string(&count).unwrap(), "3");
    let sender_id = sender.expected().authority_id;
    sender.close().await;
    assert!(
        matches!(blocked.await.unwrap().unwrap(), Response::Unknown { handle: Some(handle) } if handle == original)
    );
    std::fs::write(daemon.join("release"), "release").unwrap();
    let sender = Fixture::with_discovery(&temporary.path().join("sender"), mesh.clone()).await;
    sender.ready().await;
    assert_eq!(sender.expected().authority_id, sender_id);
    connect(&mesh, &receiver, &sender).await;
    let pending = requests(&sender, &receiver).await.last().unwrap().clone();
    assert_eq!(pending.handle, original);
    assert_eq!(pending.outcome, Outcome::Unknown);
    let reconciled = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let reply = operation(
                &sender,
                Request::Outcome {
                    expected: sender.expected(),
                    handle: original.clone(),
                },
                false,
            )
            .await
            .unwrap();
            if let Response::Status { status } = reply {
                if status.outcome.terminal() {
                    break status;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        reconciled.outcome,
        Outcome::Result {
            document: r#"{"count":3}"#.into()
        }
    );
    assert_eq!(std::fs::read_to_string(&count).unwrap(), "3");
    let Response::Status { status } = invoke(&sender, &receiver, r#"{"lose":true}"#).await else {
        panic!("unknown status");
    };
    assert_eq!(status.outcome, Outcome::Unknown);
    assert_eq!(std::fs::read_to_string(&count).unwrap(), "4");
    let receiver_id = receiver.expected().authority_id;
    let sender_id = sender.expected().authority_id;
    let sender_root = temporary.path().join("sender");
    sender.close().await;
    let reopened = Fixture::with_discovery(&sender_root, mesh.clone()).await;
    reopened.ready().await;
    assert_eq!(reopened.expected().authority_id, sender_id);
    connect(&mesh, &receiver, &reopened).await;
    assert_eq!(receiver.expected().authority_id, receiver_id);
    assert_eq!(
        operation(
            &reopened,
            Request::Outcome {
                expected: reopened.expected(),
                handle: status.handle.clone()
            },
            false
        )
        .await
        .unwrap(),
        Response::Status { status }
    );
    assert_eq!(std::fs::read_to_string(&count).unwrap(), "4");
    reopened.close().await;
    receiver.plugins().lock().unwrap().shutdown();
    receiver.close().await;
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn peer_operation_external_verification_releases_guards_and_refuses_changed_selection() {
    let temporary = tempfile::tempdir().unwrap();
    let _paths = crate::paths::push_test_path_root(temporary.path());
    let mesh = Arc::new(Mesh::new());
    let receiver = Fixture::with_discovery(&temporary.path().join("receiver"), mesh.clone()).await;
    let sender = Fixture::with_discovery(&temporary.path().join("sender"), mesh.clone()).await;
    start(&receiver, true, "receiver").await;
    start(&sender, true, "sender").await;
    let daemon = temporary.path().join("daemon");
    let count = install_daemon(&receiver, &daemon);
    daemon_ready().await;
    let (document, transaction) = prepare(&receiver, &sender).await;
    redeem(&sender, document, transaction).await;
    let enrollment = pending(&receiver).await;
    assert!(matches!(
        action(
            &receiver,
            EnrollmentRequest::Approve {
                expected: receiver.expected(),
                key: enrollment,
            }
        )
        .await,
        AdminResponse::Changed { .. }
    ));
    assert!(matches!(
        finished(&sender, transaction).await,
        AttemptState::Completed { .. }
    ));
    connect(&mesh, &receiver, &sender).await;
    assert!(matches!(
        call(
            &receiver,
            AdminRequest::SetGrants {
                expected: receiver.expected(),
                peer_id: sender.expected().authority_id,
                grants: vec![key()],
            }
        )
        .await,
        AdminResponse::Changed { .. }
    ));
    let plugins = receiver.plugins();
    let original = plugins
        .lock()
        .unwrap()
        .get("qol-operation-fixture")
        .unwrap()
        .manifest
        .clone();
    for (change, skip) in [
        ("manifest", 0),
        ("manifest", 1),
        ("duplicate", 0),
        ("duplicate", 1),
        ("runtime", 0),
    ] {
        if change == "runtime" {
            plugins
                .lock()
                .unwrap()
                .operation_plugin_mut_for_test("qol-operation-fixture")
                .manifest
                .actions["count"]
                .peer = None;
            std::fs::write(daemon.join("qol-runtime.toml"), "schema_version = 1\n[action.count]\ndescription = \"Original runtime declaration\"\npeer = { replay = \"idempotent\" }\n").unwrap();
        }
        let (entered, resume) =
            crate::plugins::action_executor::remote::register_fixture_verification_pause(
                &daemon.join("fixture-daemon"),
                skip,
            );
        let invocation = tokio::spawn(operation_shared(
            sender.shared.clone(),
            Request::Invoke {
                expected: sender.expected(),
                peer: receiver.expected().authority_id,
                body: body("{}"),
            },
            false,
        ));
        tokio::task::spawn_blocking(move || entered.recv_timeout(Duration::from_secs(5)).unwrap())
            .await
            .unwrap();
        let handle = receiver.owner.handle();
        let root = daemon.clone();
        let manifest = original.clone();
        let acquired = tokio::task::spawn_blocking(move || {
            let (changed, observed) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                handle.change_operation_selection_for_test(|manager| {
                    match change {
                    "manifest" => {
                        manager.operation_plugin_mut_for_test("qol-operation-fixture").manifest.actions["count"].label = "Changed".into();
                    }
                    "duplicate" => {
                        manager.insert_plugin_for_test(crate::plugins::Plugin::new(
                            crate::plugins::PluginId::new("qol-duplicate-fixture"), manifest, root,
                        ));
                    }
                    "runtime" => {
                        std::fs::write(root.join("qol-runtime.toml"), "schema_version = 1\n[action.count]\ndescription = \"Changed runtime declaration\"\npeer = { replay = \"idempotent\" }\n").unwrap();
                    }
                        _ => unreachable!(),
                    }
                });
                changed.send(()).unwrap();
            });
            let acquired = observed.recv_timeout(Duration::from_secs(5)).is_ok();
            resume.send(()).unwrap();
            worker.join().unwrap();
            acquired
        }).await.unwrap();
        assert!(
            acquired,
            "host and manager must be available during external verification: {change}/{skip}"
        );
        let response = invocation.await.unwrap().unwrap();
        if skip == 0 && change != "runtime" {
            assert_eq!(
                response,
                Response::Error {
                    error: Failure::ChangedDeclaration
                },
                "{change}/{skip}"
            );
            cancel_pending(&sender, &receiver).await;
        } else {
            let Response::Status { status } = response else {
                panic!("durable refusal required: {change}/{skip}: {response:?}");
            };
            assert_eq!(
                status.outcome,
                Outcome::Refused {
                    reason: Failure::ChangedDeclaration
                },
                "{change}/{skip}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(&count).unwrap(),
            "0",
            "{change}/{skip}"
        );
        let mut manager = plugins.lock().unwrap();
        manager
            .operation_plugin_mut_for_test("qol-operation-fixture")
            .manifest = original.clone();
        if change == "duplicate" {
            manager
                .operation_plugin_mut_for_test("qol-duplicate-fixture")
                .manifest
                .plugin
                .uid = Some(qol_conventions::plugin_id::PluginUid::new(
                "separate-fixture-uid",
            ));
        }
    }
    plugins.lock().unwrap().shutdown();
    sender.close().await;
    receiver.close().await;
}
