# QOL Tray UI Style Guide

This is the source of truth for the *rules*. The live token values are in
`theme-tokens.css`; the live component inventory is the Dev tab's
`ComponentsCatalog.js`. This file never enumerates current per-component state -
that rots. It only encodes the invariants every new surface must obey.

## Source of Truth

- Palette and semantic tokens are defined in `theme-tokens.css`.
- `styles.css` is the entry aggregator - it only `@import`s the stylesheets; it defines no tokens.
- All UI consumes semantic tokens. Hardcoded color literals are not allowed in shared styles.

## Token Layers

Three layers, consumed inside-out. Never reach past a layer:

1. **Palette** (`--slate-*`, `--blue-*`, `--green-*`, `--red-*`, `--amber-*`) - raw anchors. Only referenced when *defining* a semantic token.
2. **Channel** (`--accent-rgb`, `--success-rgb`, `--danger-rgb`, `--warning-rgb`, `--ink-rgb`, `--paper-rgb`) - bare RGB triples for alpha composition.
3. **Semantic** (`--bg-*`, `--surface-*`, `--text-*`, `--border-*`, `--accent`, `--success`, `--danger`, `--warning`, `--state-*`, `--layer-*`) - what views actually use.

## Usage Rules

- Reach for a semantic token first (`--bg-surface`, `--text-muted`, `--border-default`).
- Use a palette token only when defining or extending a semantic token.
- For alpha variants, compose a channel token: `rgba(var(--accent-rgb), 0.2)`, `rgba(var(--ink-rgb), 0.6)`.
- If an alpha pattern repeats in 2+ places, promote it to a `--layer-*` token.
- Keep compatibility aliases stable unless you migrate every usage in one change.

## Component Contracts

Each role binds to semantic tokens, never to literals:

- **Buttons** - primary `--accent`, success `--success`, danger `--danger`, ghost `--bg-*` + `--border-*` + `--text-*` (each with its `-hover`).
- **Feedback/status** - `--state-{info,success,danger,warning}-bg` plus the matching semantic border/text.
- **Cards/panels** - default `--bg-surface` + `--border-subtle`; hover `--bg-hover` + `--border-hover`; selected `--bg-selected` + accent border.
- **Progress** - never a fully-invisible loading state; the fill must stay clearly brighter than its track on every dark surface; drive width via `--progress-scale` on the shared `progress-track`/`progress-fill` classes, never inline `width`; determinate bars start at 0%.

## File Scope Rules

- Each stylesheet owns exactly one scope (tokens, a component family, a page, or app-shell layout).
- Aggregator stylesheets (e.g. `styles.css`, `common-components.css`) only `@import`; they hold no rules of their own.
- View-specific stylesheets hold layout and view treatment only - no token definitions.
- Standalone pages alias the shared ramp (the `--cfg-*` config tokens live in `common-config-primitives.css`/`auto-config-page.css` and alias the accent/tui tokens); they never re-declare raw color literals.

## Migration Rules

- Touching a block with a hardcoded color literal? Migrate that block to tokens in the same change.
- A new visual meaning means a new semantic token in `theme-tokens.css` first - never a one-off literal.

## Review Checklist

- No new hardcoded color values in shared CSS.
- Opacity overlays use `rgba(var(--*-rgb), alpha)` channels.
- Reused patterns use existing component classes/tokens.
- Mobile breakpoints preserve contrast and hierarchy.

---

# TUI Design Language

The visual identity is a DOS / terminal-window desktop. This section is the source
of truth for the *look*; the token sections above are the source of truth for the
*values*. Every new surface must place itself in this metaphor before styling.

## The Layer Metaphor

From outermost in, each layer has exactly one treatment. Do not mix treatments across layers.

| Layer | Role | Treatment |
|-------|------|-----------|
| Desktop | app backdrop behind every view | `--tui-desktop-bg` (accent-tinted radial over near-black), fixed attachment |
| Window | the page panel (every view root) | `--border-w-3 double --tui-line` frame, `--tui-panel-bg`, `--tui-panel-shadow`, scanline `::after` |
| Sign | the page title | framed plaque straddling the top border line, mono uppercase, `--tui-glow-text`; scales with zoom; a fixed title-clearance band keeps it off content |
| Screen | a CRT surface inside the window (card cover, live panels) | `--tui-screen-bg`, scanline `::after`, glowing accent monogram/content |
| Cartridge | a card (plugin / store) | single `--tui-line-soft` hairline, `--tui-bg-card`, crisp shadow, accent select ring; holds a Screen |
| Controls | buttons, inputs, selects, toggles | single hairline, mono uppercase labels, square-ish radius, accent on active/primary |
| Lines | list / table rows | left accent border = the cursor marker; warm-amber selected fill |

## Accent SSOT

- One triple, `--accent-rgb`, drives every accent color. `--accent: rgb(var(--accent-rgb))`.
- Amber is the everyday default; green is reserved for dev mode; other presets are user-selectable.
- The accent palette and default key come from the boot document (`window.__QOL_BOOT__`); the runtime hook in `ui/lib/accent-presets.js` applies the selected triple to `:root`. The static seed lives in `theme-tokens.css`.
- Never reference `--blue-*` for accent. Never hardcode an accent literal. Alpha variants use `rgba(var(--accent-rgb), a)`.

## Frame Hierarchy (hard rule)

- **Double line** (`--border-w-3 double --tui-line`) is reserved for the Window and the Sign only - the "this is application chrome" marker.
- **Single hairline** (`--tui-line` solid, or `--tui-line-soft` for quiet edges) is for everything inside: cards, controls, rows, alerts, popovers.
- Two double-lines must never nest. Reaching for a double border inside a panel means you want a single hairline.

## Surface Treatments

- **Scanline** (`--tui-scanline`): only on Screens (card covers, live/active panels). Apply via a `::after` overlay with `pointer-events:none`; lift real content with `position:relative; z-index:1`.
- **Glow** (`--tui-glow-text`): accent text emphasis - signs, monograms, health dots, active labels. Not on body text.
- **Tint**: panel/desktop backgrounds get a faint accent wash over near-black. Keep alphas in the 0.03-0.08 range so the retint stays subtle.

## Typography

- **Mono uppercase** (`--font-mono` + `text-transform:uppercase` + `--ls-md`/`--ls-lg`) is the TUI label voice: page signs, card names, button labels, badges, section labels, key cells. Promote the trio to the shared `.tui-label` utility rather than repeating it.
- **Sans** stays for body copy, descriptions, values, and long text. Do not uppercase prose.

## Selection

- Selected = accent left-border (rows) or accent ring (`--selected-ring`) + warm-amber fill from `--bg-selected`. The fill must be accent-warm, never blue.
- `[data-selected-surface][data-selected="true"]{border-color:var(--accent)}` is the global selection language. Reuse it; never invent per-view selection styling.

## TUI Token Inventory

Defined in `theme-tokens.css` (this lists the families, not the values):

- Lines: `--tui-line`, `--tui-line-soft`
- Textures: `--tui-scanline`, `--tui-glow-text`
- Surfaces: `--tui-panel-bg`, `--tui-screen-bg`, `--tui-sign-bg`, `--tui-desktop-bg`
- Near-black ramp: `--tui-bg-screen` (deepest), `--tui-bg-card`, `--tui-bg-panel`, `--tui-bg-desktop`. The `--tui-*-bg` composites and card backgrounds reference these - never hardcode a near-black literal.
