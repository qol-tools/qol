use plugin_cli_sessions::attention::{reduce, Attention, Evidence, Phase, Reason, GRACE_SECS};
use plugin_cli_sessions::status::Status;
use proptest::prelude::*;
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

const EVERY_STATUS: [Status; 6] = [
    Status::Working,
    Status::Service,
    Status::YourTurn,
    Status::NeedsYou,
    Status::Unknown,
    Status::Acknowledged,
];

#[test]
fn scroll_while_working_never_creates_attention_for_every_harness() {
    let harnesses = [
        (
            "codex",
            evidence(RT::Working, RT::Unknown, VP::Unknown, Some(true), false),
        ),
        (
            "claude",
            evidence(RT::Unknown, RT::Working, VP::Unknown, Some(true), false),
        ),
        (
            "pi",
            evidence(RT::Unknown, RT::Working, VP::Unknown, Some(true), true),
        ),
        (
            "kimi",
            evidence(RT::Unknown, RT::Working, VP::Unknown, Some(true), false),
        ),
        (
            "generic",
            Evidence {
                is_generic: true,
                ..evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, false)
            },
        ),
    ];
    for (name, ev) in harnesses {
        let prev = working(Status::Working, 10);
        let out = reduce(&prev, &ev, 100);
        assert_eq!(
            out.attention.status,
            Status::Working,
            "{name}: scrolling while working must hold"
        );
        assert!(
            out.transition.is_none(),
            "{name}: no transition while scrolling"
        );
        assert_eq!(
            out.attention.working_since,
            Some(10),
            "{name}: the run timer survives"
        );
    }
}

#[test]
fn one_stable_poll_is_insufficient_for_completion() {
    let ev = evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, false);
    let mut prev = working(Status::Working, 0);
    prev.settled_since = None;
    let first = reduce(&prev, &ev, 100);
    assert_eq!(
        first.attention.status,
        Status::Working,
        "the first settled poll holds"
    );
    assert_eq!(first.attention.settled_since, Some(100));
    let second = reduce(&first.attention, &ev, 100);
    assert_eq!(
        second.attention.status,
        Status::Working,
        "a second poll at the same instant holds"
    );
    let at_grace = reduce(&first.attention, &ev, 100 + GRACE_SECS);
    assert_eq!(
        at_grace.attention.status,
        Status::YourTurn,
        "grace expiry completes"
    );
}

#[test]
fn grace_requires_no_live_evidence_and_no_fresh_activity() {
    let mut prev = working(Status::Working, 0);
    prev.settled_since = None;

    let fresh = evidence(RT::Unknown, RT::Unknown, VP::Unknown, Some(true), false);
    let out = reduce(&prev, &fresh, 100 + GRACE_SECS);
    assert_eq!(
        out.attention.status,
        Status::Working,
        "fresh activity blocks completion"
    );

    let working_ev = evidence(RT::Working, RT::Unknown, VP::Unknown, None, false);
    let out = reduce(&prev, &working_ev, 100 + GRACE_SECS);
    assert_eq!(
        out.attention.status,
        Status::Working,
        "live work blocks completion"
    );

    let moving = evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, true);
    let out = reduce(&prev, &moving, 100 + GRACE_SECS);
    assert_eq!(
        out.attention.status,
        Status::Working,
        "movement blocks completion"
    );
}

#[test]
fn live_evidence_overrides_historical_looking_content() {
    let mut prev = working(Status::Working, 0);
    prev.settled_since = None;
    let ev = evidence(RT::Working, RT::Unknown, VP::Historical, Some(false), false);
    let out = reduce(&prev, &ev, 100);
    assert_eq!(
        out.attention.status,
        Status::Working,
        "codex title evidence beats the scrolled viewport"
    );
    assert_eq!(out.phase, Phase::Busy);
}

