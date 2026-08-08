# qol-tray web UI theming and component consolidation

Approved 2026-07-16.
Scope: tray web UI only; themes defined centrally in `libs/qol-theme` so gpui plugins can adopt later.

## Goals

1. Every view composes components from the component gallery; no stray raw interactive elements.
2. Full named theme palettes, switchable at runtime, dark variants only in the initial set.
3. One global style pass (depth, typography, density) that lands in every theme.

Accent remains an independent axis and composes with any theme.
Typography, spacing, and elevation are global tokens, not per-theme values.

## 1. Theme model in qol-theme

A `Theme` is a named, complete palette defined in Rust: the ~17 `--qol-system-*` values (4 surface tiers, 4 text tiers, border tiers, status RGBs) plus the overlay/scrim RGBs currently hardcoded in `theme-tokens.css`, which move into the theme definition.

The generator (`qol-theme-css`) emits into `generated-theme-tokens.css`:

- the default theme's values on `:root` as the no-flash fallback,
- one `:root[data-theme="<name>"]` override block per theme,
- `QOL_THEMES` metadata (key, label) into `generated-theme-tokens.js`, mirroring `QOL_ACCENT_PRESETS`.

Initial themes, all dark:

- `slate`: today's palette retuned for stronger surface-tier separation.
- `graphite`: warm neutral dark.
- `void`: near-black OLED.

Rust tests assert per-theme contrast floors between adjacent surface tiers and text-on-surface pairs, so a mushy palette fails CI.
The existing stale-check test covers regeneration drift.
The abstraction must not assume dark: a light theme later is a new palette, not a rework.

## 2. Persistence, boot, and the switcher

- `src/features/theme.rs`: `theme.json` gains `theme: Option<String>` beside `accent`, with the same validated-key save/clear/resolve trio and env passthrough pattern for future plugin use.
- `theme_handlers.rs`: GET/PUT endpoints matching the accent ones.
- `boot.rs`: inject `theme: { selectedKey, themes }` into `__QOL_BOOT__`; the HTML shell sets `data-theme` on `<html>` before first paint.
- `ui/lib/theme-presets.js` mirrors `accent-presets.js`; `applyTheme(key)` sets the attribute only, no per-property loop.

Switcher UI: theme swatch row (surface stack preview plus name) beside the accent picker in settings, built from gallery components, keyboard-first.
The components gallery page gets the same switcher inline for auditing primitives under every theme.

## 3. Component consolidation and guard test

- Migrate the ~21 view files rendering raw `<button>`/`<select>`/checkbox inputs to gallery primitives (`Button`, `CustomSelect`, `ToggleSwitch`, ...).
- Missing primitives (inline text input in the `LinkInput` style; possibly an icon-button variant) are promoted into `lib/components/` and cataloged first, then consumed.
- Domain rows stay in `components/domain-rows/` but compose gallery primitives internally.
- Guard test: a `node --test` file walks `views/`, `components/`, and `app/` and fails on raw `<button`, `<select`, `<input`, `<textarea` outside `lib/components/`, with an explicit grandfather list whose failure message names the offending file.
- The sweep is per-file atomic: each converted view is its own commit, compiles, and shrinks the grandfather list.

## 4. Global style pass

After theming lands, one pass over non-theme tokens in `lib/components/` and shared CSS only:

- Depth: `--elevation-1..3` shadow tokens applied consistently in `Surface`/`Card`/modal primitives.
- Typography: tighten the `--fs-*`/`--fw-*` scale in gallery primitives; fewer sizes, clearer row-title/meta/section hierarchy.
- Density: normalize row heights and paddings through `--space-*` in `ListRow`/`TableRow`/fields.

Judged on the components gallery page under all three themes.

## Verification

- qol-theme stale-check and contrast tests (Rust).
- Theme endpoint tests.
- Stray-component guard test.
- Existing UI tests stay green.
- Playwright pass on the live UI: recompile, switch each theme, screenshot gallery and main views.

## Build order

1. Theme model.
2. Persistence and switcher.
3. Consolidation.
4. Style pass.
