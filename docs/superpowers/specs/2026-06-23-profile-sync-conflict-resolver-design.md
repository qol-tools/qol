# Profile Sync Conflict Resolver — Design

Date: 2026-06-23
Status: Approved design, pending implementation plan
Area: `apps/qol-tray` Profile / sync feature

## Problem

Profile sync stores the user's settings in a git repo and syncs by push/pull.
When two machines both change settings since their last shared commit, the
histories diverge. Today this is an unrecoverable dead-end from the UI:

- `GitRepo::pull` (`src/features/profile/sync/git_repo.rs`) is **fast-forward
  only**. On divergence it returns `PullOutcome::Diverged` and does nothing —
  no merge, no rebase, no backup.
- `do_pull` (`src/features/profile/sync/service.rs`) turns that into a
  `SyncIncident` and re-renders "Local <x> differs from remote <y>". Clicking
  **Pull Now** again is a permanent no-op while diverged.
- **Push Now** fails with libgit2 `NotFastForward (-11)`.
- **Acknowledge** only clears the incident banner (`incident = None`). Because
  `auto_push_if_dirty` is gated on "no incident pending", acknowledging
  silently re-arms auto-push and the divergence widens.
- `Disconnect` (nukes `.git`) and a manual `git reset --hard` are the only real
  escapes, both outside the product.

Observed in the field: a profile repo `ahead 9, behind 70`, stuck in
"Review required" with no in-UI path forward.

## Goals

1. Divergence is always recoverable from the UI, with zero terminal use.
2. The user is never asked to pick a whole "side". Resolution is per setting,
   and only for settings that genuinely clash.
3. Every resolution is non-destructive: both sides are backed up before any
   write, so any choice is reversible from the existing Backups surface.
4. The act of choosing is unmistakably clear — the user can see exactly what
   each option keeps and discards.
5. Divergence becomes rare and small, not a 70-commit cliff.

## Non-goals

- No automatic "newest wins" / CRDT tiebreak in v1. Genuine clashes are always
  surfaced to the user. (Revisit later if per-field timestamps prove reliable.)
- No change to the storage trichotomy (core/os/device) or the promote allowlist.
- No general-purpose 3-way merge UI for arbitrary files — scoped to the synced
  profile document.

## Architecture

Three pieces:

1. **Merge engine** (new, backend) — a pure, well-tested function that takes
   three profile snapshots (merge-base, local, remote) and produces a merged
   profile plus a list of unresolved conflicts. No I/O.
2. **Reconcile flow** (backend) — wires the engine into the sync service: on
   divergence, compute conflicts; if none, auto-apply and finish; if some,
   record them and expose them to the UI; on user resolution, apply and push.
3. **Resolver dive** (frontend) — a world-canvas dive target off the Profile
   sync section that walks the user through conflicts one field at a time and
   commits their choices.

### Data flow

```
Pull (manual or launch)
  └─ fetch; if fast-forward → apply as today
     else compute merge-base (git2 merge_base of local, remote)
       └─ load 3 snapshots of the synced profile document
          └─ merge_engine(base, local, remote)
               ├─ merged document
               └─ conflicts: Vec<FieldConflict>
     conflicts empty?
       yes → write merged, commit, push (now fast-forwards), Healthy
       no  → persist conflicts to sync state, health = Attention,
             surface "Resolve conflicts" entry; auto-push stays gated off
Resolver dive (user picks a side per field) → Apply
  └─ snapshot BOTH sides → sync/backups/<ts>-conflict.json
     write merged-with-choices, commit, push, clear incident, Healthy
```

## Merge engine (backend)

Operates on the **parsed profile document**, not git trees — git's line-level
conflicts on pretty-printed JSON are exactly the unusable output we're replacing.

For each synced file (the promote allowlist: `manifest.json`, `core/*`,
`os/<bucket>/*`; backups are not merged), load the three versions and merge by
key path:

- key present/equal in all three, or changed identically → take that value
- changed in local only (base ≠ local, base == remote) → **keep mine**, auto
- changed in remote only (base == local, base ≠ remote) → **take remote**, auto
- changed in both to different values → **FieldConflict** (surfaced)
- added on one side only → take it
- removed on one side only → respect the removal (records as an auto change)
- nested objects → recurse; arrays are compared as whole values (a changed
  array is one leaf for v1)

Special cases:

- **`plugins.lock.json`** uses the existing union semantics
  (`qol-tray-feature-profile`: preserve unsupported plugins, preserve repo URLs
  for survivors). The engine delegates lock reconciliation to the existing rule
  rather than treating it as generic JSON. A plugin that changed *version* on
  both sides is a `FieldConflict` like any other field.
- **Files outside the allowlist** are never merged or promoted (defense-in-depth
  matches `promote.rs`).

```
struct FieldConflict {
    file: String,         // e.g. default/core/plugin-configs/plugin-alt-tab.json
    plugin: Option<String>,
    key_path: String,     // dotted path within the file, e.g. "opacity"
    local: serde_json::Value,
    remote: serde_json::Value,
    local_edited: Option<String>,   // display only
    remote_edited: Option<String>,  // display only
}

enum MergeOutcome {
    Clean(MergedProfile),
    Conflicts { merged_so_far: MergedProfile, conflicts: Vec<FieldConflict> },
}
```

### "Last edited" source — decision

Per-field edit dates are **display only** (never used to auto-resolve). Source:

- Primary: **git-blame the key's line** in the pretty-printed file at each
  side's tip (`local` = HEAD, `remote` = `origin/main`), take that commit's
  time. Pretty-printed one-key-per-line JSON makes this a good approximation.