#[test]
fn stale_any_line_done_markers_do_not_alert() {
    let stale_done = evidence(RT::Unknown, RT::Ready, VP::Unknown, Some(false), false);
    for prev in [Status::Unknown, Status::YourTurn, Status::Acknowledged] {
        let out = reduce(&att(prev), &stale_done, 100);
        assert_eq!(
            out.attention.status, prev,
            "stale done evidence holds {prev:?}"
        );
        assert!(out.transition.is_none());
    }
}

#[test]
fn claude_done_summary_in_scrollback_is_not_live_while_working() {
    let mut prev = working(Status::Working, 0);
    prev.settled_since = None;
    let ev = evidence(RT::Unknown, RT::Ready, VP::Unknown, Some(true), false);
    let out = reduce(&prev, &ev, 100);
    assert_eq!(
        out.attention.status,
        Status::Working,
        "a done summary in the tail with a fresh transcript is scrollback, not a finished turn"
    );
    assert!(out.transition.is_none());
}

#[test]
fn kimi_historical_hold_precedes_awaiting_short_circuit() {
    let stale_picker = evidence(RT::Unknown, RT::NeedsInput, VP::Live, Some(false), false);
    for prev in EVERY_STATUS {
        let out = reduce(&att(prev), &stale_picker, 100);
        assert_eq!(
            out.attention.status, prev,
            "a stale questionnaire holds {prev:?}"
        );
        assert_eq!(
            out.phase,
            Phase::Hold,
            "hold is decided before awaiting for {prev:?}"
        );
        assert!(
            out.transition.is_none(),
            "no alert from history for {prev:?}"
        );
    }
    let live_picker = evidence(RT::Unknown, RT::NeedsInput, VP::Live, None, false);
    assert_eq!(
        reduce(&att(Status::Unknown), &live_picker, 100)
            .attention
            .status,
        Status::NeedsYou,
        "a fresh questionnaire alerts"
    );
}

#[test]
fn strong_needs_input_becomes_needs_you_immediately() {
    let input = evidence(RT::NeedsInput, RT::Unknown, VP::Unknown, None, true);
    let out = reduce(&att(Status::Unknown), &input, 100);
    assert_eq!(out.attention.status, Status::NeedsYou);
    assert_eq!(out.phase, Phase::Blocked);
    assert_eq!(out.transition.unwrap().reason, Reason::StrongNeedsInput);
}

#[test]
fn stale_needs_you_clears_after_stable_idle_evidence() {
    let idle = evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, false);
    let first = reduce(&att(Status::NeedsYou), &idle, 100);
    assert_eq!(first.attention.status, Status::NeedsYou);
    assert_eq!(first.attention.settled_since, Some(100));
    assert_eq!(
        reduce(&first.attention, &idle, 100 + GRACE_SECS - 1)
            .attention
            .status,
        Status::NeedsYou
    );

    let cleared = reduce(&first.attention, &idle, 100 + GRACE_SECS);
    assert_eq!(cleared.attention.status, Status::Unknown);
    assert_eq!(cleared.phase, Phase::Idle);
    assert_eq!(cleared.transition.unwrap().reason, Reason::QuickIdle);
}

#[test]
fn acknowledgement_survives_cosmetic_redraw_and_stale_markers() {
    let redraw = evidence(RT::Unknown, RT::Ready, VP::Unknown, None, true);
    let out = reduce(&working(Status::Acknowledged, 0), &redraw, 100);
    assert_eq!(
        out.attention.status,
        Status::Acknowledged,
        "cosmetic redraw must not re-arm"
    );
    assert!(out.transition.is_none());

    let settled = evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, false);
    let mut prev = working(Status::Acknowledged, 0);
    prev.settled_since = None;
    let out = reduce(&prev, &settled, 200 + GRACE_SECS);
    assert_eq!(
        out.attention.status,
        Status::Acknowledged,
        "grace expiry cannot re-arm an ack"
    );
}

