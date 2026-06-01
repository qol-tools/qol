//! Pin the resolver contract: PID + exe -> session jsonl path.
//!
//! The resolver is the cross-platform face on top of libproc (macOS) and
//! the deferred Linux implementation. Structural tests pin the typed
//! error variants and the early-exit cases that don't require an actual
//! running Claude process:
//!
//! - exe != "claude" -> `Err(ResolveError::NotClaude { .. })`. We refuse
//!   to even probe the fd table for non-Claude processes; this is the
//!   first line of defense against PID spoofing.
//! - Linux / Windows -> `Err(ResolveError::PlatformUnsupported)`. macOS
//!   is the only supported host today; future work tracks the others.
//!
//! Concerns that need a running Claude (live fd walking, jsonl regex
//! match) live under `tests/macos_resolver_live.rs` and are gated to
//! manual runs; they are not part of CI.
//!
//! Closes: CSESS-1.1 (resolver contract invariant).

use plugin_claude_sessions::resolver::{resolve_session_jsonl, ResolveError};

#[test]
fn resolver_rejects_non_claude_exe() {
    // Even with a valid PID, a process whose exe is not `claude` must
    // produce `NotClaude` without any fd probing. The caller surface
    // names the seen exe so logs say what was rejected and why.
    let res = resolve_session_jsonl(1, "bash");
    match res {
        Err(ResolveError::NotClaude { seen }) => assert_eq!(seen, "bash"),
        other => panic!(
            "expected NotClaude {{ seen: \"bash\" }}, got {other:?}; \
             the resolver must short-circuit on the foreground.exe \
             check before any libproc syscall"
        ),
    }
}

#[test]
fn resolver_not_claude_carries_seen_exe_verbatim() {
    let res = resolve_session_jsonl(1, "fake-claude");
    match res {
        Err(ResolveError::NotClaude { seen }) => assert_eq!(seen, "fake-claude"),
        other => panic!("expected NotClaude with seen=fake-claude, got {other:?}"),
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn resolver_platform_unsupported_on_linux_and_windows() {
    // On non-macOS hosts the resolver compiles (per qol-arch-code's
    // no-`compile_error!` rule) and returns a typed error at runtime so
    // the broker can decide UX without panicking.
    let res = resolve_session_jsonl(1, "claude");
    assert_eq!(res, Err(ResolveError::PlatformUnsupported));
}

#[test]
fn resolve_error_variants_are_distinct() {
    // PartialEq derive must hold; broker code branches on variant
    // identity, not on the Display string.
    let a = ResolveError::NotClaude {
        seen: "x".to_string(),
    };
    let b = ResolveError::NotClaude {
        seen: "x".to_string(),
    };
    let c = ResolveError::PidDead(42);
    assert_eq!(a, b);
    assert_ne!(a, c);
}
