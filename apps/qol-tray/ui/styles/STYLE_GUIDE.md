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
