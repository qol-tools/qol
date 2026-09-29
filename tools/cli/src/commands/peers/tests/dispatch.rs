use super::*;
use crate::commands::peers::input::{Action, Mutation};

#[test]
fn convenience_mutations_use_one_fresh_stamp_and_dispatch_once() {
    for mutation in [
        Mutation::Stop,
        Mutation::Rename("new name".into()),
        Mutation::Revoke(peer()),
        Mutation::SetGrants(peer(), Vec::new()),
    ] {
        let mut client = FakeClient::new([Ok(status()), Ok(changed())]);
        let result = execute(Action::Mutation(mutation), &mut |request| {
            client.call(request)
        })
        .unwrap();
        assert_eq!(result, vec![changed()]);
        assert_eq!(client.requests.len(), 2);
        assert_eq!(client.requests[0], Request::Status);
        let expected = match &client.requests[1] {
            Request::Stop { expected }
            | Request::Rename { expected, .. }
            | Request::Revoke { expected, .. }
            | Request::SetGrants { expected, .. } => expected,
            request => panic!("unexpected mutation: {request:?}"),
        };
        assert_eq!(*expected, authority().expected());
    }
}

#[test]
fn explicit_start_intent_preserves_host_refusals_without_retry() {
    for request in [
        Request::StartSession {
            name: "fixture".into(),
        },
        Request::CreatePersistent {
            name: "fixture".into(),
        },
        Request::OpenPersistent,
    ] {
        for error in [
            qol_peers::admin::Error::Standby,
            qol_peers::admin::Error::AlreadyActive,
            qol_peers::admin::Error::Shutdown,
        ] {
            let reply = Response::Error { error };
            let mut client = FakeClient::new([Ok(status()), Ok(reply.clone())]);
            let result = execute(Action::Start(request.clone()), &mut |request| {
                client.call(request)
            });
            assert_eq!(
                result.unwrap_err().to_string(),
                serde_json::to_string(&reply).unwrap()
            );
            assert_eq!(client.requests, [Request::Status, request.clone()]);
        }
    }
}

#[test]
fn inactive_authority_allows_explicit_start_but_prevents_stamped_mutation() {
    let inactive = Response::Status {
        status: Status {
            lifecycle: Lifecycle::Inactive,
            authority: None,
        },
    };
    let request = Request::StartSession {
        name: "fixture".into(),
    };
    let mut client = FakeClient::new([Ok(inactive.clone()), Ok(changed())]);
    assert_eq!(
        execute(Action::Start(request.clone()), &mut |r| client.call(r)).unwrap(),
        vec![changed()]
    );
    assert_eq!(client.requests, [Request::Status, request]);
    let mut client = FakeClient::new([Ok(inactive)]);
    assert!(execute(Action::Mutation(Mutation::Stop), &mut |r| client.call(r)).is_err());
    assert_eq!(client.requests, [Request::Status]);
}

#[test]
fn stale_stamps_and_unknown_outcomes_are_never_replayed() {
    for reply in [
        Ok(Response::Error { error: qol_peers::admin::Error::StaleAuthority }),
        Err(anyhow::anyhow!("peer administration mutation outcome is unknown; inspect status before deciding what to do next")),
    ] {
        let mut client = FakeClient::new([Ok(status()), reply]);
        let result = execute(Action::Mutation(Mutation::Stop), &mut |r| client.call(r));
        assert!(result.is_err());
        assert_eq!(client.requests, [Request::Status, Request::Stop { expected: authority().expected() }]);
    }
}

#[test]
fn raw_canonical_requests_dispatch_once_without_rewriting_stamps() {
    let mut expected = authority().expected();
    expected.revision = StoreRevision::new(2);
    for request in [
        Request::Status,
        Request::Stop { expected },
        Request::Rename {
            expected,
            name: "fixture".into(),
        },
        Request::Peers { cursor: cursor(1) },
    ] {
        let wire = serde_json::to_vec(&request).unwrap();
        let action =
            crate::commands::peers::input::parse(&["request"], &mut wire.as_slice()).unwrap();
        let mut client = FakeClient::new([Ok(changed())]);
        assert_eq!(
            execute(action, &mut |r| client.call(r)).unwrap(),
            vec![changed()]
        );
        assert_eq!(client.requests, [request]);
    }
}

