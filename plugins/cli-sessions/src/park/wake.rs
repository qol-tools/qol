use anyhow::{anyhow, bail, Result};
use qol_sessions_engine::knocks;
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::park::{ParkRecord, ParkState};
use qol_terminal_sessions::{SessionBinding, TerminalSessionService};

use super::prompt::{message_wake_prompt, UNKNOCKED_WAKE};
use super::resume::resume;
use super::store::ParkStore;

pub fn wakeable(store: &ParkStore, key: &str) -> Option<ParkRecord> {
    if key == qol_sessions_engine::switchboard::SWITCHBOARD_KEY {
        return None;
    }
    let keyed = store.list().ok()?.into_iter().filter(|record| {
        store
            .spawn_key(&record.id)
            .is_some_and(|kept| kept.as_str() == key)
    });
    newest_wakeable(keyed)
}

fn newest_wakeable(records: impl IntoIterator<Item = ParkRecord>) -> Option<ParkRecord> {
    records
        .into_iter()
        .max_by_key(|record| record.created_at)
        .filter(lets_go)
}

fn lets_go(record: &ParkRecord) -> bool {
    match record.state {
        ParkState::Waiting => record.caller_closed,
        ParkState::Finished | ParkState::Resumed | ParkState::Delivered => true,
        ParkState::Exited | ParkState::Failed => false,
    }
}

pub fn wake(key: &str) -> Result<String> {
    let store = ParkStore::system()?;
    let record = wakeable(&store, key)
        .ok_or_else(|| anyhow!("`{key}` has no closed parked session to reopen"))?;
    let Some(_lock) = store.lock_unpark(&record.id)? else {
        bail!("`{}` is already being resumed or reopened", record.id);
    };
    let Some(_turn) = store.try_lock_conversation(&record.external_id)? else {
        bail!("`{}` is already being resumed", record.id);
    };
    let record = store.load(&record.id)?;
    if !lets_go(&record) {
        bail!("`{}` is no longer closed ({:?})", record.id, record.state);
    }
    let terminals = TerminalSessionService::system();
    let interpreter = CliSessionInterpreter::system();
    let caller = record
        .session
        .parse::<SessionBinding>()
        .map_err(|error| anyhow!("invalid parked session token: {error}"))?;
    super::ensure_not_open(&store, &terminals, &interpreter, &record, &caller)?;
    let claimed = knocks::claim(key)?;
    let knock = claimed
        .as_ref()
        .map_or_else(|| UNKNOCKED_WAKE.to_owned(), knocks::ClaimedKnock::text);
    let prompt = message_wake_prompt(&record, &knock);
    match resume(&terminals, &interpreter, &store, &record, &prompt) {
        Ok(session) => {
            qol_runtime::probe!(
                "CLI_SESSION_PARK",
                "event=woken_by_message id={} key={key} session={session}",
                record.id
            );
            Ok(format!("woke {key} from {} in {session}", record.id))
        }
        Err(error) => {
            if let Some(claimed) = claimed.as_ref() {
                if let Err(restore_error) = knocks::restore(claimed) {
                    log::warn!("[cli-sessions] knock for {key} not put back: {restore_error}");
                }
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::park::tests::record;

    fn parked(id: &str, created_at: u64, state: ParkState, caller_closed: bool) -> ParkRecord {
        ParkRecord {
            id: id.to_owned(),
            created_at,
            caller_closed,
            ..record(state)
        }
    }

    #[test]
    fn only_a_newest_park_that_let_go_of_its_terminal_wakes() {
        let cases = [
            (ParkState::Waiting, false, false),
            (ParkState::Waiting, true, true),
            (ParkState::Exited, true, false),
            (ParkState::Failed, true, false),
            (ParkState::Delivered, false, true),
            (ParkState::Resumed, true, true),
            (ParkState::Finished, false, true),
        ];
        for (state, caller_closed, expected) in cases {
            let older = parked("park-1-1", 1, ParkState::Finished, false);
            let newest = parked("park-2-2", 2, state, caller_closed);
            assert_eq!(
                newest_wakeable([newest, older]).map(|record| record.id),
                expected.then(|| "park-2-2".to_owned()),
                "{state:?} caller_closed={caller_closed}"
            );
        }
        assert!(newest_wakeable([]).is_none());
    }

    #[test]
    fn a_session_is_found_by_the_spawn_key_its_parks_kept() {
        let root = tempfile::TempDir::new().unwrap();
        let store = ParkStore::with_dir(root.path().join("parked"));
        let key = |name: &str| qol_terminal_sessions::SpawnKey::new(name).unwrap();
        for (id, created_at, lane) in [
            ("park-1-1", 1, "lane-a"),
            ("park-2-2", 2, "lane-a"),
            ("park-3-3", 3, "lane-b"),
        ] {
            store
                .record(&parked(id, created_at, ParkState::Finished, true))
                .unwrap();
            store.record_spawn_key(id, &key(lane)).unwrap();
        }
        assert_eq!(
            wakeable(&store, "lane-a").map(|record| record.id),
            Some("park-2-2".to_owned())
        );
        assert!(wakeable(&store, "lane-c").is_none());
        store
            .record(&parked("park-4-4", 4, ParkState::Finished, true))
            .unwrap();
        store
            .record_spawn_key("park-4-4", &key("switchboard"))
            .unwrap();
        assert!(wakeable(&store, "switchboard").is_none());
    }
}
