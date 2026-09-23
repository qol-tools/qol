use super::watch::{Latch, LatchAction};
use super::Observation;

fn mismatch(loaded: &str, on_disk: &str) -> Observation {
    Observation::Mismatch {
        loaded: loaded.to_string(),
        on_disk: on_disk.to_string(),
    }
}

fn pending(loaded: &str, on_disk: &str) -> Latch {
    Latch::Pending {
        loaded: loaded.to_string(),
        on_disk: on_disk.to_string(),
    }
}

#[test]
fn unsupported_and_unavailable_preserve_the_latch() {
    let observations = [
        Observation::Unsupported,
        Observation::LoadedUnavailable,
        Observation::OnDiskUnavailable {
            loaded: "580.159.02".to_string(),
        },
    ];
    for observation in observations {
        let (latch, action) = Latch::next(Latch::Idle, &observation);
        assert_eq!(latch, Latch::Idle, "{observation:?}");
        assert_eq!(action, LatchAction::Preserved, "{observation:?}");
        let (latch, action) = Latch::next(pending("580.159.02", "580.173.00"), &observation);
        assert_eq!(
            latch,
            pending("580.159.02", "580.173.00"),
            "{observation:?}"
        );
        assert_eq!(action, LatchAction::Preserved, "{observation:?}");
    }
}

#[test]
fn positive_states_clear_the_latch() {
    let observations = [
        Observation::NotLoaded,
        Observation::Matched {
            loaded: "580.159.02".to_string(),
        },
    ];
    for observation in observations {
        let (latch, action) = Latch::next(Latch::Idle, &observation);
        assert_eq!(latch, Latch::Idle, "{observation:?}");
        assert_eq!(action, LatchAction::Cleared, "{observation:?}");
        let (latch, action) = Latch::next(pending("580.159.02", "580.173.00"), &observation);
        assert_eq!(latch, Latch::Idle, "{observation:?}");
        assert_eq!(action, LatchAction::Cleared, "{observation:?}");
    }
}

#[test]
fn new_mismatch_sends_once_and_the_same_pair_dedupes() {
    let first = mismatch("580.159.02", "580.173.00");
    let (latch, action) = Latch::next(Latch::Idle, &first);
    assert_eq!(action, LatchAction::Sent);
    assert_eq!(latch, pending("580.159.02", "580.173.00"));
    let (latch, action) = Latch::next(latch, &first);
    assert_eq!(action, LatchAction::Deduped);
    assert_eq!(latch, pending("580.159.02", "580.173.00"));
}

#[test]
fn changed_mismatch_pair_sends_again() {
    let (latch, action) = Latch::next(
        pending("580.159.02", "580.173.00"),
        &mismatch("580.159.02", "580.200.00"),
    );
    assert_eq!(action, LatchAction::Sent);
    assert_eq!(latch, pending("580.159.02", "580.200.00"));
}

#[test]
fn mismatch_to_unavailable_to_same_mismatch_never_resends() {
    let first = mismatch("580.159.02", "580.173.00");
    let (latch, action) = Latch::next(Latch::Idle, &first);
    assert_eq!(action, LatchAction::Sent);
    let (latch, action) = Latch::next(latch, &Observation::LoadedUnavailable);
    assert_eq!(action, LatchAction::Preserved);
    assert_eq!(latch, pending("580.159.02", "580.173.00"));
    let (latch, action) = Latch::next(latch, &first);
    assert_eq!(action, LatchAction::Deduped);
    assert_eq!(latch, pending("580.159.02", "580.173.00"));
}

#[test]
fn mismatch_after_a_positive_clear_resends() {
    let versions = mismatch("580.159.02", "580.173.00");
    let (latch, action) = Latch::next(Latch::Idle, &versions);
    assert_eq!(action, LatchAction::Sent);
    let (latch, action) = Latch::next(latch, &Observation::NotLoaded);
    assert_eq!(action, LatchAction::Cleared);
    assert_eq!(latch, Latch::Idle);
    let (latch, action) = Latch::next(latch, &versions);
    assert_eq!(action, LatchAction::Sent);
    assert_eq!(latch, pending("580.159.02", "580.173.00"));
}