#[test]
fn working_to_your_turn_is_the_only_completion_path() {
    let ready = evidence(RT::Unknown, RT::Ready, VP::Unknown, None, false);
    let out = reduce(&working(Status::Working, 0), &ready, 100 + GRACE_SECS);
    assert_eq!(out.attention.status, Status::YourTurn);
    assert_eq!(out.transition.unwrap().reason, Reason::GraceCompleted);

    for from in [
        Status::Service,
        Status::Unknown,
        Status::YourTurn,
        Status::Acknowledged,
    ] {
        let out = reduce(&working(from, 0), &ready, 100 + GRACE_SECS);
        assert_eq!(
            out.attention.status, from,
            "completion only fires from Working ({from:?})"
        );
    }
    assert_eq!(
        reduce(&working(Status::NeedsYou, 0), &ready, 100 + GRACE_SECS)
            .attention
            .status,
        Status::Unknown,
        "a stale needs-you state clears instead of becoming your turn"
    );
}

#[test]
fn codex_working_title_stays_working_while_scrolled() {
    let mut prev = working(Status::Working, 0);
    prev.settled_since = None;
    let scrolled = evidence(RT::Working, RT::Unknown, VP::Unknown, Some(false), false);
    let out = reduce(&prev, &scrolled, 100 + GRACE_SECS);
    assert_eq!(
        out.attention.status,
        Status::Working,
        "descriptor Working title evidence outlives the settle grace"
    );
}

#[test]
fn pi_stale_loader_line_settles_to_your_turn_after_grace() {
    let stale_loader = evidence(RT::Unknown, RT::Working, VP::Unknown, Some(false), false);
    let mut prev = working(Status::Working, 0);
    prev.settled_since = None;
    let first = reduce(&prev, &stale_loader, 100);
    assert_eq!(
        first.attention.status,
        Status::Working,
        "first settled sighting is conservative"
    );
    let out = reduce(&first.attention, &stale_loader, 100 + GRACE_SECS);
    assert_eq!(
        out.attention.status,
        Status::YourTurn,
        "a settled stale loader line is your turn"
    );
}

#[test]
fn first_observation_of_a_moving_session_never_arms_your_turn() {
    let moving = evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, true);
    let out = reduce(&att(Status::Unknown), &moving, 100);
    assert_eq!(
        out.attention.status,
        Status::Unknown,
        "a first sighting with no evidence fails safe"
    );
    let out = reduce(&out.attention, &moving, 100 + GRACE_SECS);
    assert_eq!(
        out.attention.status,
        Status::Unknown,
        "unknown evidence never completes"
    );
}

#[test]
fn generic_shell_flow_preserves_quick_command_and_long_command_semantics() {
    let base = evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, false);
    let running = Evidence {
        is_generic: true,
        ..base
    };
    let out = reduce(&att(Status::Unknown), &running, 100);
    assert_eq!(out.attention.status, Status::Working);

    let prompt = Evidence {
        at_prompt: true,
        ..base
    };
    let quick = reduce(&working(Status::Working, 98), &prompt, 100);
    assert_eq!(
        quick.attention.status,
        Status::Unknown,
        "a quick command does not flash done"
    );
    assert_eq!(quick.phase, Phase::Idle);
    assert_eq!(quick.transition.unwrap().reason, Reason::QuickIdle);

    let long = reduce(&working(Status::Working, 90), &prompt, 100);
    assert_eq!(
        long.attention.status,
        Status::YourTurn,
        "a long command is your turn at the prompt"
    );
    assert_eq!(long.transition.unwrap().reason, Reason::GraceCompleted);
}

#[test]
fn generic_service_reads_live_then_returns_at_prompt() {
    let base = evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, false);
    let service = Evidence {
        is_generic: true,
        is_service: true,
        ..base
    };
    let out = reduce(&att(Status::Unknown), &service, 100);
    assert_eq!(out.attention.status, Status::Service);
    assert_eq!(out.phase, Phase::Service);

    let prompt = Evidence {
        at_prompt: true,
        ..base
    };
    let ended = reduce(&working(Status::Service, 90), &prompt, 100);
    assert_eq!(
        ended.attention.status,
        Status::YourTurn,
        "a long service ends in your turn"
    );
    let brief = reduce(&working(Status::Service, 98), &prompt, 100);
    assert_eq!(
        brief.attention.status,
        Status::Unknown,
        "a brief service returns to unknown"
    );
}

