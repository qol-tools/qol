# QOL Tray UI Style Guide

## Source of Truth

- Global palette and semantic tokens live in `ui/styles/styles.css`.
- All new UI styles must consume tokens from `:root`.
- Hardcoded colors are not allowed in shared styles.

## Token Layers

1. Palette tokens
- `--slate-*`
- `--blue-*`
- `--green-*`
- `--red-*`
- `--amber-*`

2. Channel tokens
- `--accent-rgb`
- `--success-rgb`
- `--danger-rgb`
- `--warning-rgb`
- `--ink-rgb`
- `--paper-rgb`

3. Semantic tokens
- Surfaces: `--surface-*`, `--bg-*`
- Text: `--text-*`
- Borders: `--border-*`
- States: `--accent`, `--success`, `--danger`, `--warning`, `--state-*`
- Layer alphas: `--layer-paper-*`, `--layer-ink-*`

## Usage Rules

- Use semantic tokens first (`--bg-surface`, `--text-muted`, `--border-default`).
- Use palette tokens only when defining or extending semantic tokens.
- For alpha variants, use channel tokens:
  - `rgba(var(--accent-rgb), 0.2)`
  - `rgba(var(--paper-rgb), 0.08)`
  - `rgba(var(--ink-rgb), 0.6)`
- If an alpha pattern is reused in 2+ places, promote it to a `--layer-*` token.
- Keep compatibility aliases in `styles.css` stable unless migrating all usages in one change.

## Component Contracts

- Buttons
  - Primary: `--accent` / `--accent-hover`
  - Success: `--success` / `--success-hover`
  - Danger: `--danger` / `--danger-hover`
  - Ghost: `--bg-*` + `--border-*` + `--text-*`

- Feedback/status
  - Info: `--state-info-bg` + accent border/text
  - Success: `--state-success-bg` + success border/text
  - Error: `--state-danger-bg` + danger border/text
  - Warning: `--state-warning-bg` + warning border/text

- Progress indicators
  - Never allow a fully invisible loading state.
  - Determinate progress bars start at 0% and must not force non-zero initial width.
  - Queued states may use copy and subtle row treatment instead of forced fill width.
  - Progress affordances used by action cards must keep a bright active segment on a darker track.
  - Determinate progress should still use semantic accent tokens.
  - Use shared progress tokens: `--progress-track-bg`, `--progress-track-inset`, `--progress-fill-start`, `--progress-fill-end`, `--progress-fill-glow`.
  - Reuse `progress-track` + `progress-fill` shared classes and set fill via `--progress-scale` instead of inline `width`.
  - Progress fill must remain clearly brighter than its track in all dark surfaces.

- Cards/panels
  - Default: `--bg-surface` + `--border-subtle`
  - Hover: `--bg-hover` + `--border-hover`
  - Selected: `--bg-selected` + accent border

## File Scope Rules

- `styles.css`: only global tokens and app-wide primitives.
- `theme-tokens.css`: color palette and semantic token definitions.
- `common-controls.css`: shared form controls, search bars, input styles.
- `common-components.css`: reusable component styles only.
- `page-header.css`: PageHeader layout, noise border/reveal animation, command palette positioning.
- `app-shell.css`: sidebar, view slots, app-level layout.
- View-specific files (for example `dev-layout.css`, `plugin-grid.css`): layout and view treatment only.
- Standalone pages (for example `auto-config.html`) must define local `--cfg-*` tokens and avoid repeating raw color literals.

## Migration Rules

- When touching a style block with hardcoded color literals, migrate that block to tokens in the same change.
- Do not introduce new one-off colors without first adding a token.
- If a new visual meaning is needed, add a semantic token in `styles.css` first.

## Review Checklist

- No new hardcoded color values in shared CSS.
- New opacity overlays use `rgba(var(--*-rgb), alpha)` channels.
- Reused UI patterns use existing component classes/tokens.
- Mobile breakpoints preserve contrast and hierarchy.

---

# TUI Design Language

The visual identity is a DOS / terminal-window desktop. This section is the source of
truth for the *look*; the token sections above are the source of truth for the *values*.
Every new surface must place itself in this metaphor before styling.

## The Layer Metaphor

From outermost in, each layer has exactly one treatment. Do not mix treatments across layers.

| Layer | Role | Treatment |
|-------|------|-----------|
| Desktop | app backdrop behind every view | `--tui-desktop-bg` (accent-tinted radial over near-black), fixed attachment |
| Window | the page panel (every view root) | `--border-w-3 double --tui-line` frame, `--tui-panel-bg`, `--tui-panel-shadow`, scanline `::after` |
| Sign | the page title | framed plaque straddling the top border line, mono uppercase, `--tui-glow-text`; scales with zoom so its dip stays bounded; a fixed title-clearance band keeps it off content |
| Screen | a CRT surface inside the window (card cover, live panels) | `--tui-screen-bg`, scanline `::after`, glowing accent monogram/content |
| Cartridge | a card (plugin / store) | single `--tui-line-soft` hairline, `--tui-bg-card`, crisp shadow, accent select ring; holds a Screen |
| Controls | buttons, inputs, selects, toggles | single hairline, mono uppercase labels, square-ish radius, accent on active/primary |
| Lines | list / table rows | left accent border = the cursor marker; warm-amber selected fill |

