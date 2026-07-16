# Theme Identity Axis Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Themes change identity (typography, casing, radius, glow, surface treatment), not just palette; retro terminal becomes the slate/graphite/void identity, midnight goes modern per the approved mocks.

**Architecture:** `ThemeIdentity` is preset data in qol-theme, emitted as CSS tokens through the existing base-`:root` + per-theme diff-block pipeline. The UI consumes only tokens (plus one `identityKey` switch for monogram markup and minimap draw). Retro token values are chosen so slate/graphite/void render pixel-identical to today.

**Tech Stack:** Rust (qol-theme, qol-theme-css generator), CSS custom properties, Preact, node:test, Playwright live verification.

**Spec:** `docs/superpowers/specs/2026-07-16-theme-identity-design.md`. Mock target: `identity-mock-v6-minimap.png`.

## Global Constraints

- No code comments anywhere.
- Conventional one-line commits; pathspec-scoped (`git commit -m "..." -- <paths>`); never push.
- Regenerate ALL generator artifacts after qol-theme changes (`--profile tray-css|tray-js|plugin-lights|plugin-keyremap|alt-tab-cinnamon`); the `generated_artifacts_are_current` test enforces this.
- Retro themes must stay pixel-identical: every retro token value must equal the hardcoded value it replaces.
- Selection/focus indication must exist in every identity.
- After each task: `cargo test -p qol-theme`, UI suite via `mapfile -d '' t < <(find . -name '*.test.js' -print0) && node --test "${t[@]}"` (from `apps/qol-tray/ui`), `cargo fmt --check`, clippy clean.
- Full identity verification at the end runs on the live tray (recompile-self, Playwright).

---

### Task 1: ThemeIdentity model in qol-theme

**Files:**
- Modify: `libs/qol-theme/src/lib.rs`
- Test: `libs/qol-theme/tests/theme.rs`

**Interfaces:**
- Produces: `pub struct ThemeIdentity { pub key, pub font_ui, pub font_data, pub case_label, pub tracking_label, pub radius_2xs..radius_xl (u8, px), pub glow_text, pub frame_border, pub frame_texture, pub frame_bg, pub crt_band_display, pub card_border, pub card_bg, pub card_shadow, pub cover_bg, pub cover_texture, pub sel_outline, pub sel_outline_offset, pub ghost_btn_bg, pub ghost_btn_radius, pub hint_bg, pub hint_border, pub hint_shadow, pub panel_bg, pub panel_border, pub panel_radius, pub panel_shadow, pub heading_size, pub heading_weight, pub minimap_slab_radius }` — all `&'static str` except radii (`u8`); `RETRO_IDENTITY`, `MODERN_IDENTITY` consts; `TrayThemePreset.identity: &'static ThemeIdentity`.

- [ ] **Step 1: Add the struct and consts.** All retro values are string-literal copies of the CURRENT hardcoded CSS values (verify each against the style sheet named in Task 4-10 tables before writing it; the guard test in Task 3 re-checks). Modern values come from the v6 mock:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeIdentity {
    pub key: &'static str,
    pub font_ui: &'static str,
    pub font_data: &'static str,
    pub case_label: &'static str,
    pub tracking_label: &'static str,
    pub radius_2xs: u8,
    pub radius_xs: u8,
    pub radius_sm: u8,
    pub radius_chip: u8,
    pub radius_md: u8,
    pub radius_lg: u8,
    pub radius_lg_plus: u8,
    pub radius_xl: u8,
    pub glow_text: &'static str,
    pub frame_border: &'static str,
    pub frame_texture: &'static str,
    pub frame_bg: &'static str,
    pub crt_band_display: &'static str,
    pub card_border: &'static str,
    pub card_bg: &'static str,
    pub card_shadow: &'static str,
    pub cover_bg: &'static str,
    pub cover_texture: &'static str,
    pub sel_outline: &'static str,
    pub sel_outline_offset: &'static str,
    pub ghost_btn_bg: &'static str,
    pub ghost_btn_radius: &'static str,
    pub hint_bg: &'static str,
    pub hint_border: &'static str,
    pub hint_shadow: &'static str,
    pub panel_bg: &'static str,
    pub panel_border: &'static str,
    pub panel_radius: &'static str,
    pub panel_shadow: &'static str,
    pub heading_size: &'static str,
    pub heading_weight: &'static str,
    pub minimap_slab_radius: u8,
}

