# Theme Identity Axis Design

Date: 2026-07-16
Status: Approved direction via live mock iteration (see Reference mocks); spec pending user review.

## Goal

Themes currently change only color palettes, which reads as "the same app, slightly tinted".
This design adds an identity axis so a theme can change typography, casing, radius, glow,
and surface treatment. The retro terminal look stops being the app's baseline and becomes
the identity of the slate, graphite, and void themes. Midnight adopts a modern identity.

## Reference mocks

Approved target for midnight (injected-CSS mocks on the live UI, repo root):

- `identity-mock-v6-minimap.png` (final: cards, title, palette hint, kebab, minimap)
- `theme-midnight-violet-plugins.png` (before: retro identity for contrast)

## Identity values

Two named identities ship initially. Retro is the current appearance and must stay
visually unchanged for slate/graphite/void.

| Axis | Retro (current) | Modern (mock) |
| --- | --- | --- |
| UI font | mono stack | sans stack |
| Data font (versions, keys, logs, numbers) | mono | mono |
| Label casing / tracking | uppercase, wide | none, normal |
| Heading presence | small boxed mono chip | large unboxed sans, weight 700 |
| Radius scale | 2-12px | 6-18px |
| Text glow | `--tui-glow-text` | none |
| Page frame | double border + scanline + CRT band animation | none (transparent shell) |
| Card surface | outlined box, accent-wash gradient, inset glow | filled raised tile, no border, layered shadow |
| Cover placeholder | scanline screen + glowing mono monogram | accent-gradient icon tile, white sans initials |
| Selection indicator | accent border + glow ring | accent outline ring with offset |
| Secondary action buttons (kebab, cog) | bare/bordered | accent-tinted ghost square |
| Shortcut hint | faint borderless text | elevated pill + mono kbd chip |
| Peripherals (minimap, cog anchor) | sharp bordered panels | rounded floating panels with shadow |
| Minimap slabs (canvas-drawn) | sharp rects | rounded rects via radius token |

## Architecture

### 1. qol-theme model

```rust
pub struct ThemeIdentity {
    pub key: &'static str,            // "retro" | "modern"
    pub font_ui: &'static str,        // CSS font stack
    pub font_data: &'static str,
    pub case_label: &'static str,     // CSS text-transform value
    pub tracking_label: &'static str, // CSS letter-spacing value
    pub radius: RadiusScale,          // 2xs..xl in px
    pub glow_strength: f32,           // 0.0..1.0 multiplier
    pub frame: FrameStyle,            // Terminal | None
    pub card: CardStyle,              // Outlined | Filled
    pub cover: CoverStyle,            // Screen | IconTile
}
```

`TrayThemePreset` gains `identity: &'static ThemeIdentity`. Two consts:
`RETRO_IDENTITY` (slate, graphite, void) and `MODERN_IDENTITY` (midnight).
Exact enum/field shapes may be refined in the plan; the constraint is that identity
is preset DATA, so retuning which themes are retro is a one-line change.

### 2. Generator

`tray_css()` emits identity tokens in the same base-`:root` + per-theme
`:root[data-qol-theme="<key>"]` diff blocks used for colors. New tokens:

`--font-ui`, `--font-data`, `--case-label`, `--tracking-label`,
`--radius-*` (existing scale, now theme-driven), `--glow-text`,
`--identity-frame` / `--identity-card` / `--identity-cover` style values
(concrete token set finalized in the plan; discrete style switches emit
component-consumable values, e.g. `--card-border`, `--card-shadow`,
`--cover-texture`, `--frame-border`, not JS-visible mode flags).

`tray_theme_js()` gains `identityKey` per theme for the few JS consumers
(minimap draw, monogram rendering) that branch on structure rather than tokens.

### 3. UI migration (one-time sweep)

- Route the 31 hardcoded `text-transform: uppercase` sites through `var(--case-label)`
  (paired with `var(--tracking-label)`).
- Split font usage: `--font-ui` on interface text, `--font-data` on versions, hotkey
  captures, logs, numeric readouts. The existing `--font-sans`/`--font-mono` stay as
  raw stacks that identities point at.
- Card, cover, frame, badge, palette-hint, peripheral styles consume the new identity
  tokens instead of hardcoded `--tui-*` retro values. Retro identity token values are
  chosen so the rendered result is pixel-identical to today.
- Monogram rendering: cover style token switches between screen-monogram and icon-tile
  markup/classes (single component, class switch from `identityKey`).
- Minimap draw code reads the radius token (via getComputedStyle at draw time) and
  rounds slabs; retro radius keeps them sharp.

### 4. Keyboard-first invariant

Selection/focus indication exists in every identity (outline ring in modern, border+glow
in retro). No identity may remove the visible selection state.

## Testing

- qol-theme: identity emission tests (retro themes emit retro token values; midnight
  emits modern values); contrast tests unchanged; generated-artifact currency test
  already guards regeneration.
- Retro no-change guard: a test asserts slate's emitted CSS for the migrated tokens
  matches the current hardcoded values (protects the "pixel-identical retro" promise).
- UI: existing suite plus targeted tests where logic branches on identityKey.
- Live verification via Playwright screenshots on plugins page, config page, settings
  panel for midnight (modern) and slate (retro unchanged).

## Out of scope

- Layout/composition changes (grid stays a grid in every theme).
- Light themes (palette model already supports adding them later).
- Seed-generated palettes (separate follow-up; scratch generator exists).
