use super::*;

#[test]
fn retained_unknown_results_are_not_evicted_to_admit_a_new_transaction() {
    let authority = PeerAuthority::session("local".into(), std::time::SystemTime::now()).unwrap();
    let remote = PeerAuthority::session("remote".into(), std::time::SystemTime::now()).unwrap();
    let invitation = remote
        .create_invitation(vec!["127.0.0.1:1234".parse().unwrap()])
        .unwrap();
    let transaction = authority
        .prepare_join(authority.projection().unwrap().revision, &invitation)
        .unwrap();
    let (control, _owner) = prepare(true);
    for index in 0..=MAX_RESULTS {
        let key = TransactionId::from_random((index as u128).to_be_bytes());
        if key == transaction {
            continue;
        }
        if control.attempts.lock().unwrap().len() == MAX_RESULTS {
            break;
        }
        control.attempts.lock().unwrap().insert(
            key,
            Attempt {
                state: AttemptState::Unknown {
                    reason: EnrollmentFailure::Transport,
                },
                cancel: None,
            },
        );
    }
    let result = control.admit(
        &authority,
        authority.projection().unwrap().revision,
        transaction,
        Some(invitation.export().unwrap()),
        Vec::new(),
    );
    assert_eq!(result, Err(refusal(EnrollmentFailure::Capacity)));
    assert_eq!(control.attempts.lock().unwrap().len(), MAX_RESULTS);
    assert!(control
        .attempts
        .lock()
        .unwrap()
        .values()
        .all(|attempt| matches!(
            attempt.state,
            AttemptState::Unknown {
                reason: EnrollmentFailure::Transport
            }
        )));
}