pub const RETRO_IDENTITY: ThemeIdentity = ThemeIdentity {
    key: "retro",
    font_ui: "var(--font-mono)",
    font_data: "var(--font-mono)",
    case_label: "uppercase",
    tracking_label: "var(--ls-md)",
    radius_2xs: 2,
    radius_xs: 3,
    radius_sm: 4,
    radius_chip: 5,
    radius_md: 6,
    radius_lg: 8,
    radius_lg_plus: 10,
    radius_xl: 12,
    glow_text: "var(--tui-glow-text)",
    frame_border: "var(--border-w-3) double var(--tui-line)",
    frame_texture: "var(--tui-scanline)",
    frame_bg: "var(--tui-bg-screen)",
    crt_band_display: "block",
    card_border: "var(--border-w-1) solid var(--tui-line-soft)",
    card_bg: "linear-gradient(180deg, rgba(var(--accent-rgb), 0.05), transparent 55%), var(--tui-bg-card)",
    card_shadow: "inset 0 1px 0 rgba(var(--accent-rgb), 0.08), 0 6px 18px var(--layer-ink-45)",
    cover_bg: "var(--tui-screen-bg)",
    cover_texture: "var(--tui-scanline)",
    sel_outline: "none",
    sel_outline_offset: "0px",
    ghost_btn_bg: "transparent",
    ghost_btn_radius: "var(--radius-sm)",
    hint_bg: "transparent",
    hint_border: "none",
    hint_shadow: "none",
    panel_bg: "var(--tui-bg-panel)",
    panel_border: "var(--border-w-1) solid var(--tui-line)",
    panel_radius: "var(--radius-md)",
    panel_shadow: "none",
    heading_size: "var(--fs-md)",
    heading_weight: "var(--fw-bold)",
    minimap_slab_radius: 3,
};

pub const MODERN_IDENTITY: ThemeIdentity = ThemeIdentity {
    key: "modern",
    font_ui: "var(--font-sans)",
    font_data: "var(--font-mono)",
    case_label: "none",
    tracking_label: "normal",
    radius_2xs: 6,
    radius_xs: 7,
    radius_sm: 8,
    radius_chip: 9,
    radius_md: 10,
    radius_lg: 13,
    radius_lg_plus: 15,
    radius_xl: 18,
    glow_text: "none",
    frame_border: "none",
    frame_texture: "none",
    frame_bg: "transparent",
    crt_band_display: "none",
    card_border: "none",
    card_bg: "var(--qol-system-surface-elevated)",
    card_shadow: "0 14px 36px rgba(0, 0, 0, 0.5), 0 2px 6px rgba(0, 0, 0, 0.35)",
    cover_bg: "transparent",
    cover_texture: "none",
    sel_outline: "2px solid rgba(var(--accent-rgb), 0.85)",
    sel_outline_offset: "3px",
    ghost_btn_bg: "rgba(var(--accent-rgb), 0.14)",
    ghost_btn_radius: "10px",
    hint_bg: "rgba(var(--qol-system-overlay-surface-rgb), 0.92)",
    hint_border: "1px solid rgba(var(--accent-rgb), 0.25)",
    hint_shadow: "0 6px 18px rgba(0, 0, 0, 0.4)",
    panel_bg: "rgba(var(--qol-system-overlay-surface-rgb), 0.95)",
    panel_border: "none",
    panel_radius: "16px",
    panel_shadow: "0 14px 36px rgba(0, 0, 0, 0.5)",
    heading_size: "1.55rem",
    heading_weight: "700",
    minimap_slab_radius: 8,
};
```

NOTE for the implementer: `sel_outline: "none"` for retro is correct because retro keeps its existing border+glow selection styling untouched; the outline token is additive and only modern uses it. Cross-check retro's `card_bg`/`card_shadow`/`frame_*` strings character-for-character against `common-plugin-cards.css:1-9` and `world.css:63-70` before committing.

- [ ] **Step 2: Add `identity` to `TrayThemePreset` and all four presets.** `pub identity: &'static ThemeIdentity,` after `accent_key`. slate/graphite/void get `identity: &RETRO_IDENTITY`, midnight gets `identity: &MODERN_IDENTITY`.

- [ ] **Step 3: Test.** In `libs/qol-theme/tests/theme.rs`:

```rust
#[test]
fn tray_theme_identities_are_assigned() {
    for preset in qol_theme::tray_theme_presets() {
        let expected = if preset.key == "midnight" { "modern" } else { "retro" };
        assert_eq!(preset.identity.key, expected, "{}", preset.key);
    }
    assert_eq!(qol_theme::RETRO_IDENTITY.font_data, qol_theme::RETRO_IDENTITY.font_ui);
    assert_eq!(qol_theme::MODERN_IDENTITY.font_data, "var(--font-mono)");
}
```

- [ ] **Step 4: Run** `cargo test -p qol-theme` — expect only `generated_artifacts_are_current` may fail (regen happens in Task 2; if it fails here, regenerate all five profiles now).
- [ ] **Step 5: Commit** `feat(qol-theme): model theme identity as preset data` — `libs/qol-theme/src/lib.rs libs/qol-theme/tests/theme.rs` (+ regenerated artifacts if step 4 required them).

---

### Task 2: Identity token emission

**Files:**
- Modify: `libs/qol-theme/src/css.rs`, `libs/qol-theme/tests/theme.rs`
- Regenerate: all five artifacts

**Interfaces:**
- Produces CSS tokens (base `:root` + per-theme diffs): `--font-ui`, `--font-data`, `--case-label`, `--tracking-label`, `--radius-2xs..--radius-xl`, `--glow-text`, `--frame-border`, `--frame-texture`, `--frame-bg`, `--crt-band-display`, `--card-border`, `--card-bg`, `--card-shadow`, `--cover-bg`, `--cover-texture`, `--sel-outline`, `--sel-outline-offset`, `--ghost-btn-bg`, `--ghost-btn-radius`, `--hint-bg`, `--hint-border`, `--hint-shadow`, `--panel-bg`, `--panel-border`, `--panel-radius`, `--panel-shadow`, `--heading-size`, `--heading-weight`, `--minimap-slab-radius`.
- Produces JS: `QOL_THEMES` entries gain `identityKey: "retro"|"modern"`; boot `ThemeEntry` mirrors it (Task 2 also touches `apps/qol-tray/src/features/plugin_store/server/boot.rs`).

- [ ] **Step 1:** In `tray_theme_base_css`, after the tui pushes, append every identity token via a `push_identity(&mut out, preset.identity)` helper that writes `    --font-ui: {};\n` style lines (radii as `{}px`). The diff pipeline in `tray_css()` then emits midnight's identity block automatically.
- [ ] **Step 2:** Remove the now-duplicated static `--radius-*` definitions from `apps/qol-tray/ui/styles/theme-tokens.css` (lines 179-186) so the generated tokens are the single source. Keep `--font-sans`/`--font-mono` raw stacks in theme-tokens.css (identities reference them).
- [ ] **Step 3:** In `tray_theme_js()`, extend the entry line to `{ key, label, accentKey, identityKey }`.
- [ ] **Step 4:** In `boot.rs`, add `identity_key` (serde rename `identityKey`) to `ThemeEntry`, sourced from `preset.identity.key`; extend `boot_json_carries_theme_selection_and_palette` with `assert_eq!(themes[0]["identityKey"], "retro");`.
- [ ] **Step 5: Tests** in `theme.rs`: extend `tray_theme_js_emits_theme_metadata` for `identityKey: "retro"`/`"modern"`; new test:

```rust
#[test]
fn tray_css_emits_identity_tokens_per_theme() {
    let css = css::tray_css();
    let base = css.split(":root[data-qol-theme").next().unwrap();
    assert!(base.contains("--font-ui: var(--font-mono);"));
    assert!(base.contains("--radius-md: 6px;"));
    let midnight = css.split(":root[data-qol-theme=\"midnight\"]").nth(1).unwrap();
    let midnight = midnight.split('}').next().unwrap();
    assert!(midnight.contains("--font-ui: var(--font-sans);"));
    assert!(midnight.contains("--radius-md: 10px;"));
    assert!(midnight.contains("--crt-band-display: none;"));
    let graphite = css.split(":root[data-qol-theme=\"graphite\"]").nth(1).unwrap();
    let graphite = graphite.split('}').next().unwrap();
    assert!(!graphite.contains("--font-ui"), "retro themes emit no identity diff");
}
```

- [ ] **Step 6:** Regenerate all five artifacts; run `cargo test -p qol-theme` (green) and the UI suite (green; nothing consumes the tokens yet).
- [ ] **Step 7: Commit** `feat(qol-theme): emit identity tokens and metadata per theme`.

---

### Task 3: Retro no-change guard

**Files:**
- Test: `libs/qol-theme/tests/theme.rs`