#[test]
fn monotonic_clock_boundaries_hold_and_recover() {
    let ev = evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, false);
    let prev = working(Status::Working, 200);
    let jumped = reduce(&prev, &ev, 100);
    assert_eq!(
        jumped.attention.status,
        Status::Working,
        "a backwards clock cannot complete"
    );
    assert_eq!(jumped.attention.working_since, None, "timers reset");
    assert_eq!(jumped.attention.settled_since, None);
    assert!(
        jumped.transition.is_none(),
        "a clock hold is not a transition"
    );

    let same = reduce(&working(Status::Working, 100), &ev, 100);
    assert_eq!(
        same.attention.status,
        Status::Working,
        "now == timer start is valid"
    );

    let recovered = reduce(&jumped.attention, &ev, 105);
    assert_eq!(recovered.attention.settled_since, Some(105));
    let done = reduce(&recovered.attention, &ev, 110);
    assert_eq!(
        done.attention.status,
        Status::YourTurn,
        "grace counts from the reset"
    );
}

#[test]
fn exhausted_evidence_matrix_never_creates_attention_without_strong_evidence_or_grace() {
    let descriptor_states = [RT::Unknown, RT::Working, RT::Ready, RT::NeedsInput];
    let screen_states = [RT::Unknown, RT::Working, RT::Ready, RT::NeedsInput];
    let viewports = [VP::Unknown, VP::Live, VP::Historical];
    let fresh_values = [None, Some(true), Some(false)];
    let mut checked = 0;
    for descriptor in descriptor_states {
        for screen in screen_states {
            for viewport in viewports {
                for fresh in fresh_values {
                    let ev = evidence(descriptor, screen, viewport, fresh, false);
                    for prev in EVERY_STATUS {
                        let out = reduce(&att(prev), &ev, 100);
                        let strong_work = descriptor == RT::Working
                            || (screen == RT::Working && fresh != Some(false));
                        let strong_input = descriptor == RT::NeedsInput
                            || (screen == RT::NeedsInput && fresh != Some(false));
                        if out.attention.status != prev && out.attention.status.is_attention() {
                            let allowed = strong_work
                                || strong_input
                                || (prev == Status::Working
                                    && out.attention.status == Status::YourTurn);
                            assert!(allowed, "unexpected attention from {ev:?} on {prev:?}");
                        }
                        if prev == Status::Acknowledged
                            && !strong_work
                            && !strong_input
                            && out.attention.status != Status::Acknowledged
                        {
                            panic!("ack re-armed by {ev:?}");
                        }
                        checked += 1;
                    }
                }
            }
        }
    }
    assert_eq!(checked, 4 * 4 * 3 * 3 * 6, "matrix fully enumerated");
}

fn runtime_strategy() -> impl Strategy<Value = RT> {
    prop_oneof![
        Just(RT::Unknown),
        Just(RT::Working),
        Just(RT::Ready),
        Just(RT::NeedsInput),
    ]
}

