#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SyncAction {
    NoOp,
    FastForwardFromRemote,
    PushLocal,
    Conflict,
}

pub(crate) fn resolve_sync_action(
    local_hash: &str,
    remote_hash: &str,
    last_synced: Option<&str>,
) -> SyncAction {
    if local_hash == remote_hash {
        return SyncAction::NoOp;
    }
    match last_synced {
        Some(ls) if ls == local_hash => SyncAction::FastForwardFromRemote,
        Some(ls) if ls == remote_hash => SyncAction::PushLocal,
        _ => SyncAction::Conflict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_when_local_matches_remote() {
        assert_eq!(resolve_sync_action("A", "A", Some("A")), SyncAction::NoOp);
        assert_eq!(resolve_sync_action("A", "A", None), SyncAction::NoOp);
        assert_eq!(resolve_sync_action("A", "A", Some("Z")), SyncAction::NoOp);
    }

    #[test]
    fn fast_forward_when_local_equals_last_synced_and_remote_is_newer() {
        assert_eq!(
            resolve_sync_action("A", "B", Some("A")),
            SyncAction::FastForwardFromRemote
        );
    }

    #[test]
    fn push_local_when_remote_equals_last_synced_and_local_is_newer() {
        assert_eq!(
            resolve_sync_action("B", "A", Some("A")),
            SyncAction::PushLocal
        );
    }

    #[test]
    fn conflict_when_both_sides_diverged_from_last_synced() {
        assert_eq!(
            resolve_sync_action("B", "C", Some("A")),
            SyncAction::Conflict
        );
    }

    #[test]
    fn conflict_when_last_synced_is_unknown_and_hashes_differ() {
        assert_eq!(resolve_sync_action("A", "B", None), SyncAction::Conflict);
    }

    #[test]
    fn resolve_action_full_table() {
        let cases: &[(&str, &str, Option<&str>, SyncAction, &str)] = &[
            ("A", "A", Some("A"), SyncAction::NoOp, "fully-synced"),
            ("A", "A", None, SyncAction::NoOp, "matched-without-history"),
            (
                "A",
                "A",
                Some("Z"),
                SyncAction::NoOp,
                "matched-overrides-stale-history",
            ),
            (
                "A",
                "B",
                Some("A"),
                SyncAction::FastForwardFromRemote,
                "remote-advanced-from-known-base",
            ),
            (
                "B",
                "A",
                Some("A"),
                SyncAction::PushLocal,
                "local-advanced-from-known-base",
            ),
            (
                "B",
                "C",
                Some("A"),
                SyncAction::Conflict,
                "both-diverged-from-known-base",
            ),
            ("A", "B", None, SyncAction::Conflict, "first-sync-disagrees"),
            (
                "",
                "B",
                Some(""),
                SyncAction::FastForwardFromRemote,
                "empty-local-matches-empty-history",
            ),
            (
                "A",
                "",
                Some("A"),
                SyncAction::FastForwardFromRemote,
                "remote-cleared-from-known-base-this-is-the-wipe-path",
            ),
            ("", "", Some("A"), SyncAction::NoOp, "both-cleared"),
            ("", "", None, SyncAction::NoOp, "both-empty-no-history"),
            (
                "B",
                "B",
                Some("A"),
                SyncAction::NoOp,
                "drifted-but-now-aligned",
            ),
        ];
        for (local, remote, last, expected, label) in cases {
            assert_eq!(
                &resolve_sync_action(local, remote, *last),
                expected,
                "{label}: local={local:?} remote={remote:?} last_synced={last:?}",
            );
        }
    }
}