- [ ] **Step 1:** Add a table-driven test pinning retro token values to the exact strings the sweeps will remove from the style sheets:

```rust
#[test]
fn retro_identity_matches_legacy_hardcoded_values() {
    let cases = [
        ("--case-label", "uppercase"),
        ("--tracking-label", "var(--ls-md)"),
        ("--font-ui", "var(--font-mono)"),
        ("--radius-xs", "3px"),
        ("--frame-border", "var(--border-w-3) double var(--tui-line)"),
        ("--frame-texture", "var(--tui-scanline)"),
        ("--card-border", "var(--border-w-1) solid var(--tui-line-soft)"),
        ("--cover-bg", "var(--tui-screen-bg)"),
        ("--minimap-slab-radius", "3"),
    ];
    let css = css::tray_css();
    let base = css.split(":root[data-qol-theme").next().unwrap();
    for (name, value) in cases {
        assert!(base.contains(&format!("{name}: {value};")), "{name}");
    }
}
```

- [ ] **Step 2:** Run, commit `test(qol-theme): pin retro identity to legacy values`.

---

### Task 4: Casing sweep

**Files (31 sites):** every `text-transform: uppercase` below becomes `text-transform: var(--case-label);` and its adjacent `letter-spacing` line (when present in the same rule) becomes `letter-spacing: var(--tracking-label);`:

`page-header.css:43`, `common-settings.css:80`, `common-controls.css:92,232,614,742,784`, `common-config-primitives.css:202`, `app-shell.css:25` (`.tui-label`), `common-plugin-cards.css:115,202,343,456`, `logs.css:251`, `searchable-action-list.css:230`, `profile-conflicts.css:149,197`, `table.css:19`, `dev-plugin-list.css:74,476`, `world.css:272,646`, `plugin-config.css:286,749,984,1059,1517`, `dev-layout.css:50,150,176`, `profile.css:276`.

- [ ] **Step 1:** Apply the substitution at all sites (line numbers are pre-sweep; re-grep `text-transform: uppercase` after editing — the count must reach 0 in `apps/qol-tray/ui/styles`).
- [ ] **Step 2:** Rules whose letter-spacing is intentionally NOT label-tracking (e.g. `.plugin-cover-monogram`) keep their own value; only pair the swap when the rule's letter-spacing is `var(--ls-md)` or `var(--ls-sm)` on an uppercase label.
- [ ] **Step 3:** UI suite green; visual spot-check via Playwright on slate: labels still render uppercase.
- [ ] **Step 4: Commit** `refactor(qol-tray): route label casing through identity tokens`.

---

### Task 5: Font split sweep

Replace `var(--font-mono)` per this classification (complete inventory):

**→ `var(--font-data)`** (codes, keys, paths, versions, numeric readouts, logs, diffs):
`auto-config-page.css` `.color-hex` `.slider-val`; `common-config-primitives.css` `.mod-chip-static` `.mod-chip` `.key-chip` `.key-label` `.key-input`; `common-controls.css` `.code-block` `.btn kbd`; `common-plugin-cards.css` `.plugins-grid .plugin-card .version` `.plugin-version`; `common-settings.css` `.key-input-row input` `kbd`; `dev-gpui-subpage.css` `.gpui-setting-color`; `dev-plugin-list.css` `.plugin-row .plugin-path` `.plugin-build-overlay-sub` `.plugin-row .plugin-cpu-strip-value` `.link-input-row input`; `list-row.css` `.list-row-mono`; `logs.css` all six (`.log-src` `.log-loc` `.suppressed-key` `.suppressed-msg` `.suppressed-detail-value.mono` `.suppressed-version`); `plugin-config.css` all gamepad selectors (`.gamepad-connection output` `.gamepad-signal-history-heading strong` `.gamepad-rumble-heading output` `.gamepad-vector-value` `.gamepad-vector-device-control text` `.gamepad-vector-auxiliary-overflow` `.gamepad-vector-auxiliary text` `.gamepad-active-inputs strong` `.gamepad-axis output` `.gamepad-button-chip output`); `profile-conflicts.css` `.profile-conflicts-plugin` `.profile-conflicts-key` `.profile-conflicts-file` `.profile-conflicts-field-row` `.profile-conflicts-side-value` `.profile-conflicts-diff` `.profile-conflicts-kbd kbd` `.profile-conflicts-confirm-value` `.profile-conflicts-note code`; `profile.css` `.profile-settings-value` `.profile-settings-link` `.profile-device-code` `.profile-device-uri` `.profile-result-id` `.profile-backup-file`; `searchable-action-list.css` `.searchable-action-list-count`; `world.css` `.world-minimap-container > .world-minimap-depth`.

