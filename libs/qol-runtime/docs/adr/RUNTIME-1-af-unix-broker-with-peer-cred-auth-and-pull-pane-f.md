# RUNTIME-1 Af-unix Broker With Peer-cred Auth And Pull Pane-fields Api

- **Status:** Proposed
- **Issue:** #1
- **Date:** 2026-05-12
- **Related:** none yet

## Problem

The terminal-workspace-restore design (see `docs/superpowers/specs/2026-05-12-terminal-workspace-restore-design.md`) treats inter-plugin RPC as HTTP-over-loopback and pushes the full `PaneSnapshot` to every restore-rule plugin. Both decisions widen the trust surface unnecessarily: any same-uid local process can hit a plugin's loopback port, and every restore-rule plugin sees `cwd`, `title`, and full foreground `argv` regardless of what it actually needs. qol-runtime is the broker between plugin-kitty (the snapshot producer) and N restore-rule plugins (the consumers), so the structural fix lands here.

```mermaid
sequenceDiagram
    autonumber
    participant K as plugin-kitty
    participant R as qol-runtime broker (today)
    participant P1 as plugin-claude-sessions
    participant P2 as other restore-rule plugin
    participant X as same-uid attacker
    K->>R: snapshot (Vec<PaneSnapshot>)
    R->>P1: POST 127.0.0.1:portA/restore-rule<br/>full PaneSnapshot
    R->>P2: POST 127.0.0.1:portB/restore-rule<br/>full PaneSnapshot
    X->>P1: curl 127.0.0.1:portA/restore-rule<br/>(same uid, no peer check)
    P1-->>X: 200 OK (accepts forged request)
    Note over P1,P2: every plugin sees cwd + title + argv,<br/>regardless of declared need
    classDef bad fill:#f5c2c7,stroke:#842029,color:#000
    classDef warn fill:#ffeeba,stroke:#856404,color:#000
    class X,P1 bad
    class P2 warn
```

| ID | State | Smell |
|----|-------|-------|
| RUNTIME-1.1 | 🔴 Broken | HTTP-over-loopback gives any same-uid process unauthenticated access to every plugin's `/restore-rule` endpoint. |
| RUNTIME-1.2 | 🔴 Broken | Broker pushes the full `PaneSnapshot` (cwd, title, argv) to every restore-rule plugin, regardless of what the plugin actually needs. |
| RUNTIME-1.3 | 🟡 Leaky | No capability declaration for which pane fields a plugin may read, so install-time consent cannot be field-scoped. |
| RUNTIME-1.4 | 🟡 Leaky | Bearer tokens alone (without peer identity) cannot distinguish a real plugin daemon from a same-uid process that scraped the token from a log or env dump. |

> Severity: 🔴 bad (broken / silent failure / data loss), 🟡 warn (leaky / race / brittle), 🟢 good (used in proposal diagrams to mark what is now safe)

## Proposals

### Proposal A, AF_UNIX broker with peer-credential auth and pull-based pane-fields API `[heavy]`

Replace HTTP-over-loopback with AF_UNIX sockets under qol-runtime's runtime dir (mode `0600`, owned by the daemon uid). On `accept()`, the broker reads peer credentials via `SO_PEERCRED` (Linux) or `LOCAL_PEERCRED` (macOS) and rejects any peer whose pid is not a plugin daemon currently supervised by qol-tray. The broker stops pushing `PaneSnapshot`; it sends an opaque handle `{ pane_id, opaque_token }` and exposes a pull API on the same socket. Plugins query specific fields via `GET /panes/<id>?fields=<field>` and the broker enforces a per-plugin capability allowlist declared in `plugin.toml`:

```toml
[capabilities.restore-rule]
templates = ["claude-session"]
pane-fields = ["foreground.exe", "foreground.pid", "foreground.cwd"]
```

Bearer tokens (32 random bytes, 5 s TTL, single-use) remain as defense in depth for replay resistance within an already-authenticated peer.