#[test]
fn errors_produce_nonzero_headless_execution_without_success_output() {
    fn refused(_: Option<&str>, _: &[String]) -> Result<Vec<Response>> {
        let mut client = FakeClient::new([Ok(Response::Error {
            error: qol_peers::admin::Error::Standby,
        })]);
        let result = execute(Action::Direct(Request::OpenPersistent), &mut |r| {
            client.call(r)
        });
        assert_eq!(client.requests, [Request::OpenPersistent]);
        result
    }
    fn unknown(_: Option<&str>, _: &[String]) -> Result<Vec<Response>> {
        let reply = Err(anyhow::anyhow!("mutation outcome is unknown"));
        let mut client = FakeClient::new([reply]);
        let result = execute(Action::Direct(Request::OpenPersistent), &mut |r| {
            client.call(r)
        });
        assert_eq!(client.requests, [Request::OpenPersistent]);
        result
    }
    for runner in [refused as crate::commands::peers::Runner, unknown] {
        for format in [Vec::new(), vec!["--json"]] {
            let app = qol_headless::HeadlessApp::new("qol", "qol")
                .command(crate::commands::peers::command_with_runner(runner));
            let args = ["peers", "request"].into_iter().chain(format);
            let execution = app.execute(args.map(str::to_string).collect::<Vec<_>>());
            assert_eq!(execution.exit_code, qol_headless::EXIT_RUNTIME_ERROR);
            assert!(execution.stdout.is_empty());
            assert!(!execution.stderr.is_empty());
        }
    }
}

#[test]
fn output_serializes_canonical_responses_without_a_cli_schema() {
    fn fixture(_: Option<&str>, _: &[String]) -> Result<Vec<Response>> {
        Ok(vec![status(), changed()])
    }
    let app = qol_headless::HeadlessApp::new("qol", "qol")
        .command(crate::commands::peers::command_with_runner(fixture));
    let plain = app.execute(["peers", "status"].map(str::to_string));
    let json = app.execute(["peers", "status", "--json"].map(str::to_string));
    assert_eq!(plain.exit_code, qol_headless::EXIT_SUCCESS);
    assert_eq!(json.exit_code, qol_headless::EXIT_SUCCESS);
    let lines: Vec<Response> = plain
        .stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let array: Vec<Response> = serde_json::from_str(&json.stdout).unwrap();
    assert_eq!(lines, [status(), changed()]);
    assert_eq!(array, lines);
}

#[test]
fn enrollment_admission_uses_one_stamp_and_never_retries_unknown_or_stale_results() {
    use crate::commands::peers::enrollment::EnrollmentAction;
    let transaction = "AAAAAAAAAAAAAAAAAAAAAA".parse().unwrap();
    for reply in [
        Ok(Response::Error {
            error: qol_peers::admin::Error::StaleAuthority,
        }),
        Err(anyhow::anyhow!("mutation outcome unknown")),
    ] {
        let mut client = FakeClient::new([Ok(status()), reply]);
        let action = Action::Mutation(Mutation::Enrollment(EnrollmentAction::Recover(
            transaction,
            vec!["192.0.2.1:1234".parse().unwrap()],
        )));
        assert!(execute(action, &mut |request| client.call(request)).is_err());
        assert_eq!(client.requests.len(), 2);
        assert_eq!(client.requests[0], Request::Status);
        assert!(
            matches!(&client.requests[1], Request::Enrollment { request: qol_peers::admin::EnrollmentRequest::Recover { expected, transaction: actual, .. } } if *expected == authority().expected() && *actual == transaction)
        );
    }
}