fn viewport_strategy() -> impl Strategy<Value = VP> {
    prop_oneof![Just(VP::Unknown), Just(VP::Live), Just(VP::Historical)]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn prop_attention_requires_strong_evidence_or_completed_grace(
        descriptor in runtime_strategy(),
        screen in runtime_strategy(),
        viewport in viewport_strategy(),
        fresh in any::<Option<bool>>(),
        moved in any::<bool>(),
        started_at in 0u64..10_000,
        now in 0u64..10_000,
    ) {
        let prev = working(Status::Working, started_at.min(now));
        let ev = evidence(descriptor, screen, viewport, fresh, moved);
        let out = reduce(&prev, &ev, now);
        let writing = fresh == Some(true)
            && prev
                .settled_since
                .is_some_and(|start| now.saturating_sub(start) > 0);
        let strong_work = descriptor == RT::Working
            || (screen == RT::Working && fresh != Some(false));
        let strong_input = descriptor == RT::NeedsInput
            || (screen == RT::NeedsInput && fresh != Some(false) && !writing);
        let grace_ok = prev
            .settled_since
            .is_some_and(|start| now.saturating_sub(start) >= GRACE_SECS);
        let decayed_completion = out.attention.status == Status::YourTurn
            && grace_ok
            && !moved
            && (!writing || descriptor == RT::Ready)
            && viewport != VP::Historical;
        if out.attention.status.is_attention() {
            prop_assert!(
                strong_work || strong_input || decayed_completion,
                "attention from {ev:?} at now={now} lacks strong evidence or grace"
            );
        }
    }

    #[test]
    fn prop_ack_is_sticky_until_strong_work_or_input(
        descriptor in runtime_strategy(),
        screen in runtime_strategy(),
        fresh in any::<Option<bool>>(),
        moved in any::<bool>(),
        now in 0u64..10_000,
    ) {
        let prev = working(Status::Acknowledged, 0);
        let ev = evidence(descriptor, screen, VP::Unknown, fresh, moved);
        let out = reduce(&prev, &ev, now);
        let strong_work = descriptor == RT::Working
            || (screen == RT::Working && (moved || fresh == Some(true)));
        let strong_input = descriptor == RT::NeedsInput
            || (screen == RT::NeedsInput && fresh != Some(false));
        if !strong_work && !strong_input {
            prop_assert_eq!(out.attention.status, Status::Acknowledged, "ack must survive");
        }
    }

    #[test]
    fn prop_clock_never_moves_backwards(
        seed in 0u64..10_000,
        now in 0u64..10_000,
        descriptor in runtime_strategy(),
        fresh in any::<Option<bool>>(),
    ) {
        let prev = working(Status::Working, seed);
        let ev = evidence(descriptor, RT::Unknown, VP::Unknown, fresh, false);
        let out = reduce(&prev, &ev, now);
        if now < seed {
            prop_assert_eq!(out.attention.working_since, None, "timers reset on a backwards clock");
            prop_assert_eq!(out.attention.status, Status::Working, "a backwards clock holds");
        }
    }
}

#[test]
fn authoritative_ready_completes_despite_fresh_activity() {
    let ready = evidence(RT::Ready, RT::Unknown, VP::Unknown, Some(true), false);
    let mut prev = working(Status::Working, 0);
    prev.settled_since = None;
    let settled = reduce(&prev, &ready, 100);
    assert_eq!(
        settled.attention.status,
        Status::Working,
        "settle starts at the first stable frame"
    );
    assert_eq!(
        settled.attention.settled_since,
        Some(100),
        "fresh transcript writes must not block the settle for an authoritative ready"
    );
    assert_eq!(
        reduce(&settled.attention, &ready, 104).attention.status,
        Status::Working,
        "grace not yet elapsed"
    );
    assert_eq!(
        reduce(&settled.attention, &ready, 105).attention.status,
        Status::YourTurn,
        "a codex ready title completes after settle and grace even while file writes stay fresh"
    );
}

#[test]
fn authoritative_ready_beats_a_settled_fresh_screen_spinner() {
    let ready = evidence(RT::Ready, RT::Working, VP::Unknown, Some(true), false);
    let mut prev = working(Status::Working, 0);
    prev.settled_since = None;
    let settled = reduce(&prev, &ready, 100);
    assert_eq!(settled.attention.status, Status::Working);
    assert_eq!(
        reduce(&settled.attention, &ready, 105).attention.status,
        Status::YourTurn,
        "the ready title is the harness state; weak freshness does not override it"
    );
}

