# Bone Palette Design

Date: 2026-08-20
Status: Spec for implementation review. Ramp derived in a prior design lane, not re-derived here.

## What bone is

Bone is the first light theme in a dark-only codebase: the stage is a warm paper
panel (bone_050 through bone_400 surfaces) instead of a night surface, while the
tray keeps its dark anchor in the ink rail, a bone_950 strip that carries on_ink
text. The fixed blue accent #2f74a0 and its family (accent_ink, accent_fill)
carry the interaction color onto the paper. Nothing else in the theme machinery
changes: bone is a TrayThemePreset exactly like slate, midnight, and the planned
graphite, so the dark-only assumption lives in the presets, not in the palette
plumbing.

## Ramp

13 steps, lightest to darkest. Relative luminance is WCAG 2.1 linearized
(Y = 0.2126R + 0.7152G + 0.0722B). Source tags: T2 = design study, G = graphite
plan (docs/plans/2026-07-16-tray-theming.md:147), I = interpolated.

| token | hex | luminance | source |
| --- | --- | --- | --- |
| bone_050 | 0xFFFDF8 | 0.98288 | T2 |
| bone_100 | 0xFAF7F0 | 0.93137 | T2 |
| bone_200 | 0xF2EDE4 | 0.85047 | G |
| bone_300 | 0xEFE8D9 | 0.81074 | T2 |
| bone_400 | 0xE4DCCB | 0.71992 | T2 |
| bone_500 | 0xCFC7B8 | 0.57573 | G |
| bone_600 | 0xA89F8D | 0.35044 | T2 |
| bone_700 | 0x8C8270 | 0.22711 | T2 |
| bone_650 | 0x7A7161 | 0.16811 | I |
| bone_800 | 0x6E6556 | 0.13294 | T2 |
| bone_850 | 0x5C5448 | 0.09084 | I |
| bone_900 | 0x4A443A | 0.05896 | T2 |
| bone_950 | 0x2B2721 | 0.02074 | T2 |

Monotonicity: luminance strictly decreases across all 13 steps, confirmed
(0.98288 > 0.93137 > 0.85047 > 0.81074 > 0.71992 > 0.57573 > 0.35044 >
0.22711 > 0.16811 > 0.13294 > 0.09084 > 0.05896 > 0.02074). No step is equal
to or lighter than its predecessor. bone_650 sits below bone_700, not between
bone_600 and bone_700: its hex 0x7A7161 is darker than bone_700's 0x8C8270 in
every channel (luminance 0.16811 vs 0.22711). The token name reads one step
higher than its true position; the name is kept because the text_faint fill
references it.

Fixed non-ramp colors: accent 0x2F74A0, accent_ink 0x1F5A82, accent_fill
0xD7E8F3, on_ink 0xFAF7F0, on_ink_muted 0xA89A7C, ink rail surface 0x2B2721
(= bone_950), stage backdrop 0x1B1815.

## Palette fills

Read from the live structs: SystemPalette at libs/qol-theme/src/lib.rs:172,
OverlayPalette at lib.rs:217, TuiBackgroundPalette at lib.rs:225.

### SystemPalette

| field | token | hex | rationale |
| --- | --- | --- | --- |
| surface_canvas | bone_400 | 0xE4DCCB | darkest paper, the stage ground; canvas is the surface furthest from the text color |
| surface_elevated | bone_100 | 0xFAF7F0 | lighter paper, one surface above the canvas ground |
| surface_raised | bone_050 | 0xFFFDF8 | lightest paper; panels and cards sit on raised paper |
| surface_hovered | bone_300 | 0xEFE8D9 | between canvas and elevated, the visible warm shift on paper |
| text_primary | bone_900 | 0x4A443A | dark warm ink, body text on paper |
| text_secondary | bone_850 | 0x5C5448 | secondary body text, darkened for 4.5:1 on canvas, see Corrections |
| text_muted | bone_800 | 0x6E6556 | supporting text, darkened for 3:1 on canvas, see Corrections |
| text_faint | bone_650 | 0x7A7161 | correction, see Corrections; bone_600 fails 3:1 |
| border_subtle | bone_400 | 0xE4DCCB | hairline borders on paper, decorative only |
| accent | (fixed) | 0x2F74A0 | fixed blue, the bone accent |
| success | (fixed) | 0x296F2D | darkened green for the paper ground, 4.52:1 on canvas |
| danger | (fixed) | 0xBA2626 | darkened red for the paper ground, 4.54:1 on canvas |
| info | (fixed) | 0x1F5A82 | equals accent_ink, hue-consistent 5.42:1 on canvas |
| warning | (fixed) | 0x885700 | darkened amber for the paper ground, 4.52:1 on canvas |

### OverlayPalette

| field | token | hex | rationale |
| --- | --- | --- | --- |
| surface_rgb | bone_100 | 0xFAF7F0 | overlay popovers are elevated paper, matching the dark presets' surface-above-canvas step |
| deep_rgb | (fixed) | 0x1B1815 | stage backdrop, the dark environment the paper floats on |
| ink_rgb | bone_950 | 0x2B2721 | the ink rail surface itself, dark anchor carrying on_ink text |
| scrim_rgb | (interp) | 0x0D0B08 | warm near-black, darker than deep like every dark preset's scrim |

### TuiBackgroundPalette

The TUI keeps the dark rail identity (terminals stay ink, not paper).

| field | token | hex | rationale |
| --- | --- | --- | --- |
| desktop | (fixed) | 0x1B1815 | stage backdrop behind the TUI |
| screen | (interp) | 0x100D0A | warm near-black terminal screen, darkest TUI surface like slate's screen |
| panel | bone_950 | 0x2B2721 | ink rail, lightest TUI surface like slate's panel ordering |
| card | (interp) | 0x201D19 | between rail and backdrop, mirroring slate's card-between ordering |

