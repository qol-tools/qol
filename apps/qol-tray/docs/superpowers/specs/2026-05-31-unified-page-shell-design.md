# Unified page shell + single sign anchoring

## Problem

There is no shared page-shell component. Every view hand-rolls its chrome:

- `plugins`: `view-container content-shell` → `SurfaceContainer.view-body`, no header.
- `logs` / `hotkeys` / `shortcuts` / `task-runner` / `profile`: same, plus an in-page `PageHeader <h1>`.
- plugin sections (layer -1): `SurfaceContainer.plugin-config-detail`, no header, no `view-container`.

The floating "sign" (region label) is the real title, but it is positioned from the registry's nominal `entry.height` (900) with rendered-height measurement gated to layer 0 only. On content-sized sub-pages the sign detaches ("flies") on zoom-out. Per-frame work is scattered across 6+ independent `camera.subscribe` sites that each re-walk slots and force layout, so zoom-out is sluggish.

## Decision (agreed)

The sign is the sole title. It stays anchored to each page's top border, behaving identically on every page across the whole world (layer 0, plugin sections, sub-pages). No in-page `<h1>` competing with it.

## Design

1. **`PageShell`** (`ui/components/PageShell.js`) — the one chrome every page renders through: `view-container content-shell` → content frame (`SurfaceContainer`) → `children`, plus an optional thin sub-strip slot for subtitle / key-legend. No `<h1>` title (the sign owns the title).
2. **Top-level views** refactored to render their body through `PageShell`; drop the hand-rolled containers and the in-page `<h1>`.
3. **Plugin sections** (`PluginConfigSectionView`) and **sub-pages** render their content as `PageShell` children — same shell, different content.
4. **Sign anchoring** — `RegionLabels` positions each sign from its slot's *measured rendered rect*, not the nominal height, in one coalesced rAF pass per camera change: write slot scales → force one layout → read all visible slot rects → write all sign positions. Layer-agnostic, so the sign stays put on every layer. Clamp to the viewport top border when the slot top is above the viewport (preserve "sign on the window border when zoomed out"). This single pass replaces the scattered per-subscriber forced-layout work, removing the zoom-out lag.

## Sequencing

1. Extract `PageShell`; refactor layer-0 views onto it. Tests + visual parity green.
2. Route plugin sections + sub-pages through `PageShell`.
3. Unify sign anchoring + coalesced pass; verify on layer -1 (no fly, smooth zoom) live.

## Non-goals

Field rendering, atmosphere/peripheral-preview traits, minimap, the `qol dev <branch>` silent-fallback footgun.

## Risks

Keep `SurfaceContainer` in the shell (keyboard nav breaks silently without it). Hold the `contentSized` invariant and the locked content-sized + region-label tests; update region-label tests to the measured-rect contract rather than weaken them.
