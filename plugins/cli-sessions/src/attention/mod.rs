use qol_terminal_sessions::cli::{CliRuntimeState, CliViewportState};

use crate::session::status::Status;

pub const GRACE_SECS: u64 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Busy,
    Service,
    Blocked,
    Done,
    Idle,
    Hold,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Evidence {
    pub descriptor_runtime: CliRuntimeState,
    pub screen_runtime: CliRuntimeState,
    pub viewport: CliViewportState,
    pub file_fresh: Option<bool>,
    pub file_quiet_secs: Option<u64>,
    pub screen_changed: bool,
    pub at_prompt: bool,
    pub is_generic: bool,
    pub is_service: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Attention {
    pub status: Status,
    pub working_since: Option<u64>,
    pub settled_since: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reason {
    LiveWork,
    StrongNeedsInput,
    GraceCompleted,
    QuickIdle,
    Service,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Transition {
    pub reason: Reason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reduction {
    pub attention: Attention,
    pub phase: Phase,
    pub transition: Option<Transition>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionPolicy {
    Explicit,
    Quiescent,
}

pub fn reduce(prev: &Attention, ev: &Evidence, now: u64) -> Reduction {
    reduce_with_policy(prev, ev, now, CompletionPolicy::Quiescent)
}

pub fn reduce_with_policy(
    prev: &Attention,
    ev: &Evidence,
    now: u64,
    policy: CompletionPolicy,
) -> Reduction {
    let explicit = policy == CompletionPolicy::Explicit;
    let working_since = prev.working_since;
    let settled_since = prev.settled_since;
    if !timestamps_are_monotonic(working_since, settled_since, now) {
        return Reduction {
            attention: Attention {
                status: prev.status,
                working_since: None,
                settled_since: None,
            },
            phase: Phase::Hold,
            transition: None,
        };
    }
    let settled = !ev.screen_changed;
    let fresh = ev.file_fresh == Some(true);
    let stale = ev.file_fresh == Some(false);
    let authoritative_ready = ev.descriptor_runtime == CliRuntimeState::Ready;
    let settle_start = settled_since.unwrap_or(now);
    let writing_during_settle = fresh
        && ev
            .file_quiet_secs
            .is_some_and(|quiet| now.saturating_sub(settle_start) > quiet);
    let next_settled_since = if settled && (!writing_during_settle || authoritative_ready) {
        Some(settle_start)
    } else {
        None
    };
    let grace_elapsed =
        next_settled_since.is_some_and(|start| now.saturating_sub(start) >= GRACE_SECS);
    let screen_working = ev.screen_runtime == CliRuntimeState::Working
        && (next_settled_since.is_none() || (explicit && ev.viewport == CliViewportState::Live));
    if ev.descriptor_runtime == CliRuntimeState::Working || screen_working {
        return Reduction {
            attention: Attention {
                status: Status::Working,
                working_since: Some(working_since.unwrap_or(now)),
                settled_since: None,
            },
            phase: Phase::Busy,
            transition: transition(prev.status, Status::Working, Reason::LiveWork),
        };
    }
    if ev.viewport == CliViewportState::Historical {
        return Reduction {
            attention: Attention {
                status: prev.status,
                working_since,
                settled_since,
            },
            phase: Phase::Hold,
            transition: None,
        };
    }
    let screen_needs_input = ev.screen_runtime == CliRuntimeState::NeedsInput
        && !stale
        && !(prev.status == Status::Working && (writing_during_settle || !settled))
        && !(prev.status == Status::Working && ev.file_fresh.is_none() && !grace_elapsed);
    let needs_input = ev.descriptor_runtime == CliRuntimeState::NeedsInput || screen_needs_input;
    if needs_input {
        return Reduction {
            attention: Attention {
                status: Status::NeedsYou,
                working_since: None,
                settled_since: None,
            },
            phase: Phase::Blocked,
            transition: transition(prev.status, Status::NeedsYou, Reason::StrongNeedsInput),
        };
    }
    if ev.at_prompt {
        let episode = working_since.map(|start| now.saturating_sub(start));
        if matches!(prev.status, Status::Working | Status::Service)
            && episode.is_some_and(|elapsed| elapsed >= GRACE_SECS)
        {
            return Reduction {
                attention: Attention {
                    status: Status::YourTurn,
                    working_since: None,
                    settled_since: None,
                },
                phase: Phase::Done,
                transition: transition(prev.status, Status::YourTurn, Reason::GraceCompleted),
            };
        }
        let status = if prev.status == Status::Acknowledged {
            Status::Acknowledged
        } else {
            Status::Unknown
        };
        return Reduction {
            attention: Attention {
                status,
                working_since: None,
                settled_since: None,
            },
            phase: Phase::Idle,
            transition: transition(prev.status, status, Reason::QuickIdle),
        };
    }
    if ev.is_service {
        return Reduction {
            attention: Attention {
                status: Status::Service,
                working_since: Some(working_since.unwrap_or(now)),
                settled_since: None,
            },
            phase: Phase::Service,
            transition: transition(prev.status, Status::Service, Reason::Service),
        };
    }
    if ev.is_generic {
        return Reduction {
            attention: Attention {
                status: Status::Working,
                working_since: Some(working_since.unwrap_or(now)),
                settled_since: None,
            },
            phase: Phase::Busy,
            transition: transition(prev.status, Status::Working, Reason::LiveWork),
        };
    }
    if prev.status == Status::NeedsYou && settled && grace_elapsed {
        return Reduction {
            attention: Attention {
                status: Status::Unknown,
                working_since: None,
                settled_since: None,
            },
            phase: Phase::Idle,
            transition: transition(prev.status, Status::Unknown, Reason::QuickIdle),
        };
    }
    let completion = !explicit
        || ev.descriptor_runtime == CliRuntimeState::Ready
        || ev.screen_runtime == CliRuntimeState::Ready;
    if prev.status == Status::Working && settled && grace_elapsed && completion {
        return Reduction {
            attention: Attention {
                status: Status::YourTurn,
                working_since: None,
                settled_since: None,
            },
            phase: Phase::Done,
            transition: transition(prev.status, Status::YourTurn, Reason::GraceCompleted),
        };
    }
    Reduction {
        attention: Attention {
            status: prev.status,
            working_since,
            settled_since: next_settled_since,
        },
        phase: Phase::Hold,
        transition: None,
    }
}

fn timestamps_are_monotonic(
    working_since: Option<u64>,
    settled_since: Option<u64>,
    now: u64,
) -> bool {
    working_since.is_none_or(|start| now >= start) && settled_since.is_none_or(|start| now >= start)
}

fn transition(from: Status, to: Status, reason: Reason) -> Option<Transition> {
    (from != to).then_some(Transition { reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_terminal_sessions::cli::{CliRuntimeState as RT, CliViewportState as VP};

    fn att(status: Status) -> Attention {
        Attention {
            status,
            working_since: None,
            settled_since: None,
        }
    }

    fn working(status: Status, since: u64) -> Attention {
        Attention {
            status,
            working_since: Some(since),
            settled_since: Some(since),
        }
    }

    fn evidence(
        descriptor: RT,
        screen: RT,
        viewport: VP,
        fresh: Option<bool>,
        changed: bool,
    ) -> Evidence {
        Evidence {
            descriptor_runtime: descriptor,
            screen_runtime: screen,
            viewport,
            file_fresh: fresh,
            file_quiet_secs: fresh.map(|is_fresh| if is_fresh { 0 } else { 600 }),
            screen_changed: changed,
            at_prompt: false,
            is_generic: false,
            is_service: false,
        }
    }

    #[test]
    fn strong_working_beats_scrolled_and_historical_content() {
        let cases = [
            evidence(RT::Working, RT::Unknown, VP::Historical, Some(true), true),
            evidence(RT::Unknown, RT::Working, VP::Historical, None, true),
            evidence(RT::Working, RT::NeedsInput, VP::Live, None, false),
        ];
        for ev in cases {
            let out = reduce(&working(Status::Working, 10), &ev, 100);
            assert_eq!(out.attention.status, Status::Working, "ev: {ev:?}");
            assert_eq!(out.phase, Phase::Busy, "ev: {ev:?}");
        }
    }

    #[test]
    fn descriptor_working_title_survives_a_settled_stale_screen() {
        let ev = evidence(RT::Working, RT::Unknown, VP::Unknown, Some(false), false);
        let out = reduce(&working(Status::Working, 10), &ev, 200);
        assert_eq!(
            out.attention.status,
            Status::Working,
            "title evidence must stay working"
        );
        assert_eq!(out.attention.working_since, Some(10));
    }

    #[test]
    fn screen_working_decays_only_when_settled_and_not_fresh() {
        let stale = evidence(RT::Unknown, RT::Working, VP::Unknown, Some(false), false);
        let mut prev = working(Status::Working, 10);
        prev.settled_since = None;
        let out = reduce(&prev, &stale, 100);
        assert_eq!(
            out.attention.status,
            Status::Working,
            "one stable poll is not enough"
        );
        assert_eq!(out.attention.settled_since, Some(100));
        let out = reduce(&out.attention, &stale, 105);
        assert_eq!(
            out.attention.status,
            Status::YourTurn,
            "a settled stale spinner completes after grace"
        );

        let fresh = evidence(RT::Unknown, RT::Working, VP::Unknown, Some(true), false);
        let mut prev = working(Status::Working, 10);
        prev.settled_since = None;
        assert_eq!(
            reduce(&prev, &fresh, 100).attention.status,
            Status::Working,
            "a fresh transcript keeps the spinner live"
        );

        let moving = evidence(RT::Unknown, RT::Working, VP::Unknown, Some(false), true);
        assert_eq!(
            reduce(&working(Status::Working, 10), &moving, 100)
                .attention
                .status,
            Status::Working,
            "a moving spinner is live work"
        );
    }

    #[test]
    fn grace_expiry_completes_a_prior_working_turn_without_live_or_fresh_evidence() {
        let ev = evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, false);
        let mut prev = working(Status::Working, 0);
        prev.settled_since = None;
        let settled = reduce(&prev, &ev, 100);
        assert_eq!(settled.attention.status, Status::Working, "settled at 100");
        assert_eq!(settled.attention.settled_since, Some(100));
        let out = reduce(&settled.attention, &ev, 104);
        assert_eq!(
            out.attention.status,
            Status::Working,
            "grace not yet elapsed"
        );
        let out = reduce(&settled.attention, &ev, 105);
        assert_eq!(out.attention.status, Status::YourTurn, "grace elapsed");
        assert_eq!(out.phase, Phase::Done);
        assert_eq!(out.transition.unwrap().reason, Reason::GraceCompleted);
    }

    #[test]
    fn ready_evidence_completes_only_after_settle_and_grace() {
        let ready = evidence(RT::Unknown, RT::Ready, VP::Unknown, None, false);
        let mut prev = working(Status::Working, 0);
        prev.settled_since = None;
        let settled = reduce(&prev, &ready, 100);
        assert_eq!(
            settled.attention.status,
            Status::Working,
            "settle starts at 100"
        );
        let out = reduce(&settled.attention, &ready, 104);
        assert_eq!(
            out.attention.status,
            Status::Working,
            "ready evidence is not enough before grace"
        );
        let out = reduce(&settled.attention, &ready, 105);
        assert_eq!(out.attention.status, Status::YourTurn);
    }

    #[test]
    fn writes_during_the_settle_stretch_block_completion() {
        let mut ready = evidence(RT::Unknown, RT::Ready, VP::Unknown, Some(true), false);
        ready.file_quiet_secs = Some(2);
        let mut prev = working(Status::Working, 0);
        prev.settled_since = Some(100);
        let out = reduce(&prev, &ready, 110);
        assert_eq!(
            out.attention.status,
            Status::Working,
            "a write landing mid-settle proves quiet work"
        );
        assert_eq!(
            out.attention.settled_since, None,
            "the mid-settle write resets the settle stretch"
        );
    }

    #[test]
    fn a_final_write_before_the_settle_stretch_completes_at_grace() {
        let mut ready = evidence(RT::Unknown, RT::Ready, VP::Unknown, Some(true), false);
        ready.file_quiet_secs = Some(20);
        let mut prev = working(Status::Working, 0);
        prev.settled_since = Some(100);
        let out = reduce(&prev, &ready, 105);
        assert_eq!(
            out.attention.status,
            Status::YourTurn,
            "a still-fresh final write must not delay completion"
        );
    }

    #[test]
    fn strong_needs_input_is_immediate_even_while_moving() {
        let ev = evidence(RT::Unknown, RT::NeedsInput, VP::Live, None, true);
        let out = reduce(&att(Status::Unknown), &ev, 100);
        assert_eq!(out.attention.status, Status::NeedsYou);
        assert_eq!(out.phase, Phase::Blocked);
        assert_eq!(out.transition.unwrap().reason, Reason::StrongNeedsInput);
    }

    #[test]
    fn descriptor_needs_input_is_strong_regardless_of_staleness() {
        let ev = evidence(RT::NeedsInput, RT::Unknown, VP::Unknown, Some(false), false);
        assert_eq!(
            reduce(&att(Status::Unknown), &ev, 100).attention.status,
            Status::NeedsYou
        );
    }

    #[test]
    fn stale_screen_needs_input_holds_instead_of_alerting() {
        let stale = evidence(RT::Unknown, RT::NeedsInput, VP::Live, Some(false), false);
        for prev in [
            Status::Unknown,
            Status::Working,
            Status::NeedsYou,
            Status::YourTurn,
            Status::Acknowledged,
        ] {
            let out = reduce(&att(prev), &stale, 100);
            assert_eq!(
                out.attention.status, prev,
                "historical questionnaire holds {prev:?}"
            );
            assert_eq!(
                out.phase,
                Phase::Hold,
                "kimi historical hold precedes awaiting"
            );
            assert!(out.transition.is_none());
        }
    }

    #[test]
    fn historical_viewport_holds_prior_state_and_never_creates_attention() {
        let ev = evidence(RT::Unknown, RT::Unknown, VP::Historical, None, false);
        for prev in [
            Status::Unknown,
            Status::Working,
            Status::NeedsYou,
            Status::YourTurn,
            Status::Acknowledged,
        ] {
            let out = reduce(&att(prev), &ev, 100);
            assert_eq!(
                out.attention.status, prev,
                "historical viewport holds {prev:?}"
            );
            assert!(
                out.transition.is_none(),
                "historical viewport cannot create attention"
            );
        }
    }

    #[test]
    fn historical_viewport_precedes_screen_needs_input() {
        let ev = evidence(RT::Unknown, RT::NeedsInput, VP::Historical, None, false);
        let out = reduce(&att(Status::Working), &ev, 100);
        assert_eq!(
            out.attention.status,
            Status::Working,
            "hold precedes the awaiting short-circuit"
        );
        assert!(out.transition.is_none());
    }

    #[test]
    fn first_sighting_of_ready_or_idle_never_arms_your_turn() {
        let ready = evidence(RT::Unknown, RT::Ready, VP::Unknown, None, false);
        let out = reduce(&att(Status::Unknown), &ready, 100);
        assert_eq!(
            out.attention.status,
            Status::Unknown,
            "a first sighting never completes"
        );
        assert_eq!(out.phase, Phase::Hold);

        let stale_done = evidence(RT::Unknown, RT::Ready, VP::Unknown, Some(false), false);
        assert_eq!(
            reduce(&att(Status::Unknown), &stale_done, 100)
                .attention
                .status,
            Status::Unknown,
            "stale any-line done markers do not alert without a prior working turn"
        );

        let descriptor_ready = evidence(RT::Ready, RT::Unknown, VP::Unknown, None, false);
        assert_eq!(
            reduce(&att(Status::Unknown), &descriptor_ready, 100)
                .attention
                .status,
            Status::Unknown
        );
    }

    #[test]
    fn completed_turn_requires_prior_working_status() {
        let ready = evidence(RT::Unknown, RT::Ready, VP::Unknown, None, false);
        let mut prev = working(Status::Unknown, 0);
        prev.settled_since = Some(0);
        let out = reduce(&prev, &ready, 100);
        assert_eq!(
            out.attention.status,
            Status::Unknown,
            "unknown never completes"
        );
        let out = reduce(&working(Status::YourTurn, 0), &ready, 100);
        assert_eq!(out.attention.status, Status::YourTurn, "your-turn stays");
        let out = reduce(&working(Status::Acknowledged, 0), &ready, 100);
        assert_eq!(out.attention.status, Status::Acknowledged, "ack stays");
    }

    #[test]
    fn acknowledgement_survives_cosmetic_redraws_and_holds() {
        let redraw = evidence(RT::Unknown, RT::Ready, VP::Unknown, None, true);
        let out = reduce(&working(Status::Acknowledged, 0), &redraw, 100);
        assert_eq!(
            out.attention.status,
            Status::Acknowledged,
            "a redraw must not re-arm"
        );

        let settled = evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, false);
        let out = reduce(&working(Status::Acknowledged, 0), &settled, 200);
        assert_eq!(out.attention.status, Status::Acknowledged);

        let historical = evidence(RT::Unknown, RT::Unknown, VP::Historical, None, false);
        assert_eq!(
            reduce(&working(Status::Acknowledged, 0), &historical, 200)
                .attention
                .status,
            Status::Acknowledged
        );
    }

    #[test]
    fn acknowledgement_breaks_on_new_work_and_strong_input() {
        let work = evidence(RT::Working, RT::Unknown, VP::Unknown, None, false);
        assert_eq!(
            reduce(&working(Status::Acknowledged, 0), &work, 100)
                .attention
                .status,
            Status::Working
        );
        let input = evidence(RT::NeedsInput, RT::Unknown, VP::Unknown, None, false);
        assert_eq!(
            reduce(&working(Status::Acknowledged, 0), &input, 100)
                .attention
                .status,
            Status::NeedsYou
        );
    }

    #[test]
    fn working_episode_start_is_preserved_across_holds() {
        let hold = evidence(RT::Unknown, RT::Unknown, VP::Historical, None, true);
        let out = reduce(&working(Status::Working, 10), &hold, 100);
        assert_eq!(
            out.attention.working_since,
            Some(10),
            "the running timer survives the hold"
        );
    }

    #[test]
    fn scroll_while_working_never_produces_attention_for_any_harness() {
        let harnesses = [
            evidence(RT::Working, RT::Unknown, VP::Unknown, Some(true), true),
            evidence(RT::Unknown, RT::Working, VP::Unknown, None, true),
            evidence(RT::Unknown, RT::Unknown, VP::Unknown, Some(true), true),
            evidence(RT::Unknown, RT::NeedsInput, VP::Unknown, Some(true), true),
            evidence(RT::Unknown, RT::NeedsInput, VP::Unknown, Some(true), false),
            evidence(RT::Unknown, RT::Ready, VP::Unknown, Some(true), false),
        ];
        for ev in harnesses {
            let out = reduce(&working(Status::Working, 10), &ev, 100);
            assert!(
                matches!(out.attention.status, Status::Working),
                "scrolling while working must never alert, got {:?} for ev: {ev:?}",
                out.attention.status
            );
            assert!(out.transition.is_none(), "ev: {ev:?}");
        }
    }

    #[test]
    fn fresh_work_keeps_a_picker_looking_scrollback_from_alerting() {
        let residue = evidence(RT::Unknown, RT::NeedsInput, VP::Unknown, Some(true), false);
        let out = reduce(&working(Status::Working, 10), &residue, 100);
        assert_eq!(
            out.attention.status,
            Status::Working,
            "a fresh transcript proves the picker-looking tail is scrollback residue"
        );
        assert!(out.transition.is_none());
    }

    #[test]
    fn generic_shell_lifecycle() {
        let base = evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, false);
        let running = Evidence {
            at_prompt: false,
            is_generic: true,
            ..base
        };
        let out = reduce(&att(Status::Unknown), &running, 100);
        assert_eq!(
            out.attention.status,
            Status::Working,
            "generic busy by default"
        );
        assert_eq!(out.attention.working_since, Some(100));

        let quick = Evidence {
            at_prompt: true,
            ..base
        };
        let prev = working(Status::Working, 98);
        let out = reduce(&prev, &quick, 100);
        assert_eq!(
            out.attention.status,
            Status::Unknown,
            "a quick command does not flash done"
        );
        assert_eq!(out.phase, Phase::Idle);
        assert_eq!(out.transition.unwrap().reason, Reason::QuickIdle);

        let long = Evidence {
            at_prompt: true,
            ..base
        };
        let prev = working(Status::Working, 90);
        let out = reduce(&prev, &long, 100);
        assert_eq!(
            out.attention.status,
            Status::YourTurn,
            "a long command completes at the prompt"
        );

        let acked = working(Status::Acknowledged, 90);
        assert_eq!(
            reduce(&acked, &quick, 100).attention.status,
            Status::Acknowledged,
            "the prompt must not un-acknowledge"
        );
    }

    #[test]
    fn generic_service_reads_live_and_ends_at_the_prompt() {
        let base = evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, false);
        let service = Evidence {
            is_generic: true,
            is_service: true,
            ..base
        };
        let out = reduce(&att(Status::Unknown), &service, 100);
        assert_eq!(out.attention.status, Status::Service);
        assert_eq!(out.phase, Phase::Service);
        assert_eq!(out.transition.unwrap().reason, Reason::Service);

        let prompt = Evidence {
            at_prompt: true,
            ..base
        };
        let prev = working(Status::Service, 90);
        let out = reduce(&prev, &prompt, 100);
        assert_eq!(
            out.attention.status,
            Status::YourTurn,
            "a long service ends in your turn"
        );

        let quick = working(Status::Service, 98);
        assert_eq!(
            reduce(&quick, &prompt, 100).attention.status,
            Status::Unknown,
            "a brief service returns to unknown"
        );
    }

    #[test]
    fn clock_jump_back_holds_and_resets_timers() {
        let ev = evidence(RT::Unknown, RT::Ready, VP::Unknown, None, false);
        let prev = working(Status::Working, 100);
        let out = reduce(&prev, &ev, 50);
        assert_eq!(
            out.attention.status,
            Status::Working,
            "a backwards clock cannot complete"
        );
        assert_eq!(
            out.attention.working_since, None,
            "timers reset after the jump"
        );
        assert_eq!(out.attention.settled_since, None);
        assert!(out.transition.is_none(), "a hold is not a transition");
        let settled = evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, false);
        let recovered = reduce(&out.attention, &settled, 55);
        assert_eq!(recovered.attention.status, Status::Working);
        assert_eq!(
            recovered.attention.settled_since,
            Some(55),
            "the settle stretch restarts"
        );
        let done = reduce(&recovered.attention, &settled, 60);
        assert_eq!(
            done.attention.status,
            Status::YourTurn,
            "grace counts from the reset"
        );
    }

    #[test]
    fn exact_grace_boundaries() {
        let ev = evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, false);
        let mut prev = working(Status::Working, 0);
        prev.settled_since = Some(100);
        assert_eq!(reduce(&prev, &ev, 104).attention.status, Status::Working);
        assert_eq!(reduce(&prev, &ev, 105).attention.status, Status::YourTurn);
    }
}