## Contrast

WCAG 2.1, ratio = (Ylight + 0.05) / (Ydark + 0.05). Bars: body text 4.5:1
(primary, secondary, on_ink on accent), non-body 3:1 (muted, faint, accent as
UI color). PASS marks the role's own bar; a non-body row that crosses 4.5 is
still labeled PASS3, as in the pre-remap table. Muted at 3:1 mirrors the dark
reference, where slate_500 on night_950 is 3.9:1.

| foreground | surface_canvas | surface_elevated | surface_raised | surface_hovered |
| --- | --- | --- | --- | --- |
| text_primary 0x4A443A | 7.07 PASS4.5 | 9.01 PASS4.5 | 9.48 PASS4.5 | 7.90 PASS4.5 |
| text_secondary 0x5C5448 | 5.47 PASS4.5 | 6.97 PASS4.5 | 7.33 PASS4.5 | 6.11 PASS4.5 |
| text_muted 0x6E6556 | 4.21 PASS3 | 5.36 PASS3 | 5.65 PASS3 | 4.70 PASS3 |
| text_faint 0x7A7161 | 3.53 PASS3 | 4.50 PASS3 | 4.74 PASS3 | 3.95 PASS3 |
| accent 0x2F74A0 | 3.73 PASS3 | 4.76 PASS3 | 5.01 PASS3 | 4.17 PASS3 |
| on_ink 0xFAF7F0 on accent | 4.76 PASS4.5 | - | - | - |

Supporting pairs: on_ink on ink rail 13.87, on_ink_muted on ink rail 5.36,
accent_ink on accent_fill 5.88.

### Corrections

The surface order here is the design study's: canvas -> elevated -> raised as
increasing elevation, canvas the darkest paper and the furthest surface from
the text color. The darkenings below are not from the study; the contrast math
forced them once canvas moved to bone_400. text_faint's darkening predates the
remap and is carried over from this spec's first pass.

- text_secondary, bone_800 0x6E6556 to bone_850 0x5C5448: on the new canvas
  bone_800 is 4.21:1, under the 4.5:1 body bar. Darkened to 5.47:1 on canvas,
  6.97 / 7.33 / 6.11 on elevated / raised / hovered (all PASS4.5).
- text_muted, bone_700 0x8C8270 to bone_800 0x6E6556: on the new canvas
  bone_700 is 2.78:1, under the 3:1 non-body bar (3.54 / 3.73 / 3.11 on the
  other surfaces). Darkened to 4.21:1 on canvas, 5.36 / 5.65 / 4.70 on
  elevated / raised / hovered (all PASS3, and PASS4.5 off canvas).
- text_faint, bone_600 0xA89F8D to bone_650 0x7A7161: bone_600 fails the 3:1
  bar on every surface (1.92 / 2.45 / 2.58 / 2.15 on canvas / elevated /
  raised / hovered). The interpolated bone_650 gives 3.53 / 4.50 / 4.74 / 3.95
  (all PASS3, PASS4.5 on elevated and raised). This correction was already in
  the spec before the remap; the darker canvas only widens the gap bone_600
  must clear.
- Semantics: on the new canvas, success 0x2E7D32 (3.76:1), danger 0xC62828
  (4.12:1), and warning 0x8F5C00 (4.16:1) all fall under the 4.5:1 body bar.
  Darkened, hue preserved, to success 0x296F2D (4.52:1), danger 0xBA2626
  (4.54:1), warning 0x885700 (4.52:1); each stays at least 6.0:1 against
  surface_raised, so none is near-black. info 0x1F5A82 already passes at
  5.42:1 on canvas and is untouched.

## Semantic colors on light ground

| role | hex | vs surface_canvas | vs surface_raised | verdict |
| --- | --- | --- | --- | --- |
| success | 0x296F2D | 4.52 | 6.07 | PASS4.5 |
| danger | 0xBA2626 | 4.54 | 6.09 | PASS4.5 |
| warning | 0x885700 | 4.52 | 6.06 | PASS4.5 |
| info | 0x1F5A82 | 5.42 | 7.27 | PASS4.5 |

The dark reference's green_400 / red_500 / blue_400 / amber_500 are all too
light for a paper ground and are not reused. The darkened roles scale each
channel by the same factor, so they stay recognizably green, red, and amber
while clearing 4.5:1 on canvas.

## Open questions

1. Preset key and label: spec assumes key "bone", label "Bone" in
TRAY_THEME_PRESETS; DEFAULT_TRAY_THEME_KEY stays "slate".
2. accent_key: bone's accent is a fixed palette color (0x2F74A0), so the preset
should carry accent_key "blue", matching the existing blue accent machinery in
the theming plan tests. Confirm it must not collide with user-selected accent
overrides.
3. Identity: MODERN_IDENTITY hardcodes dark-tuned values (frame_shadow
rgba(0,0,0,0.55), card_shadow 0.5 alpha, line_soft paper-rgb textures) that
read wrong on a light ground. Does bone reuse MODERN_IDENTITY or need its own
BONE_IDENTITY with light shadows and warm line treatments?
4. Undefined pairs: the study fixed only the ramp, accent family, on_ink
family, ink rail, and stage backdrop. scrim_rgb, tui screen, and tui card are
interpolated here and need review. Whether the TUI stays dark (ink) or should
go light (paper) is unresolved in the study.
5. Paper ground level: the study drew canvas at 0xE4DCCB, which makes canvas
the darkest paper and forces every text role and three semantic colors into
darkened variants (see Corrections). Should the paper ground be the lightest
step (bone_050 or bone_100) instead, recovering the original text roles?