**→ `var(--font-ui)`** (interface text): `action-menu.css` `.action-menu-trigger` `.action-menu-item`; `app-shell.css` `.tui-label` `.wt-picker-option`; `common-controls.css` `.btn` `.search-bar:not(.palette-hint)::before` `.search-input` `.palette-titlebar` `.palette-filter-pill`; `common-config-primitives.css` `.global-badge`; `common-plugin-cards.css` `.plugin-cover-monogram` `.plugin-name-text`; `dev-layout.css` `.dev-view-shell .page-header-main h1` `.catalog-input`; `plugin-config.css` `.custom-select-option`; `profile-conflicts.css` `.profile-conflicts-diff-title`; `searchable-action-list.css` `.searchable-action-list-search-mark` `.searchable-action-list-action`; `selection-wedge.css` `.selection-cursor-overlay > .selection-wedge-depth`; `table.css` `.table-list-header .table-cell`; `world.css` `.wsp-pill` `.world-region-label`.

- [ ] **Step 1:** Apply; re-grep `var(--font-mono)` in `apps/qol-tray/ui/styles` — remaining hits must be only the raw-stack definition in `theme-tokens.css` and `--font-data`/`--font-ui` retro values coming from generated CSS.
- [ ] **Step 2:** UI suite green; Playwright spot-check slate unchanged, midnight body text renders sans.
- [ ] **Step 3: Commit** `refactor(qol-tray): split ui and data fonts through identity tokens`.

---

### Task 6: Frame, glow, and CRT tokenization

**Files:** `apps/qol-tray/ui/styles/world.css`

