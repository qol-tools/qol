# QOL Tray UI Style Guide

## Source of Truth

- Global palette and semantic tokens live in `ui/styles/foundation.css`.
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

## Usage Rules

- Use semantic tokens first (`--bg-surface`, `--text-muted`, `--border-default`).
- Use palette tokens only when defining or extending semantic tokens.
- For alpha variants, use channel tokens:
  - `rgba(var(--accent-rgb), 0.2)`
  - `rgba(var(--paper-rgb), 0.08)`
  - `rgba(var(--ink-rgb), 0.6)`
- Keep compatibility aliases in `foundation.css` stable unless migrating all usages in one change.

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

- Cards/panels
  - Default: `--bg-surface` + `--border-subtle`
  - Hover: `--bg-hover` + `--border-hover`
  - Selected: `--bg-selected` + accent border

## File Scope Rules

- `foundation.css`: only global tokens and app-wide primitives.
- `common-components.css`: reusable component styles only.
- View-specific files (for example `dev-page.css`): layout and view treatment only.
- Standalone pages (for example `auto-config.html`) must define local `--cfg-*` tokens and avoid repeating raw color literals.

## Migration Rules

- When touching a style block with hardcoded color literals, migrate that block to tokens in the same change.
- Do not introduce new one-off colors without first adding a token.
- If a new visual meaning is needed, add a semantic token in `foundation.css` first.

## Review Checklist

- No new hardcoded color values in shared CSS.
- New opacity overlays use `rgba(var(--*-rgb), alpha)` channels.
- Reused UI patterns use existing component classes/tokens.
- Mobile breakpoints preserve contrast and hierarchy.
