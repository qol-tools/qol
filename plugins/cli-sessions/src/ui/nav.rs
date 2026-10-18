use crate::session::registry::SessionState;
use crate::session::status::Status;

/// Index of the attention row to jump to next, over a priority-sorted slice
/// (NeedsYou before YourTurn, recent before old). With no cursor the jump lands
/// on the highest-priority row; with a cursor it advances one step and wraps, so
/// repeated presses walk the priority order.
pub fn next_attention(statuses: &[Status], current: Option<usize>) -> Option<usize> {
    next_where(statuses.len(), current, |i| statuses[i].is_attention())
}

pub fn next_attention_row(rows: &[SessionState], current: Option<usize>) -> Option<usize> {
    next_where(rows.len(), current, |i| rows[i].wants_attention())
}

fn next_where(n: usize, current: Option<usize>, wanted: impl Fn(usize) -> bool) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let start = match current {
        Some(c) => (c + 1) % n,
        None => 0,
    };
    (0..n)
        .map(|offset| (start + offset) % n)
        .find(|&i| wanted(i))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::registry::Registry;

    fn row(window_id: u64, status: Status, master: bool) -> SessionState {
        SessionState {
            id: crate::host::kitty_session_id(window_id),
            root_pid: window_id as i32,
            project: "proj".into(),
            name: None,
            cwd: "/a/b/proj".into(),
            branch: None,
            tool: qol_terminal_sessions::cli::claude_tool(),
            status,
            summary: "x".into(),
            last_activity: 1,
            screen_hash: None,
            key: master.then(|| qol_sessions_engine::SWITCHBOARD_KEY.to_owned()),
            master,
            working_since: None,
            settled_since: None,
            bridged: false,
            driving: Vec::new(),
            runtime_status: None,
            unread: 0,
            open_asks: 0,
        }
    }

    #[test]
    fn next_skips_the_resting_master_but_lands_on_it_when_it_needs_you() {
        for (master_status, expected) in [
            (Status::YourTurn, vec![Some(1), Some(1)]),
            (Status::Acknowledged, vec![Some(1), Some(1)]),
            (Status::Unknown, vec![Some(1), Some(1)]),
            (Status::Working, vec![Some(1), Some(1)]),
            (Status::NeedsYou, vec![Some(0), Some(1)]),
        ] {
            let mut registry = Registry::default();
            registry.upsert(row(9, master_status, true));
            registry.upsert(row(1, Status::Working, false));
            registry.upsert(row(2, Status::YourTurn, false));
            let rows = registry.sorted();
            assert!(rows[0].master);
            assert_eq!(rows[1].status, Status::YourTurn);
            let picked = [None, Some(0)]
                .map(|current| next_attention_row(&rows, current))
                .to_vec();
            assert_eq!(picked, expected, "{master_status:?}");
        }
    }

    #[test]
    fn a_lone_resting_master_is_never_picked() {
        let rows = [row(9, Status::YourTurn, true)];
        assert_eq!(next_attention_row(&rows, None), None);
        assert_eq!(next_attention_row(&rows, Some(0)), None);
        assert_eq!(next_attention_row(&[], None), None);
    }
}