## Accent SSOT

- One triple, `--accent-rgb`, drives every accent color. `--accent: rgb(var(--accent-rgb))`.
- Amber (`255, 180, 84`) is the everyday default. Green is reserved for dev mode. Other presets are user-selectable.
- The runtime hook (`applyAccent`/`resolveAccent`, `ui/lib/accent-presets.js`) sets `--accent-rgb` + `--accent-hover` on `:root`. Setting lives in `world-settings.js` (`accent`), control in the Appearance section of `WorldSettingsPanel`.
- Never reference `--blue-*` for accent. Never hardcode an accent literal. Alpha variants use `rgba(var(--accent-rgb), a)`.

## Frame Hierarchy (hard rule)

- **Double line** (`--border-w-3 double --tui-line`) is reserved for the Window and the Sign only. It is the "this is the application chrome" marker.
- **Single hairline** (`--tui-line` solid, or `--tui-line-soft` for quiet edges) is for everything inside: cards, controls, rows, alerts, popovers.
- Two double-lines must never nest. If you reach for a double border inside a panel, use a single hairline instead.

## Surface Treatments

- **Scanline** (`--tui-scanline`): only on Screens (card covers, live/active panels). Apply via a `::after` overlay with `pointer-events:none`; lift real content with `position:relative; z-index:1`.
- **Glow** (`--tui-glow-text`): accent text emphasis - signs, monograms, health dots, active labels. Not on body text.
- **Tint**: panel/desktop backgrounds get a faint accent wash over near-black. Keep alphas in the 0.03 to 0.08 range so the retint stays subtle.

## Typography

- **Mono uppercase** (`--font-mono` + `text-transform:uppercase` + `--ls-md`/`--ls-lg`) is the TUI label voice: page signs, card names, button labels, badges, section labels, key cells.
- **Sans** stays for body copy, descriptions, values, and long text. Do not uppercase prose.
- Promote the label trio to a shared `.tui-label` utility rather than repeating it per component.

## Selection

- Selected = accent left-border (rows) or accent ring (`--selected-ring`) + warm-amber fill. The fill comes from `--bg-selected`; it must be accent-warm, never blue.
- The wedge/`[data-selected-surface][data-selected="true"]{border-color:var(--accent)}` rule is the global selection language. Reuse it; do not invent per-view selection styling.

## TUI Token Inventory

Defined in `theme-tokens.css`:

- Lines: `--tui-line`, `--tui-line-soft`
- Textures: `--tui-scanline`, `--tui-glow-text`
- Surfaces: `--tui-panel-bg`, `--tui-panel-shadow`, `--tui-screen-bg`, `--tui-sign-bg`, `--tui-desktop-bg`

**Near-black ramp.** The dark bases are tokenized; the `--tui-*-bg` composites and the card
backgrounds reference them. Never hardcode a near-black literal again.

- `--tui-bg-screen` (deepest, CRT screen)
- `--tui-bg-card` (cartridge body)
- `--tui-bg-panel` (window + sign)
- `--tui-bg-desktop` (backdrop)

---

# Component Treatment Matrix

Audited against the dev component gallery (`ui/views/dev/components/ComponentsCatalog.js`,
16 showcases). Status: **done** = matches the language, **partial** = on-theme but incomplete,
**todo** = still generic/blue/soft.

## Chrome (done)

| Component | Treatment | Status |
|-----------|-----------|--------|
| Page panel (`.view-container.content-shell`) | double-line window + scanline + panel tint | done |
| Title sign (`.world-region-label`) | framed plaque, straddles border, mono glow, zoom-scaled | done |
| Desktop (`body`) | accent-tinted radial backdrop | done |

## Cards / Cartridges

| Component | Base | Current | Target | Status |
|-----------|------|---------|--------|--------|
| `.card` (base) | - | standard elevation, hairline | single `--tui-line-soft`, `--tui-bg-card`, crisp shadow, accent ring | todo |
| plugin media card | card | `--tui-line-soft` + accent gradient, **`#0a0b0d`** | migrate bg to `--tui-bg-card`; share one cartridge rule with store card | partial |
| store card | card | `--tui-line-soft` + gradient, **`#0c0d10`**, `radius-lg` | unify radius + shadow with media card on one shared rule | partial |
| cover + monogram | - | CRT screen + scanline + glow monogram, mono uppercase name | - | done |
| status chips / DEV badge | - | semantic uppercase pills (amber dev, green linked) | keep (chips are read-only) | done |
| cog button | - | generic ink/paper pill | hairline + accent hover to match controls | partial |
| update button | - | green success btn | keep (success semantic) | done |

## Controls