#[test]
fn authoritative_ready_still_waits_for_the_screen_to_settle() {
    let moving = evidence(RT::Ready, RT::Working, VP::Unknown, Some(true), true);
    let out = reduce(&working(Status::Working, 0), &moving, 100);
    assert_eq!(
        out.attention.status,
        Status::Working,
        "even a ready title waits while the screen moves"
    );
    assert!(out.transition.is_none());
}

#[test]
fn missing_metadata_picker_from_working_requires_time_confirmation() {
    let mut prev = working(Status::Working, 0);
    prev.settled_since = None;

    let moving = evidence(RT::Unknown, RT::NeedsInput, VP::Unknown, None, true);
    let out = reduce(&prev, &moving, 100);
    assert_eq!(
        out.attention.status,
        Status::Working,
        "a picker-looking frame right after work, with unknown freshness, holds"
    );
    assert!(out.transition.is_none(), "no immediate alert");
    assert_eq!(
        out.attention.settled_since, None,
        "a moving viewport cannot confirm anything"
    );
    assert_eq!(
        reduce(&working(Status::Working, 0), &moving, 110)
            .attention
            .status,
        Status::Working,
        "sustained movement is indistinguishable without evidence and never alerts"
    );

    let settled = evidence(RT::Unknown, RT::NeedsInput, VP::Unknown, None, false);
    let mut prev = working(Status::Working, 0);
    prev.settled_since = None;
    let out = reduce(&prev, &settled, 100);
    assert_eq!(
        out.attention.status,
        Status::Working,
        "one stable frame is not enough"
    );
    assert_eq!(out.attention.settled_since, Some(100));
    assert_eq!(
        reduce(&out.attention, &settled, 104).attention.status,
        Status::Working,
        "confirmation needs the full grace"
    );
    let confirmed = reduce(&out.attention, &settled, 105);
    assert_eq!(
        confirmed.attention.status,
        Status::NeedsYou,
        "a settled picker confirmed by the grace alerts"
    );
    assert_eq!(
        confirmed.transition.unwrap().reason,
        Reason::StrongNeedsInput
    );
}

#[test]
fn descriptor_needs_input_stays_immediate_from_working() {
    let moving = evidence(RT::NeedsInput, RT::Unknown, VP::Unknown, None, true);
    let out = reduce(&working(Status::Working, 0), &moving, 100);
    assert_eq!(
        out.attention.status,
        Status::NeedsYou,
        "descriptor needs-input is strong and needs no confirmation"
    );
    assert_eq!(out.transition.unwrap().reason, Reason::StrongNeedsInput);
}

#[test]
fn supported_harnesses_require_completion_and_live_work_overrides_old_ready_records() {
    use plugin_cli_sessions::attention::reduce_with_policy;
    use plugin_cli_sessions::tool::completion_policy;
    use qol_terminal_sessions::cli::{claude_tool, codex_tool, pi_tool};
    for tool in [claude_tool(), codex_tool(), pi_tool()] {
        let policy = completion_policy(&tool);
        for descriptor in [RT::Ready, RT::Unknown] {
            for fresh in [Some(true), Some(false), None] {
                let ev = evidence(descriptor, RT::Working, VP::Live, fresh, false);
                for status in EVERY_STATUS {
                    let mut previous = working(status, 0);
                    for now in [10, 16, 120, 1200] {
                        let out = reduce_with_policy(&previous, &ev, now, policy);
                        assert_eq!(out.attention.status, Status::Working, "{} {ev:?}", tool.id);
                        previous = out.attention;
                    }
                }
            }
        }
        let unknown = evidence(RT::Unknown, RT::Unknown, VP::Unknown, None, false);
        let previous = working(Status::Working, 0);
        assert_eq!(
            reduce_with_policy(&previous, &unknown, 1200, policy)
                .attention
                .status,
            Status::Working
        );
        let ready = evidence(RT::Ready, RT::Unknown, VP::Unknown, Some(false), false);
        assert_eq!(
            reduce_with_policy(&previous, &ready, 1200, policy)
                .attention
                .status,
            Status::YourTurn
        );
    }
}
