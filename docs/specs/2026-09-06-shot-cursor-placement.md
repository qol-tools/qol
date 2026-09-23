# Cursor-anchored shot windows

## User contract

Every preview show and pin action samples the current cursor through the shared
runtime desktop-state authority. A pin does not inherit its preview's origin.
Windows appear beside the cursor, flip at monitor edges, and clamp to the
cursor's monitor. Multiple pins retain independent positions.

## Ownership and coordinate contract

- `qol-runtime` owns native cursor and monitor observations. Its synchronous
  desktop-state request refreshes the cursor; placement consumers do not add
  platform cursor readers.
- `qol-gpui::monitor::MonitorTracker::cursor_anchor` obtains one state snapshot
  and validates its cursor and containing monitor. Missing, stale-indexed,
  nonfinite, or out-of-monitor state returns a typed error without substituting
  the focused or first monitor.
- `CursorAnchor` has private fields and no public constructor. Consumers cannot
  fabricate an anchor from an unrelated window origin.
- `qol-gpui::window::CursorWindowPlacement` combines that anchor with logical
  window dimensions. Resolution requires the actual GPUI window, so Linux
  sizing uses its scale factor. Other platforms retain native point units.
- Resolution scales dimensions before native edge placement, then converts the
  resulting native origin back to GPUI logical bounds. Gap and margin are
  native desktop units. Private `ResolvedCursorPlacement` construction keeps
  consumers on this path.
- The shared cursor layout operation resizes GPUI in logical units and applies
  native bounds through the existing popup backend. Linux verifies the native
  origin before reveal; a scale mismatch or failed application is an error.

The old general placement APIs remain for windows with different policies.
Input injection, local pointer events, hover, and drag handling are separate
capabilities; they are not alternate cursor-anchored placement authorities.

## Shot lifecycle

Cached and cold previews use the same typed placement path. Cold windows are
created hidden, prepared, resolved against the actual window, and positioned
before reveal. Linux preview preparation explicitly establishes the existing
shared override-redirect capability, including standalone CLI previews.
The retired asynchronous preview configuration adapter is removed across
platforms, eliminating competing preparation of the same preview.

Pin actions sample a fresh cursor anchor and require a resolved placement.
Cold pins cannot reveal until placement succeeds. Recycled pins clear that
state. Failed pin placement restores usable preview controls; failed preview
placement preserves capture saving and completion behavior.

`CURSOR_APPLY`, `SHOT_PREVIEW_PLACE`, and `SHOT_PIN_PLACE` diagnostic events
record native cursor, geometry, scale, and application outcomes. They use the
existing probe infrastructure and do not add a communication channel.

## Verification

The repository's `qol check` covers affected builds, Clippy, tests, doctests,
formatting, source guards, UI tests, and release-script tests.

The existing disposable Linux Mint workflow accepts these explicit inputs:

```sh
QOL_SHOT_WORKFLOW_SCALE=1 QOL_SHOT_WORKFLOW_PLACEMENT_ONLY=1 cargo run -q -p qol -- flow run qol-shot-storm --env linux/mint-cinnamon --repeat 1 --jobs 1
QOL_SHOT_WORKFLOW_SCALE=2 QOL_SHOT_WORKFLOW_PLACEMENT_ONLY=1 cargo run -q -p qol -- flow run qol-shot-storm --env linux/mint-cinnamon --repeat 1 --jobs 1
```

Both final runs passed. Coverage includes off-center cursor placement, preview
reuse, a cursor moved between preview and pin, two simultaneous pins, edge
flipping, cold standalone previews, and successful standalone exit after
Escape. The guest protocol's process handle owns child waiting and reaping.
Native positions must match within two pixels. Saved report artifacts include
requested and observed geometry, cursor samples, scales, window identities,
screenshots, and verified guest teardown.

Final repository gate: `1788731236153-1169947`.
Final scale-1 flow: `flow-qol-shot-storm-18d2d9a2bcec76f1-124aa5-0`.
Final scale-2 flow: `flow-qol-shot-storm-18d2d9ac2a622932-126259-0`.

Earlier guest evidence reproduced the scale-2 defect: a logical 360x225 pin
became 720x450 native pixels while placement used unscaled dimensions. The
correct native origin was 340,12 instead of 340,420. Cold preview preparation
also produced a verified initial-position readback until the shared Linux
preparation step was applied.

Runtime verification covers Linux Mint at scale 1 and 2. Other platforms have
source-level review and shared arithmetic coverage, not guest runtime proof.
An earlier full recording storm timed out waiting for recording cursor stats;
that separate recording check is outside this placement-only acceptance.