- Fallback: the side's tip-commit time ("last synced") when blame can't isolate
  a line (newly added file, reformatted blob).

Approximation is acceptable because it only informs the human, who makes the
call. Flagged for review — if blame proves too slow over large histories, ship
the fallback alone in v1.

## Sync service changes (backend)

`src/features/profile/sync/`:

- `git_repo.rs`: add `merge_base(local, remote)` and a way to read a file blob
  at an arbitrary commit (for the three snapshots). `pull` keeps its FF path;
  divergence now hands control to the reconcile flow instead of returning a bare
  `Diverged`.
- New `merge.rs`: the pure engine above + table tests.
- `service.rs`:
  - `do_pull`: on divergence, run the engine. Clean → apply+push. Conflicts →
    persist `Vec<FieldConflict>` into sync state, set incident
    `kind = Conflict`, health `Attention`.
  - New `resolve_conflicts(choices)`: apply the user's per-field picks to the
    merged document, snapshot both sides to `sync/backups/<ts>-conflict.json`,
    write, commit, push, clear incident.
  - **Remove `acknowledge_incident`** and the `/sync/acknowledge` route — it is
    the trap that re-arms auto-push without resolving. (Conflicts now have a
    real resolution path; there is nothing benign to acknowledge.)
  - **Pull-before-push**: `auto_push_if_dirty` and `manual_push` attempt a
    fetch+reconcile first, so a clash is caught when it is one field instead of
    after 70 commits. (Keeps the existing "skip auto-push while incident
    pending" gate.)
- `types.rs`: extend `SyncStatus` with the pending conflicts (or a count + a
  fetch endpoint for the dive to load detail). `SyncIncidentKind::Conflict`
  already exists.
- `http/sync.rs` + `http/mod.rs`: replace the `acknowledge` route with
  `GET /sync/conflicts` (load detail for the dive) and
  `POST /sync/conflicts/resolve` (submit choices).

## Resolver UI (frontend)

Built in qol-tray's real style — token CSS per `ui/styles/STYLE_GUIDE.md`,
`Surface`/`ListRow` composition, keyboard-first. See `qol-tray-ui-systems`,
`qol-tray-page-creation`, `qol-world-canvas`. The approved prototype
(`.superpowers/brainstorm/.../conflict-resolver.html`) defines layout and flow
only; none of its ad-hoc CSS ships.

- **Entry**: a dive target (e.g. `profile-sync-conflicts`) registered at the
  page-creation sites, surfaced from the Profile sync section when health is
  `Attention` with `kind = Conflict`. Diving in loads `GET /sync/conflicts`.
- **Stepper**: one `FieldConflict` per screen, `current / total`, progress dots.
  - Two selectable sides — "This Mac" vs "Remote · <other machine>" — each with
    the value and last-edited date.
  - Below, the field shown in its full config diff: the conflicting key rendered
    both ways with the chosen side highlighted; non-conflicting changes labelled
    "auto-merged" so the user sees they are handled.
  - Per-field pick; revisit/change before applying.
  - Keyboard: ←/→ pick a side, n/p (or ↑/↓) move, enter advance — routed through
    the existing keyboard nav, not ad-hoc listeners.
- **Confirm screen**: kept-mine / took-remote / auto-merged tally, per-conflict
  summary, and the apply contract (backup → commit → push → Healthy). Apply
  posts `POST /sync/conflicts/resolve`. Back returns to the stepper.
- Leaving the dive without applying keeps the incident pending (nothing is
  written); the user can return later.

## Edge cases

- **0 conflicts after fetch**: never enters the dive — auto-applies and is
  Healthy. The state the user hit becomes self-healing.
- **One side deleted a plugin/config the other edited**: surfaced as a conflict
  with delete-vs-edit framing (keep / drop), not silently resolved.
- **Non-JSON or unparseable file in the allowlist**: fall back to a whole-file
  conflict (keep mine / take remote for that file) rather than crashing.
- **Apply fails mid-push** (e.g. remote moved again): re-fetch, re-reconcile,
  re-enter the dive with any new conflicts. The pre-write backup already exists.
- **Abort / app quit mid-resolution**: nothing written until Apply; incident
  persists and the dive is re-enterable.

## Testing

- `merge.rs`: table-driven unit tests — each bucket (mine-only, remote-only,
  both-equal, true conflict, add, delete, nested, array-as-leaf), plus the lock
  union rule, with dense case sets per `qol-apps-testing`.
- `tests/profile_feature.rs`: end-to-end — set up a bare origin, diverge two
  clones, pull, assert conflicts surfaced; resolve with mixed choices; assert
  merged document, that a `<ts>-conflict.json` backup was written, that push
  fast-forwards, and that health returns to Healthy.
- Pull-before-push regression: a clash is caught at one field, not accumulated.
- Removal of `acknowledge`: assert the route is gone and divergence cannot be
  dismissed without resolving.
- UI: `node --check` on edited `ui/views/profile/` files; keyboard nav covered
  by the existing UI test patterns.
- Full verification stack per `qol-tray-feature-profile` before done.

## Decisions (locked for v1)

1. **Last-edited source** — git-blame the field's line per side as the primary
   source, falling back to the side's tip-commit time when blame can't isolate
   a line. Display only; never auto-resolves.
2. **Array granularity** — a changed array is a single leaf; the user picks the
   whole array. Element-level merge deferred.
3. **Conflict transport** — `SyncStatus` carries a conflict *count* only; the
   resolver dive loads full detail via `GET /sync/conflicts`.
