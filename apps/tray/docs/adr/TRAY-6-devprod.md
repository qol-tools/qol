# TRAY-6 5. Dev/prod — the four overlapping notions

- **Status:** Accepted
- **Issue:** #6
- **Date:** 2026-05-03

## Problem

```mermaid
graph TD
    A[1. Cargo feature 'dev' &mdash; compile-time, 132 cfg sites]
    B[2. Build artifact &mdash; target/debug vs target/release]
    C[3. UI unlock flag &mdash; localStorage, set by 7-click or typing 'dev']
    D[4. Runtime mode &mdash; which binary is exec'd]
    A -->|gates| Routes[dev UI routes / dev page / resolve_dev_candidate]
    B -->|where binary lives| Resolver[resolve_dev_candidate looks here]
    C -->|reveals| Button[mode-switch button in cogwheel]
    Button -->|POST /api/mode/switch| D
    D -->|cargo build --features dev| A
    D -->|exec new binary| B
```

| ID | State | Smell |
|----|-------|-------|
| TRAY-6.1 | 1 | compile-time feature |
| TRAY-6.2 | 2 | build profile |
| TRAY-6.3 | 3 | UI unlock |
| TRAY-6.4 | 4 | runtime mode |

## Proposals

### Proposal A — Collapse to two axes: capability + runtime mode `[medium]`

Drop the cycle. Two axes, orthogonal:

The localStorage unlock and the mode-switch endpoint go away. Mode is a config file. make install-dev sets it; tray menu can flip it. No cycle.

```mermaid
graph LR
    Build{Build} -->|make install| Prod[binary: feature off]
    Build -->|make install-dev| Dev[binary: feature on]
    Dev --> Mode{mode config}
    Mode -->|dev| Visible[dev pages visible]
    Mode -->|prod| Hidden[dev pages hidden]
    Prod --> Hidden
```

| Pros | Cons |
|------|------|
| two orthogonal axes. No cycle. No localStorage gating a rebuild. End-user prod binary literally cannot show dev pages (capability not compiled). | removes the "magic 7-click recompile to dev" flow. Devs must run make install-dev manually. Loses a cute trick. |

Implementation notes:

- Capability is compile-time. A prod-feature binary returns `false` from `/api/dev/enabled` regardless of `mode.json`.
- Runtime mode is file-backed. Missing `mode.json` means prod.
- `/api/dev/enabled` remains the unguarded capability probe. In a dev-feature binary, every other `/api/dev/*` route returns 404 while runtime mode is prod.
- The tray menu toggle writes `mode.json`, but the tray label/check state refreshes only after restart.
- An already-open UI sees mode changes after refresh/reopen, not live.
- `make dev` runs `cargo run --features dev` directly and does **not** write `mode.json`. A first-time developer therefore boots with dev pages hidden until they either run `make install-dev` once (writes `mode = dev`) or flip the tray-menu toggle. Subsequent `make dev` cycles inherit whatever value is already in `mode.json`.

**Closes:** TRAY-6.1, TRAY-6.3, TRAY-6.4

---

### Proposal B — Keep the cycle, fence the gates `[cheap]`

Keep all 4 notions but make each one's gate explicit and matched. switch_to_dev becomes cfg(feature = "dev")-only (so prod binaries don't even register the route). UI unlock check first probes whether the endpoint exists; if not, hides the button.

Recommended: A. Collapses 4→2, kills the cycle, costs one config file.

```mermaid
graph TD
    Prod[prod binary] --> NoRoute[/api/mode/switch ABSENT/]
    Dev[dev binary] --> HasRoute[/api/mode/switch present/]
    UI[UI unlock 7-click] --> Probe{HEAD /api/mode/switch}
    Probe -->|404| HideBtn[hide mode-switch button]
    Probe -->|200| ShowBtn[show button]
```

| Pros | Cons |
|------|------|
| tiny diff. Fixes the immediate "prod button does nothing" smell. | cycle remains. Future maintainer still has to reason about 4 notions. Doesn't solve the architectural confusion you started this session for. |

**Closes:** TRAY-6.4