- [ ] **Step 1:** `.view-container.content-shell`: `border: var(--frame-border); background-color: var(--frame-bg); background-image: var(--frame-texture);`
- [ ] **Step 2:** `.view-container.content-shell::before` (CRT band) gains `display: var(--crt-band-display);`
- [ ] **Step 3:** Every `text-shadow: var(--tui-glow-text)` usage across `apps/qol-tray/ui/styles` becomes `text-shadow: var(--glow-text)` (grep `tui-glow-text`; the variable itself stays defined for retro's token value).
- [ ] **Step 4:** UI suite + slate visual check (double border, scanline, band animation intact); midnight check (frameless).
- [ ] **Step 5: Commit** `refactor(qol-tray): tokenize page frame and glow per identity`.

---

### Task 7: Cards, covers, monogram tile

**Files:** `apps/qol-tray/ui/styles/common-plugin-cards.css`, `apps/qol-tray/ui/views/plugins/grid.js`, `apps/qol-tray/ui/lib/theme-presets.js`, test `apps/qol-tray/ui/views/plugins/grid.test.js` (or nearest existing grid test file)

- [ ] **Step 1:** Card rule block (`common-plugin-cards.css:1-9`): `background: var(--card-bg); border: var(--card-border); border-radius: var(--radius-md); box-shadow: var(--card-shadow);`
- [ ] **Step 2:** Selected card: append `outline: var(--sel-outline); outline-offset: var(--sel-outline-offset);` to the existing `[data-selected="true"]` rule (retro emits `none`, preserving today's look).
- [ ] **Step 3:** `.plugin-cover-placeholder`: `background: var(--cover-bg);` and `::after` `background: var(--cover-texture);`
- [ ] **Step 4:** Monogram: `theme-presets.js` exports `themeIdentityKey(key)` (mirrors `themeAccentKey`); `grid.js` adds `data-identity` from it on the placeholder, and CSS defines the icon-tile variant:

```css
[data-identity="modern"] .plugin-cover-monogram {
    width: 64px;
    height: 64px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 1.5rem;
    letter-spacing: normal;
    color: var(--qol-system-paper, #ffffff);
    background: linear-gradient(135deg, rgb(var(--accent-rgb)), rgba(var(--accent-rgb), 0.55));
    border-radius: var(--radius-xl);
    box-shadow: 0 6px 16px rgba(0, 0, 0, 0.4);
    text-shadow: none;
}
```

(`data-identity` re-resolves on theme switch via the existing theme subscription that re-renders the grid; verify with Playwright, and if the attribute is stale after a live switch, set it in `applyTheme` on `document.documentElement` instead and key the CSS off `:root[data-qol-identity="modern"]` — pick ONE mechanism and delete the other.)

- [ ] **Step 5:** UI suite + slate/midnight visual check; commit `feat(qol-tray): identity-driven card and cover treatment`.

---

### Task 8: Chrome details (hint pill, ghost buttons, badges)

**Files:** `apps/qol-tray/ui/styles/common-controls.css` (palette hint), `common-plugin-cards.css` (`.plugin-cog`, version badge), `components/CommandPalette.js`

- [ ] **Step 1:** `.search-bar.palette-hint`: `background: var(--hint-bg); border: var(--hint-border); border-radius: var(--radius-lg); box-shadow: var(--hint-shadow);`
- [ ] **Step 2:** CommandPalette hint markup becomes `<kbd class="palette-hint-kbd">Ctrl E</kbd><span class="palette-hint-text">to search & run actions</span>`; `.palette-hint-kbd { font-family: var(--font-data); background: rgba(var(--accent-rgb), 0.22); border-radius: var(--radius-md); padding: 2px 7px; }` — in retro the pill background token is transparent, so the kbd chip must also read acceptably there (visual check; if it clashes on retro, gate the chip's background on `--hint-bg` being non-transparent via a dedicated `--hint-kbd-bg` identity token added in Task 1 — decide by looking, then keep exactly one approach).
- [ ] **Step 3:** `.plugin-cog`: `background: var(--ghost-btn-bg); border-radius: var(--ghost-btn-radius);` keeping current size/rest of rule.
- [ ] **Step 4:** UI suite; Playwright: retro hint identical to before, modern hint matches mock; commit `feat(qol-tray): identity-driven chrome details`.

---

### Task 9: Peripherals and minimap

**Files:** `apps/qol-tray/ui/styles/world.css`, `apps/qol-tray/ui/lib/minimap-draw.js`, test `apps/qol-tray/ui/lib/minimap-draw.test.js` if present

- [ ] **Step 1:** `.world-minimap-container` and `.world-cog-btn` rules consume `var(--panel-bg) / var(--panel-border) / var(--panel-radius) / var(--panel-shadow)`.
- [ ] **Step 2:** `minimap-draw.js`: replace `const RADIUS = 3` with a parameter threaded from the caller; `Minimap.js` reads it once per draw via `getComputedStyle(document.documentElement).getPropertyValue('--minimap-slab-radius')` (parse int, fallback 3).
- [ ] **Step 3:** UI suite (update any minimap-draw tests for the new parameter); slate visual identical; midnight rounded panel + slabs.
- [ ] **Step 4: Commit** `feat(qol-tray): identity-driven peripherals and minimap radius`.

---

### Task 10: Heading presence

**Files:** `apps/qol-tray/ui/styles/world.css` (`.world-region-label`)

- [ ] **Step 1:** `font-size: var(--heading-size); font-weight: var(--heading-weight);` on `.world-region-label` (casing/tracking already tokenized by Task 4; retro values pin today's look).
- [ ] **Step 2:** Also drop the label's boxed border/background under modern: `border: var(--panel-border); background: transparent;` is WRONG if retro relies on the current chip styling — instead give the rule `border: var(--hint-border)`-style dedicated tokens ONLY if the current chip has a border (check the rule first; if the current label has border/background, add `--heading-border`/`--heading-bg` identity tokens in the same pattern as Task 1, retro = current values, modern = `none`/`transparent`).
- [ ] **Step 3:** UI suite; both identities visually verified; commit `feat(qol-tray): identity-driven page heading presence`.

---

### Task 11: End-to-end verification

- [ ] **Step 1:** Full gates: `cargo test -p qol-theme`, `cargo test -p qol-tray --features dev`, UI suite, `cargo fmt --check`, clippy with `--features dev --all-targets`.
- [ ] **Step 2:** POST `/api/dev/recompile-self`; wait until served CSS contains `--crt-band-display`.
- [ ] **Step 3:** Playwright: for slate then midnight — plugins page, a plugin config page (`#plugins/qol-shot/config`), settings panel open. Six screenshots at repo root (`identity-final-<theme>-<page>.png`). Slate must be visually indistinguishable from pre-branch screenshots; midnight must match the v6 mock direction.
- [ ] **Step 4:** Verify live theme switch re-skins identity without reload (fonts, radius, cover tiles) and auto-accent still re-tints.
- [ ] **Step 5:** Restore theme selection to auto; present screenshots to the user for the style verdict.