```mermaid
sequenceDiagram
    autonumber
    participant K as plugin-kitty
    participant R as qol-runtime broker (proposed)
    participant P1 as plugin-claude-sessions
    participant P2 as other restore-rule plugin
    participant X as same-uid attacker
    K->>R: snapshot (Vec<PaneSnapshot>)
    R->>P1: AF_UNIX connect + handle<br/>{pane_id, opaque_token}
    P1->>R: GET /panes/id?fields=foreground.exe
    R-->>P1: "claude"
    P1->>R: GET /panes/id?fields=foreground.cwd
    R-->>P1: /home/u/proj (capability check ok)
    P1->>R: GET /panes/id?fields=title
    R-->>P1: 403 (capability not declared)
    X->>R: AF_UNIX connect (same uid)
    R-->>X: close (SO_PEERCRED rejects unsupervised pid)
    classDef good fill:#cfe8d6,stroke:#0f5132,color:#000
    class R,P1 good
```

| Pros | Cons |
|------|------|
| Eliminates "any local process can hit this endpoint" structurally rather than via bearer-token discipline. | Adds platform-specific peer-cred code paths (Linux `SO_PEERCRED`, macOS `LOCAL_PEERCRED` returning `xucred`). |
| Pull-not-push collapses the leak surface from "every plugin sees everything" to "each plugin sees only what its manifest declared and the user approved." | Requires a manifest schema change (`pane-fields` array) and install-time UI to surface field requests. |
| Capability-gated field access enables per-field policies (e.g. workspace-level title redaction) without rewriting the producer. | More moving parts than HTTP-over-loopback: socket lifecycle, peer-cred lookup, capability check, opaque handle bookkeeping. |
| AF_UNIX socket permissions (`0600` + parent dir `0700`) compose with peer-cred to give two independent rejection layers. | Windows support is deferred (no peer-cred equivalent without named-pipe SID inspection). |

**Closes:** RUNTIME-1.1, RUNTIME-1.2, RUNTIME-1.3, RUNTIME-1.4

---

### Proposal B, keep HTTP-over-loopback, add stricter bearer tokens only `[cheap]`

Leave the loopback transport in place. Tighten bearer tokens to 32-byte single-use with 1 s TTL and rotate per call. No transport change, no pane-fields API, no capability schema change.

```mermaid
sequenceDiagram
    participant R as qol-runtime broker
    participant P as plugin
    participant X as same-uid attacker
    R->>P: POST /restore-rule + token T1
    X->>P: replay token T1 inside 1 s
    P-->>X: 200 (token still valid; no peer check)
    classDef bad fill:#f5c2c7,stroke:#842029,color:#000
    class X bad
```

| Pros | Cons |
|------|------|
| Smallest diff against current state. | Does nothing structural: any same-uid process that reads the token (logs, env, ptrace) gets the same access. |
| No platform-specific code. | Leaves `PaneSnapshot` push semantics in place, so the disclosure smell (RUNTIME-1.2) is untouched. |

**Closes:** RUNTIME-1.4

---

**Recommended:** A. Proposal B fails the threat model laid out in the source spec (sections 1 and 8): bearer tokens alone cannot bind a request to a specific supervised peer, and push semantics leak pane state to every plugin regardless of need. The cost difference is real but worth paying once, because the same broker is the integration point for every future capability-gated field.

## Notes

- Source spec sections: §1 Cross-plugin IPC authentication (lines 465 to 475 in `docs/superpowers/specs/2026-05-12-terminal-workspace-restore-design.md`), §8 Information disclosure via PaneSnapshot (lines 575 to 602), and the data-contracts block (lines 101 to 191).
- Primitives: `tokio::net::UnixListener`; `libc::getsockopt(SO_PEERCRED)` on Linux; `libc::getsockopt(..., LOCAL_PEERCRED, ...)` returning `xucred` on macOS.
- The pull API runs over the same socket as the broker; plugins reuse the authenticated connection, no second handshake.
- Bearer-token TTL of 5 s (from the spec) is kept for replay resistance after peer-cred verification.
- Manifest field `pane-fields` is additive; plugins without it default-deny all gated fields and may still receive `foreground.exe` and `foreground.pid` (always-allowed in the spec).