| Component | Current | Target | Status |
|-----------|---------|--------|--------|
| Button (primary) | amber gradient + glow, soft radius, sentence case | mono uppercase label, `radius-sm`, keep amber border+glow | partial |
| Button (ghost/secondary/danger) | token-driven, sentence case | mono uppercase label, single hairline (ghost) | partial |
| CustomSelect | soft rounded, pastel, **no TUI** | terminal menu: `--tui-line` trigger+popover, `--tui-bg-panel`, `radius-sm`, mono options, amber highlight | todo |
| ToggleSwitch | iOS pill, accent on-state | square the track (`radius-sm`), inset shadow, keep accent on; or bracket `[ON]/[OFF]` (decision) | partial |
| RefreshButton / spinner | circular accent-top spinner | keep | done |
| Search input | borderless, color-shift focus | terminal prompt: mono, leading `>`/`_`, thin amber bottom rule | todo |
| Expander | ghost button + bordered body | body top edge to `--tui-line-soft` | partial |

## Lines (rows)

| Component | Base | Current | Target | Status |
|-----------|------|---------|--------|--------|
| ListRow / TableRow base | base | 3px left accent marker, `--bg-selected` fill | **selected fill must retint amber** (single token) | partial |
| DevPluginRow | TableRow | rich states; **`var(--slate-200)`** literal | inherit retint; fix literal to token | partial |
| LogRow | ListRow | level-colored badges, mono code cells | inherit retint | partial |
| SuppressedRow | Surface | expandable card, mono | inherit retint; align border to hairline | partial |
| BackupRow | ListRow | accent marker, mono file | inherit retint | done |
| HotkeyRow | TableRow | mono keycaps | inherit retint | done |
| ShortcutRow | TableRow | minimal | inherit retint | done |

## Status Atoms & Containers

| Component | Current | Target | Status |
|-----------|---------|--------|--------|
| Badge | uppercase semantic pills | keep | done |
| HealthDot | small glowing dot; color + glow follow `color` per `data-health` (default faint, success/warning/danger) | - | done |
| Alert | `.profile-sync-alert` semantic rgba | one `.alert` rule: hairline in semantic color, `--tui-bg-panel`, mono label prefix | todo |
| EmptyState | **3 inconsistent defs**, hardcoded italic | consolidate to one `.empty-state`: mono, faint, centered | todo |
| KeyLegend | lowercase caps | optional `[key]` bracket keycaps | partial |
| Surface / selected-surface | accent border + wedge | keep (this is the selection language) | done |

## SSOT Consolidations (coherence wins)

1. **Selection color** (done) - `--surface-selected` is an accent composite that follows the active accent; rows + minimap retint together. No more blue.
2. **Near-black ramp** (done) - the `--tui-bg-*` tokens; the 5 scattered literals are gone.
3. **One cartridge rule** (done) - media + store cards share a single selector set.
4. **`.empty-state`** (deferred) - the two bare `.empty-state` defs are a contextual class-name collision (main app vs the `--cfg-*` config page, which reaches the app via `plugin-config.css`), not a component dup; scope the `--cfg-` one rather than blind-merge. The `EmptyState` component already uses the clean `.empty-state-block` family.
5. **`.alert`** (n/a) - already a single rule (`.profile-sync-alert` + warning/error variants); no duplication to collapse.
6. **`.tui-label`** (done) - the mono+uppercase+spacing trio, defined once.
7. **Frame hierarchy** (done) - double line = chrome only; hairline = everything inside.

## Build Order

- **Phase 0 (foundation) - DONE:** `--surface-selected` is an accent composite (follows the active accent; the minimap rect reads `--accent-rgb` live), the near-black ramp tokens exist, the `.tui-label` utility is defined.
- **Phase 1 (controls) - DONE:** `.btn` mono+uppercase terminal voice (with `text-transform:none` resets where the text is data or content), CustomSelect terminal menu, `>` search prompt, squared toggle.
- **Phase 2 (cards) - DONE:** media + store merged into one cartridge skin; cog on the control hairline; literals migrated to `--tui-bg-*`.
- **Phase 3 (rows) - DONE:** the three list/table containers collapsed into one recessed `--tui-bg-screen` hairline screen; mono uppercase table headers on the `.table-list-header` base; the 3px marker + cursor centralized on `.table-list-row`; `--slate-200` fixed.
- **Phase 4 (atoms) - DONE:** `HealthDot` CSS defined (was rendering invisible - small glowing dot, color/glow per `data-health`). Reassessed the rest: `.empty-state` is a class-name collision (the `--cfg-*` config-primitives copy reaches the main app via `plugin-config.css`), not a component dup - deferred to a scoping pass. `Alert` is already a single semantic rule, no dedup needed.
- Verify each phase live; use an `about:blank` bounce to defeat the nested-`@import` cache.

### Deferred (each its own change)

- The `data-accent` border-left COLOR palette is still duplicated in `list-row.css` + `table.css`. The real fix is a shared `.row` component primitive (touches the JS row components), not cross-file CSS coupling.
- The `.card` base (`card.css`) is still generic; it also skins task-runner action cards, so migrate it once that impact is assessed.
- The search input's "thin amber bottom rule" was intentionally skipped (specificity tie against the command-palette `border-bottom` override).
