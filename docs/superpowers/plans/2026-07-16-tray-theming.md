# Tray Theming and Component Consolidation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task (user requires inline execution, no subagents). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Switchable full dark theme palettes for the qol-tray web UI, every view built from gallery components with a CI guard, and a global depth/typography/density pass.

**Architecture:** Theme presets live as data in `libs/qol-theme`; the `qol-theme-css` generator emits a default `:root` block plus one `:root[data-qol-theme="<key>"]` diff block per extra theme (mirroring the existing `data-qol-accent` diff machinery), and `QOL_THEMES` metadata into the tray JS tokens.
The tray persists the selection in `theme.json` beside `accent`, injects it via `__QOL_BOOT__`, and sets the attribute before first paint.
Views consume only semantic tokens, enforced by migrating the 3 remaining `--slate-*` consumers and by a stray-interactive-element guard test.

**Tech Stack:** Rust (qol-theme, axum handlers), Preact + htm (no build step), node --test, cargo test.

## Global Constraints

- No code comments anywhere.
- Conventional one-line commits, no AI attribution, scope must be a workspace member (`qol-theme`, `qol-tray`) or umbrella.
- Every commit compiles and passes the gates it touches; run commands from the repo root unless stated.
- Em-dash character is banned in all file content.
- Accent remains an independent axis: theme blocks must not defeat the inline `--accent-rgb` override (inline style always wins; never emit accent values in theme diff blocks).
- Dark variants only; the abstraction must allow a light theme later without rework.
- Generated files are never hand-edited; regenerate with `qol-theme-css --write`.
- UI tests: `cd apps/qol-tray/ui && mapfile -d '' t < <(find . -name '*.test.js' -print0) && node --test "${t[@]}"`.
- Rust gates: `cargo test -p qol-theme`, `cargo test -p qol-tray --features dev`, `cargo fmt --all -- --check`, `cargo clippy -p qol-theme -p qol-tray -- -D warnings`.

---

### Task 1: Theme presets and contrast tests in qol-theme

**Files:**
- Modify: `libs/qol-theme/src/lib.rs` (after `DARK_SYSTEM`, line ~209)
- Test: `libs/qol-theme/tests/theme.rs`

**Interfaces:**
- Produces: `OverlayPalette`, `TrayThemePreset { key, label, system: SystemPalette, overlay: OverlayPalette }`, `DEFAULT_TRAY_THEME_KEY: &str = "slate"`, `tray_theme_presets() -> &'static [TrayThemePreset]`, `tray_theme_preset(key: &str) -> Option<TrayThemePreset>`.

- [ ] **Step 1: Write the failing tests** (append to `libs/qol-theme/tests/theme.rs`)

```rust
fn relative_luminance(rgb: u32) -> f64 {
    let channel = |c: u32| {
        let c = c as f64 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let r = channel((rgb >> 16) & 0xff);
    let g = channel((rgb >> 8) & 0xff);
    let b = channel(rgb & 0xff);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn contrast_ratio(a: u32, b: u32) -> f64 {
    let (hi, lo) = {
        let (la, lb) = (relative_luminance(a), relative_luminance(b));
        if la > lb { (la, lb) } else { (lb, la) }
    };
    (hi + 0.05) / (lo + 0.05)
}

#[test]
fn tray_theme_presets_have_unique_keys_and_a_default() {
    let presets = qol_theme::tray_theme_presets();
    let mut keys: Vec<_> = presets.iter().map(|p| p.key).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), presets.len(), "duplicate theme keys");
    assert!(qol_theme::tray_theme_preset(qol_theme::DEFAULT_TRAY_THEME_KEY).is_some());
    assert!(qol_theme::tray_theme_preset("nope").is_none());
}

#[test]
fn tray_theme_palettes_hold_contrast_floors() {
    for preset in qol_theme::tray_theme_presets() {
        let s = preset.system;
        let surfaces = [
            ("canvas", s.surface_canvas),
            ("elevated", s.surface_elevated),
            ("raised", s.surface_raised),
            ("hovered", s.surface_hovered),
        ];
        for (name, surface) in surfaces {
            let cases = [
                ("text_primary", s.text_primary, 6.5),
                ("text_secondary", s.text_secondary, 4.0),
                ("text_muted", s.text_muted, 2.4),
            ];
            for (text_name, text, floor) in cases {
                let ratio = contrast_ratio(text, surface);
                assert!(
                    ratio >= floor,
                    "{}: {text_name} on {name} = {ratio:.2}, floor {floor}",
                    preset.key
                );
            }
        }
        for pair in surfaces.windows(2) {
            let ratio = contrast_ratio(pair[0].1, pair[1].1);
            assert!(
                ratio >= 1.08,
                "{}: surfaces {} vs {} too close ({ratio:.3})",
                preset.key, pair[0].0, pair[1].0
            );
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p qol-theme --test theme tray_theme -- --nocapture`
Expected: compile error, `tray_theme_presets` not found.

- [ ] **Step 3: Implement presets** (in `libs/qol-theme/src/lib.rs`, after `DARK_SYSTEM`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayPalette {
    pub surface_rgb: u32,
    pub deep_rgb: u32,
    pub ink_rgb: u32,
    pub scrim_rgb: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrayThemePreset {
    pub key: &'static str,
    pub label: &'static str,
    pub system: SystemPalette,
    pub overlay: OverlayPalette,
}

pub const DEFAULT_TRAY_THEME_KEY: &str = "slate";

pub const TRAY_THEME_PRESETS: [TrayThemePreset; 3] = [
    TrayThemePreset {
        key: "slate",
        label: "Slate",
        system: DARK_SYSTEM,
        overlay: OverlayPalette {
            surface_rgb: 0x12161e,
            deep_rgb: 0x111419,
            ink_rgb: 0x070a0e,
            scrim_rgb: 0x040508,
        },
    },
    TrayThemePreset {
        key: "graphite",
        label: "Graphite",
        system: SystemPalette {
            surface_canvas: 0x121110,
            surface_elevated: 0x1a1816,
            surface_raised: 0x201d1a,
            surface_hovered: 0x282420,
            text_primary: 0xf2ede4,
            text_secondary: 0xcfc7b8,
            text_muted: 0x92876f,
            text_faint: 0x6b6254,
            border_subtle: 0x3a352c,
            accent: DARK_REFERENCE.orange_400,
            success: DARK_REFERENCE.green_400,
            danger: DARK_REFERENCE.red_500,
            info: DARK_REFERENCE.blue_400,
            warning: DARK_REFERENCE.amber_500,
        },
        overlay: OverlayPalette {
            surface_rgb: 0x1e1b17,
            deep_rgb: 0x191613,
            ink_rgb: 0x0c0a07,
            scrim_rgb: 0x080604,
        },
    },
    TrayThemePreset {
        key: "void",
        label: "Void",
        system: SystemPalette {
            surface_canvas: 0x000000,
            surface_elevated: 0x0a0c10,
            surface_raised: 0x10131a,
            surface_hovered: 0x181c26,
            text_primary: 0xeef2fa,
            text_secondary: 0xb6becf,
            text_muted: 0x76829c,
            text_faint: 0x4f5a72,
            border_subtle: 0x272e3c,
            accent: DARK_REFERENCE.orange_400,
            success: DARK_REFERENCE.green_400,
            danger: DARK_REFERENCE.red_500,
            info: DARK_REFERENCE.blue_400,
            warning: DARK_REFERENCE.amber_500,
        },
        overlay: OverlayPalette {
            surface_rgb: 0x0d1017,
            deep_rgb: 0x090b10,
            ink_rgb: 0x04060a,
            scrim_rgb: 0x020304,
        },
    },
];

pub fn tray_theme_presets() -> &'static [TrayThemePreset] {
    &TRAY_THEME_PRESETS
}

pub fn tray_theme_preset(key: &str) -> Option<TrayThemePreset> {
    TRAY_THEME_PRESETS.iter().copied().find(|preset| preset.key == key)
}
```

- [ ] **Step 4: Run tests until floors pass**

Run: `cargo test -p qol-theme --test theme tray_theme`
Expected: PASS.
If a floor fails for `slate`, the floor is wrong (slate is today's shipped palette): lower that floor to just under slate's actual ratio and note the value in the assert message.
If a floor fails for `graphite`/`void`, adjust that palette's failing color (lighten text or darken surface), not the floor.

- [ ] **Step 5: Gates and commit**

Run: `cargo fmt --all && cargo clippy -p qol-theme -- -D warnings && cargo test -p qol-theme`

```bash
git add libs/qol-theme/src/lib.rs libs/qol-theme/tests/theme.rs
git commit -m "feat(qol-theme): add tray theme presets with contrast floors"
```

---

### Task 2: Generator emits theme blocks and metadata

**Files:**
- Modify: `libs/qol-theme/src/css.rs` (`tray_css` line 17, `tray_theme_js` line 57, `tray_variables` line 256)
- Modify: `apps/qol-tray/ui/styles/theme-tokens.css` (overlay/scrim lines 48-51)
- Regenerate: `apps/qol-tray/ui/styles/generated-theme-tokens.css`, `apps/qol-tray/ui/lib/generated-theme-tokens.js`
- Test: `libs/qol-theme/tests/theme.rs`

**Interfaces:**
- Consumes: `tray_theme_presets()`, `TrayThemePreset`, `DEFAULT_TRAY_THEME_KEY` from Task 1.
- Produces: CSS vars `--qol-system-overlay-surface-rgb`, `--qol-system-overlay-deep-rgb`, `--qol-system-overlay-ink-rgb`, `--qol-system-scrim-rgb` on `:root`; `:root[data-qol-theme="graphite"]` and `:root[data-qol-theme="void"]` diff blocks; JS exports `QOL_THEMES` (array of `{ key, label }`) and `QOL_DEFAULT_THEME`.

- [ ] **Step 1: Write the failing tests** (append to `libs/qol-theme/tests/theme.rs`)

```rust
#[test]
fn tray_css_emits_theme_override_blocks() {
    let css = css::tray_css();
    assert!(css.contains("--qol-system-overlay-surface-rgb: 18, 22, 30;"));
    assert!(css.contains("--qol-system-scrim-rgb: 4, 5, 8;"));
    assert!(!css.contains(":root[data-qol-theme=\"slate\"]"), "default theme needs no block");
    for key in ["graphite", "void"] {
        let marker = format!(":root[data-qol-theme=\"{key}\"]");
        assert!(css.contains(&marker), "missing {marker}");
    }
    let void_block = css.split(":root[data-qol-theme=\"void\"]").nth(1).unwrap();
    let void_block = void_block.split('}').next().unwrap();
    assert!(void_block.contains("--qol-system-surface-canvas: #000000;"));
    assert!(!void_block.contains("--qol-system-accent-rgb"), "themes must not override accent");
}

#[test]
fn tray_theme_js_emits_theme_metadata() {
    let js = css::tray_theme_js();
    assert!(js.contains("export const QOL_THEMES = ["));
    assert!(js.contains("{ key: \"slate\", label: \"Slate\" },"));
    assert!(js.contains("{ key: \"graphite\", label: \"Graphite\" },"));
    assert!(js.contains("{ key: \"void\", label: \"Void\" },"));
    assert!(js.contains("export const QOL_DEFAULT_THEME = \"slate\";"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p qol-theme --test theme tray_css_emits_theme -- --nocapture`
Expected: FAIL (no overlay vars, no blocks).

- [ ] **Step 3: Implement generator changes** (in `libs/qol-theme/src/css.rs`)

Add imports `tray_theme_presets, DEFAULT_TRAY_THEME_KEY, TrayThemePreset` to the `use crate::{...}` list, then:

```rust
pub fn tray_css() -> String {
    let default = tray_default_theme_preset();
    let mut out = String::from("/* @generated by qol-theme-css; do not edit by hand. */\n");
    out.push_str(&tray_theme_base_css(":root", default));
    out.push_str(&tray_variables(":root"));
    let base_variables = css_variable_map(&out);
    for preset in tray_theme_presets() {
        if preset.key == DEFAULT_TRAY_THEME_KEY {
            continue;
        }
        let variant = tray_theme_base_css(":root", *preset);
        let diff: Vec<_> = css_variable_diff(&base_variables, &variant)
            .into_iter()
            .filter(|(name, _)| !name.contains("accent"))
            .collect();
        if diff.is_empty() {
            continue;
        }
        let _ = writeln!(out, ":root[data-qol-theme=\"{}\"] {{", preset.key);
        for (name, value) in diff {
            let _ = writeln!(out, "    {name}: {value};");
        }
        out.push_str("}\n");
    }
    out
}

fn tray_default_theme_preset() -> TrayThemePreset {
    crate::tray_theme_preset(DEFAULT_TRAY_THEME_KEY).expect("default tray theme exists")
}

fn tray_theme_base_css(selector: &str, preset: TrayThemePreset) -> String {
    let theme = Theme::from_reference_and_system(crate::ThemeMode::Dark, DARK_REFERENCE, preset.system);
    let mut out = css_variables(selector, theme);
    out.truncate(out.rfind('}').expect("closing brace"));
    push_rgb(&mut out, "qol-system-overlay-surface-rgb", preset.overlay.surface_rgb);
    push_rgb(&mut out, "qol-system-overlay-deep-rgb", preset.overlay.deep_rgb);
    push_rgb(&mut out, "qol-system-overlay-ink-rgb", preset.overlay.ink_rgb);
    push_rgb(&mut out, "qol-system-scrim-rgb", preset.overlay.scrim_rgb);
    out.push_str("}\n");
    out
}
```

Note `tray_css` no longer calls `dark_css()`; the core profile (`dark_css`) is untouched and plugins keep consuming `dark_theme()`.
In `tray_theme_js()`, before the final `out`, append:

```rust
    out.push_str("export const QOL_THEMES = [\n");
    for preset in crate::tray_theme_presets() {
        let _ = writeln!(out, "    {{ key: \"{}\", label: \"{}\" }},", preset.key, preset.label);
    }
    out.push_str("];\n");
    let _ = writeln!(out, "export const QOL_DEFAULT_THEME = \"{}\";", crate::DEFAULT_TRAY_THEME_KEY);
```

- [ ] **Step 4: Run the new tests, then the full qol-theme suite**

Run: `cargo test -p qol-theme`
Expected: the two new tests PASS; the stale-check test FAILS naming the two tray files (that is the regeneration signal).
`tray_css_layers_tray_tokens_without_polluting_core` may also fail if it asserts exact layering; update its expectations to the new structure (base block now includes overlay vars).

- [ ] **Step 5: Regenerate and rewire the semantic aliases**

```bash
cargo run -q -p qol-theme --bin qol-theme-css -- --profile tray-css --write apps/qol-tray/ui/styles/generated-theme-tokens.css
cargo run -q -p qol-theme --bin qol-theme-css -- --profile tray-js --write apps/qol-tray/ui/lib/generated-theme-tokens.js
```

In `apps/qol-tray/ui/styles/theme-tokens.css` replace lines 48-51:

```css
    --overlay-surface-rgb: var(--qol-system-overlay-surface-rgb);
    --overlay-deep-rgb: var(--qol-system-overlay-deep-rgb);
    --overlay-ink-rgb: var(--qol-system-overlay-ink-rgb);
    --scrim-rgb: var(--qol-system-scrim-rgb);
```

- [ ] **Step 6: Gates and commit**

Run: `cargo test -p qol-theme && cargo fmt --all -- --check && cargo clippy -p qol-theme -- -D warnings`
Expected: all PASS including stale-checks.
Manual smoke: open the tray UI, run `document.documentElement.setAttribute('data-qol-theme','void')` in devtools, confirm surfaces go black.

```bash
git add libs/qol-theme/src/css.rs libs/qol-theme/tests/theme.rs apps/qol-tray/ui/styles/generated-theme-tokens.css apps/qol-tray/ui/lib/generated-theme-tokens.js apps/qol-tray/ui/styles/theme-tokens.css
git commit -m "feat(qol-theme): emit tray theme override blocks and metadata"
```

---

### Task 3: Theme persistence in the tray backend

**Files:**
- Modify: `apps/qol-tray/src/features/theme.rs`
- Modify: `libs/qol-conventions/src/lib.rs` (find the `ENV_THEME_ACCENT` const and add a sibling)
- Modify: the two `apply_accent_env` call sites: `apps/qol-tray/src/plugins/action_executor/execution.rs`, `apps/qol-tray/src/plugins/daemon_lifecycle/spawn.rs`

**Interfaces:**
- Consumes: `qol_theme::tray_theme_preset`, `qol_theme::DEFAULT_TRAY_THEME_KEY`.
- Produces: `save_selected_theme_key(&str)`, `clear_selected_theme_key()`, `selected_theme_key() -> Result<Option<String>>`, `current_theme_key() -> String`, `apply_theme_name_env(&mut Command)`, `qol_conventions::ENV_THEME_NAME`.

- [ ] **Step 1: Write the failing tests** (append to `mod tests` in `theme.rs`)

```rust
    #[test]
    fn selected_theme_round_trips_and_rejects_unknown_key() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        assert!(save_selected_theme_key("not-a-theme").is_err());
        save_selected_theme_key("void").unwrap();
        assert_eq!(selected_theme_key().unwrap().as_deref(), Some("void"));
        assert_eq!(current_theme_key(), "void");
    }

    #[test]
    fn theme_clears_to_default_and_preserves_accent() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        save_selected_accent_key("blue").unwrap();
        save_selected_theme_key("graphite").unwrap();
        clear_selected_theme_key().unwrap();

        assert_eq!(selected_theme_key().unwrap(), None);
        assert_eq!(current_theme_key(), qol_theme::DEFAULT_TRAY_THEME_KEY);
        assert_eq!(selected_accent_key().unwrap().as_deref(), Some("blue"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p qol-tray --features dev features::theme`
Expected: compile error, functions not defined.

- [ ] **Step 3: Implement**

`ThemeSettings` gains `theme: Option<String>` (serde defaults a missing `Option` field to `None`, so old `theme.json` files load unchanged).
`save_selected_accent_key`/`clear_selected_accent_key` must now load-modify-write instead of overwriting the struct, so accent and theme edits preserve each other; add a private helper:

```rust
fn update_settings(update: impl FnOnce(&mut ThemeSettings)) -> Result<()> {
    let path = settings_path()?;
    let mut settings: ThemeSettings =
        crate::file_io::load_json_or_default(&path).context("failed to load theme settings")?;
    update(&mut settings);
    crate::file_io::write_pretty_json(&path, &settings).context("failed to save theme settings")
}
```

Rewrite the accent save/clear through `update_settings`, then add:

```rust
pub fn save_selected_theme_key(key: &str) -> Result<()> {
    let key = validated_theme_key(key)?.to_string();
    update_settings(|settings| settings.theme = Some(key))
}

pub fn clear_selected_theme_key() -> Result<()> {
    update_settings(|settings| settings.theme = None)
}

pub fn selected_theme_key() -> Result<Option<String>> {
    let settings: ThemeSettings = crate::file_io::load_json_or_default(&settings_path()?)
        .context("failed to load theme settings")?;
    match settings.theme.as_deref() {
        Some(key) => Ok(Some(validated_theme_key(key)?.to_string())),
        None => Ok(None),
    }
}

pub fn current_theme_key() -> String {
    selected_theme_key()
        .ok()
        .flatten()
        .unwrap_or_else(|| qol_theme::DEFAULT_TRAY_THEME_KEY.to_string())
}

pub fn apply_theme_name_env(command: &mut Command) {
    command.env(qol_conventions::ENV_THEME_NAME, current_theme_key());
}

fn validated_theme_key(key: &str) -> Result<&str> {
    if qol_theme::tray_theme_preset(key).is_some() {
        return Ok(key);
    }
    Err(anyhow!("unknown theme: {key}"))
}
```

In `qol-conventions`, next to `ENV_THEME_ACCENT`, add the matching `ENV_THEME_NAME` const following the existing naming/value style.
At both `apply_accent_env(...)` call sites add `crate::features::theme::apply_theme_name_env(...)` on the next line (adjust path from the plugins module as the accent call is written there).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p qol-tray --features dev features::theme`
Expected: all theme tests PASS, including the pre-existing accent tests.

- [ ] **Step 5: Gates and commit**

Run: `cargo clippy -p qol-tray --features dev -- -D warnings && cargo fmt --all -- --check`

```bash
git add apps/qol-tray/src/features/theme.rs libs/qol-conventions/src/lib.rs apps/qol-tray/src/plugins/action_executor/execution.rs apps/qol-tray/src/plugins/daemon_lifecycle/spawn.rs
git commit -m "feat(qol-tray): persist selected theme beside accent"
```

---

### Task 4: Theme endpoint, boot injection, first-paint attribute

**Files:**
- Modify: `apps/qol-tray/src/features/plugin_store/server/settings/theme_handlers.rs`
- Modify: `apps/qol-tray/src/features/plugin_store/server/settings/mod.rs` (routes, line ~47)
- Modify: `apps/qol-tray/src/features/plugin_store/server/boot.rs`
- Modify: `apps/qol-tray/ui/index.html`

**Interfaces:**
- Consumes: Task 3 functions.
- Produces: `GET/PUT /api/theme` with body `{ key: string | null }` and response `{ key, selectedKey }`; `__QOL_BOOT__.theme = { themes: [{key,label}], defaultKey, selectedKey }`; `data-qol-theme` set on `<html>` before first paint.

- [ ] **Step 1: Write the failing boot test** (append to tests in `boot.rs`, mirroring the accent tests there)

```rust
    #[test]
    fn boot_json_carries_theme_selection_and_palette() {
        let root = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        crate::features::theme::save_selected_theme_key("graphite").unwrap();

        let v: serde_json::Value = serde_json::from_str(&boot_json(false)).unwrap();
        assert_eq!(v["theme"]["defaultKey"], "graphite");
        assert_eq!(v["theme"]["selectedKey"], "graphite");
        let themes = v["theme"]["themes"].as_array().unwrap();
        assert_eq!(themes.len(), qol_theme::tray_theme_presets().len());
        assert_eq!(themes[0]["key"], "slate");
        assert_eq!(themes[0]["label"], "Slate");
    }
```

Match the surrounding tests' exact setup helpers if they differ from this sketch (read the neighboring accent tests first and copy their pattern).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p qol-tray --features dev boot_json_carries_theme`
Expected: FAIL (no `theme` key in boot json).

- [ ] **Step 3: Implement**

`boot.rs`: add a `ThemeBoot { themes: Vec<ThemeEntry>, default_key: String, selected_key: Option<String> }` with `#[serde(rename_all = "camelCase")]` matching the existing `AccentBoot` serde style, `ThemeEntry { key, label }` built from `qol_theme::tray_theme_presets()`, `default_key = crate::features::theme::current_theme_key()`, `selected_key = selected_theme_key().ok().flatten()`.
`theme_handlers.rs`: add `get_theme` / `set_theme` mirroring the accent pair exactly (same `blocking` wrapper, same request/response shape with `key: Option<String>`), calling the Task 3 save/clear; do NOT call `restart_running_gpui_daemons` (plugins do not consume the theme yet).
`settings/mod.rs`: add `.route("/theme", get(theme_handlers::get_theme))` and the `put` twin beside the accent routes.
`index.html`: inside the existing boot `<script>`, after the accent block, add:

```js
            if (boot && boot.theme && boot.theme.defaultKey) {
                document.documentElement.setAttribute('data-qol-theme', boot.theme.defaultKey);
            }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p qol-tray --features dev`
Expected: PASS.

- [ ] **Step 5: Gates and commit**

Run: `cargo clippy -p qol-tray --features dev -- -D warnings && cargo fmt --all -- --check`

```bash
git add apps/qol-tray/src/features/plugin_store/server/settings/theme_handlers.rs apps/qol-tray/src/features/plugin_store/server/settings/mod.rs apps/qol-tray/src/features/plugin_store/server/boot.rs apps/qol-tray/ui/index.html
git commit -m "feat(qol-tray): serve theme endpoint and boot-inject selection"
```

---

### Task 5: Frontend theme modules

**Files:**
- Create: `apps/qol-tray/ui/lib/theme-presets.js`
- Create: `apps/qol-tray/ui/lib/theme-sync.js`
- Test: `apps/qol-tray/ui/lib/theme-sync.test.js`
- Modify: `apps/qol-tray/ui/components/App.js` (line ~326, beside `applyThemeAccent()`)

**Interfaces:**
- Consumes: `QOL_THEMES`, `QOL_DEFAULT_THEME` from `generated-theme-tokens.js`; `__QOL_BOOT__.theme`; `apiJson`, `jsonRequest` from `../api/client.js`.
- Produces: `THEMES`, `DEFAULT_THEME`, `SELECTED_THEME`, `resolveTheme(key)`, `applyTheme(key)` (sets/clears the `data-qol-theme` attribute) from `theme-presets.js`; `getTheme()`, `applyThemeSelection()`, `subscribeTheme(listener)`, `setTheme(key)` from `theme-sync.js`.

- [ ] **Step 1: Write `theme-presets.js`**

```js
import { QOL_THEMES, QOL_DEFAULT_THEME } from './generated-theme-tokens.js';

const boot = (typeof window !== 'undefined' && window.__QOL_BOOT__) || null;

export const THEMES = (boot?.theme?.themes?.length ? boot.theme.themes : QOL_THEMES)
    .map((entry) => ({ key: entry.key, label: entry.label }));

export const DEFAULT_THEME = boot?.theme?.defaultKey ?? QOL_DEFAULT_THEME;
export const SELECTED_THEME = boot?.theme?.selectedKey ?? null;

export function resolveTheme(key) {
    if (key && THEMES.some((theme) => theme.key === key)) return key;
    return DEFAULT_THEME;
}

export function applyTheme(key) {
    const resolved = resolveTheme(key);
    document.documentElement.setAttribute('data-qol-theme', resolved);
}
```

- [ ] **Step 2: Write `theme-sync.js`** (mirror `theme-accent-sync.js` exactly, substituting the endpoint and module)

```js
import { apiJson, jsonRequest } from '../api/client.js';
import { applyTheme, DEFAULT_THEME, resolveTheme, SELECTED_THEME } from './theme-presets.js';

let selectedThemeKey = SELECTED_THEME;
let effectiveThemeKey = resolveTheme(selectedThemeKey ?? DEFAULT_THEME);
const listeners = new Set();

export function getTheme() {
    return selectedThemeKey;
}

export function applyThemeSelection() {
    applyTheme(effectiveThemeKey);
    return selectedThemeKey;
}

export function subscribeTheme(listener) {
    listeners.add(listener);
    return () => listeners.delete(listener);
}

export async function setTheme(key) {
    const response = await apiJson('/api/theme', jsonRequest('PUT', { key }, { qolSuppressErrorToast: true }));
    return commitTheme(response.selectedKey ?? null, response.key);
}

function commitTheme(nextSelectedKey, nextEffectiveKey) {
    const resolvedSelectedKey = nextSelectedKey && resolveTheme(nextSelectedKey) === nextSelectedKey
        ? nextSelectedKey
        : null;
    const resolvedEffectiveKey = resolveTheme(nextEffectiveKey);
    const changed = selectedThemeKey !== resolvedSelectedKey || effectiveThemeKey !== resolvedEffectiveKey;
    selectedThemeKey = resolvedSelectedKey;
    effectiveThemeKey = resolvedEffectiveKey;
    applyTheme(effectiveThemeKey);
    if (changed) {
        for (const listener of listeners) listener(selectedThemeKey);
    }
    return selectedThemeKey;
}
```

- [ ] **Step 3: Write the test** (`theme-sync.test.js`)

Read `apps/qol-tray/ui/lib/theme-accent-sync.test.js` first and clone its mocking approach (module mocking of `api/client.js` and DOM stubbing) for the theme module, covering: `setTheme('void')` PUTs to `/api/theme` and sets the attribute; unknown response keys resolve to the default; `subscribeTheme` fires on change and not on no-op.

- [ ] **Step 4: Run UI tests**

Run: `cd apps/qol-tray/ui && node --test lib/theme-sync.test.js`
Expected: PASS.

- [ ] **Step 5: Wire startup apply and commit**

In `App.js` beside `applyThemeAccent();` add `applyThemeSelection();` with the matching import.
Run the full UI suite (Global Constraints command).

```bash
git add apps/qol-tray/ui/lib/theme-presets.js apps/qol-tray/ui/lib/theme-sync.js apps/qol-tray/ui/lib/theme-sync.test.js apps/qol-tray/ui/components/App.js
git commit -m "feat(qol-tray): add frontend theme switching modules"
```

---

### Task 6: Theme switcher UI (Minimap row + gallery section)

**Files:**
- Modify: `apps/qol-tray/ui/components/shell/Minimap.js` (AccentRow is at line ~155; ThemeRow goes beside it, rendered above `AccentRow` at line ~143)
- Modify: `apps/qol-tray/ui/views/dev/components/ComponentsCatalog.js` (new `CatalogSection`)
- Modify: the stylesheet defining `.wsp-accent` / `.wsp-swatch` (grep `wsp-swatch` under `apps/qol-tray/ui/styles/`; extend it, do not fork)

**Interfaces:**
- Consumes: `THEMES` from `theme-presets.js`; `getTheme`, `setTheme`, `subscribeTheme` from `theme-sync.js`; `Surface` component.

- [ ] **Step 1: Add ThemeRow to Minimap**

```js
function ThemeRow({ value, onPick }) {
    return html`
        <div class="wsp-accent">
            <span class="wsp-label">Theme</span>
            <div class="wsp-swatches">
                ${THEMES.map((theme) => html`
                    <${Surface} as="button" key=${theme.key}
                        className=${`wsp-swatch wsp-theme-swatch${(value ?? DEFAULT_THEME) === theme.key ? ' is-active' : ''}`}
                        data-qol-theme-preview=${theme.key} title=${theme.label}
                        onActivate=${() => onPick(theme.key)} />
                `)}
            </div>
        </div>`;
}
```

State wiring copies the accent pattern at lines 98-108 (`useState(getTheme)`, `subscribeTheme`, `setTheme(key).catch(...)` with the same error toast style).
Swatch CSS: `.wsp-theme-swatch` shows the theme's surface stack; per key, a three-stop `linear-gradient` using that theme's canvas/raised/hovered hexes via `[data-qol-theme-preview="graphite"]` selectors in the same stylesheet (static CSS, values copied from the preset definitions; a stale swatch color is caught visually in the gallery, acceptable).

- [ ] **Step 2: Add the gallery section**

In `ComponentsCatalog.js`, add a `CatalogSection title="Theme"` rendering the same `ThemeRow` state wiring so every primitive can be audited per theme from one page.
Reuse the row via export from Minimap only if it stays a pure component; otherwise duplicate the 15-line render locally (gallery may diverge later).
Prefer promoting `ThemeRow` into `apps/qol-tray/ui/lib/components/ThemeRow.js` and importing it in both places; that satisfies the no-stray rule this plan enforces.

- [ ] **Step 3: Verify live**

Recompile via `POST http://127.0.0.1:42700/api/dev/recompile-self`, then with Playwright: open the UI, click each theme swatch in the minimap, assert `document.documentElement.getAttribute('data-qol-theme')` changes and persists across a reload.

- [ ] **Step 4: Gates and commit**

Run the full UI suite plus `cargo test -p qol-tray --features dev`.

```bash
git add apps/qol-tray/ui/lib/components/ThemeRow.js apps/qol-tray/ui/components/shell/Minimap.js apps/qol-tray/ui/views/dev/components/ComponentsCatalog.js apps/qol-tray/ui/styles/world.css
git commit -m "feat(qol-tray): add theme switcher to minimap and gallery"
```

(Substitute the actual stylesheet path found in Step 1.)

---

### Task 7: Stray-component guard test

**Files:**
- Create: `apps/qol-tray/ui/lib/stray-interactive-elements.test.js`

**Interfaces:**
- Produces: CI-enforced invariant; the grandfather list below is the burn-down ledger for Tasks 9-12.

- [ ] **Step 1: Write the test**

```js
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const uiRoot = join(fileURLToPath(new URL('.', import.meta.url)), '..');
const SCAN_DIRS = ['views', 'components', 'app'];
const RAW_INTERACTIVE = /<(button|select|textarea|input)\b/;

const GRANDFATHERED = new Set([
    'app/views.js',
    'components/ApiErrorToast.js',
    'components/BootHealedBanner.js',
    'components/CommandPalette.js',
    'components/domain-rows/PluginRow.js',
    'components/domain-rows/StoreCard.js',
    'components/domain-rows/SuppressedRow.js',
    'components/shell/Minimap.js',
    'components/shell/PeripheralPreview.js',
    'views/dev/components/ComponentsCatalog.js',
    'views/dev/components/LinkInput.js',
    'views/dev/components/PluginsSection.js',
    'views/dev/components/ToolingGhAccountSection.js',
    'views/dev/gpui-subpage.js',
    'views/dev/log-filters-subpage.js',
    'views/hotkeys/modal.js',
    'views/plugin-config/field-map.js',
    'views/plugin-config/fields/ActionField.js',
    'views/plugin-config/fields/NumberField.js',
    'views/plugin-config/fields/ObjectArrayField.js',
    'views/plugin-config/fields/ObjectMapField.js',
    'views/plugin-config/fields/StringArrayField.js',
    'views/plugin-config/view.js',
    'views/plugins/grid.js',
    'views/profile/components.js',
    'views/profile/view.js',
    'views/shortcuts/modal.js',
    'views/task-runner/panels.js',
    'views/task-runner-view.js',
]);

function jsFiles(dir) {
    return readdirSync(dir, { withFileTypes: true, recursive: true })
        .filter((entry) => entry.isFile() && entry.name.endsWith('.js') && !entry.name.endsWith('.test.js'))
        .map((entry) => join(entry.parentPath, entry.name));
}

test('views compose gallery components instead of raw interactive elements', () => {
    const offenders = [];
    const cleanGrandfathered = [];
    for (const dir of SCAN_DIRS) {
        for (const file of jsFiles(join(uiRoot, dir))) {
            const rel = relative(uiRoot, file);
            const raw = RAW_INTERACTIVE.test(readFileSync(file, 'utf8'));
            if (raw && !GRANDFATHERED.has(rel)) offenders.push(rel);
            if (!raw && GRANDFATHERED.has(rel)) cleanGrandfathered.push(rel);
        }
    }
    assert.deepEqual(offenders, [], `raw <button>/<select>/<input>/<textarea> outside lib/components; use gallery primitives or extend them: ${offenders.join(', ')}`);
    assert.deepEqual(cleanGrandfathered, [], `now clean; remove from GRANDFATHERED: ${cleanGrandfathered.join(', ')}`);
});
```

- [ ] **Step 2: Run to verify it passes with the current tree**

Run: `cd apps/qol-tray/ui && node --test lib/stray-interactive-elements.test.js`
Expected: PASS (everything currently stray is grandfathered).
If Task 6 added files containing raw elements, the offenders assert names them; fix those instead of grandfathering.

- [ ] **Step 3: Commit**

```bash
git add apps/qol-tray/ui/lib/stray-interactive-elements.test.js
git commit -m "test(qol-tray): guard against stray raw interactive elements"
```

---

### Task 8: TextInput primitive

**Files:**
- Create: `apps/qol-tray/ui/lib/components/TextInput.js`
- Modify: `apps/qol-tray/ui/views/dev/components/ComponentsCatalog.js` (new section)

**Interfaces:**
- Produces: `TextInput({ value, onInput, onSubmit, placeholder, type, className, inputRef, disabled })` rendering the standard styled `<input>`; the ONLY sanctioned raw text input.

- [ ] **Step 1: Extract the primitive**

Read `views/dev/components/LinkInput.js` and the input styling in `styles/common-controls.css`; implement `TextInput` as a thin `Surface`-compatible wrapper around `<input>` carrying the shared classes, keyboard handling (Enter fires `onSubmit` when provided), and forwarded ref.
Keep `LinkInput` as a consumer of `TextInput` (it keeps its link-specific validation), which removes `LinkInput` from the grandfather list.

- [ ] **Step 2: Catalog it**

Add `CatalogSection title="Text input"` with a controlled example.

- [ ] **Step 3: Verify, shrink grandfather list, commit**

Run the UI suite; remove `views/dev/components/LinkInput.js` from `GRANDFATHERED` (the second assert enforces this).

```bash
git add apps/qol-tray/ui/lib/components/TextInput.js apps/qol-tray/ui/views/dev/components/LinkInput.js apps/qol-tray/ui/views/dev/components/ComponentsCatalog.js apps/qol-tray/ui/lib/stray-interactive-elements.test.js
git commit -m "feat(qol-tray): promote TextInput gallery primitive"
```

---

### Task 9: Consolidation sweep - plugin-config fields

**Files:**
- Modify: `views/plugin-config/fields/ActionField.js`, `NumberField.js`, `ObjectArrayField.js`, `ObjectMapField.js`, `StringArrayField.js`, `views/plugin-config/field-map.js`, `views/plugin-config/view.js` (all under `apps/qol-tray/ui/`)
- Modify: `apps/qol-tray/ui/lib/stray-interactive-elements.test.js` (shrink list)

**Conversion table (applies to Tasks 9-12):**

| Raw element | Replacement | Import |
| --- | --- | --- |
| `<button>` | `Button` (or `Surface as="button"` where wedge/selection semantics are needed) | `lib/components/Button.js` / `Surface.js` |
| `<select>` | `CustomSelect` | `lib/components/CustomSelect.js` |
| `<input type="checkbox">` | `ToggleSwitch` | `lib/components/ToggleSwitch.js` |
| `<input>` (text/number) | `TextInput` (Task 8) | `lib/components/TextInput.js` |
| `<textarea>` | keep only if inside a primitive being promoted; otherwise `TextInput` with `as="textarea"` support added to `TextInput` first | `lib/components/TextInput.js` |

- [ ] **Step 1: Convert one file at a time**

For each file: replace per the table, preserving every existing handler, class hook, and keyboard flow (`onActivate` on `Surface` equals click+Enter+Space); run that file's related tests (`node --test` on siblings) plus the guard test; remove the file from `GRANDFATHERED`; commit:

```bash
git commit -m "refactor(qol-tray): compose <file> from gallery primitives"
```

One commit per file, message naming the actual file (e.g. `refactor(qol-tray): compose ActionField from gallery primitives`).

- [ ] **Step 2: Verify the group**

Run the full UI suite; drive the plugin-config page live via Playwright (open a plugin config, edit a string-array row, toggle a boolean, pick from a select) before the last commit of the group.

---

### Task 10: Consolidation sweep - dev views

**Files:** `views/dev/components/ComponentsCatalog.js` (only raw usages OUTSIDE intentional raw-element demos; if the catalog deliberately shows raw elements, wrap those demos in the sanctioned primitives instead), `views/dev/components/PluginsSection.js`, `ToolingGhAccountSection.js`, `views/dev/gpui-subpage.js`, `views/dev/log-filters-subpage.js`.

Same per-file loop, conversion table, commit pattern, and grandfather burn-down as Task 9.
Live-verify the dev page (link a plugin, filter logs) via Playwright before the last commit.

---

### Task 11: Consolidation sweep - feature views

**Files:** `views/hotkeys/modal.js`, `views/shortcuts/modal.js`, `views/plugins/grid.js`, `views/profile/components.js`, `views/profile/view.js`, `views/task-runner/panels.js`, `views/task-runner-view.js`, `app/views.js`.

Same loop.
Modals: keyboard-first is a hard rule; verify focus trapping still holds after converting modal buttons (both modals have existing tests; run them).

---

### Task 12: Consolidation sweep - shell components

**Files:** `components/ApiErrorToast.js`, `BootHealedBanner.js`, `CommandPalette.js`, `domain-rows/PluginRow.js`, `domain-rows/StoreCard.js`, `domain-rows/SuppressedRow.js`, `shell/Minimap.js`, `shell/PeripheralPreview.js`.

Same loop.
`CommandPalette.js` search input becomes `TextInput` with its existing key routing preserved exactly (arrow/Enter/Escape handling stays in the palette, not the primitive).
After this task `GRANDFATHERED` must be empty; delete the set and simplify the test to assert no offenders, in the same commit as the last conversion:

```bash
git commit -m "refactor(qol-tray): finish gallery consolidation, drop grandfather list"
```

---

### Task 13: Slate depth retune

**Files:**
- Modify: `libs/qol-theme/src/lib.rs` (slate preset only; `DARK_REFERENCE`/`DARK_SYSTEM` stay untouched so plugin profiles are unaffected)
- Regenerate: both tray token files (commands from Task 2 Step 5)

- [ ] **Step 1: Give slate its own retuned SystemPalette**

Replace `system: DARK_SYSTEM` in the slate preset with an explicit `SystemPalette { ... }` copying DARK_SYSTEM but with stronger tier separation:

```rust
            surface_canvas: 0x0b0d12,
            surface_elevated: 0x151a23,
            surface_raised: 0x1b212c,
            surface_hovered: 0x242c3a,
```

(All other fields copied verbatim from `DARK_SYSTEM` values.)

- [ ] **Step 2: Contrast tests + visual check**

Run: `cargo test -p qol-theme` (floors from Task 1 must still pass; the adjacent-surface floor should now clear more comfortably).
Regenerate, recompile, eyeball the gallery: canvas vs card vs hover must read as three distinct planes.
Iterate the four hexes until it does; the numbers above are the starting point, your eyes are the acceptance test.

- [ ] **Step 3: Commit**

```bash
git add libs/qol-theme/src/lib.rs apps/qol-tray/ui/styles/generated-theme-tokens.css apps/qol-tray/ui/lib/generated-theme-tokens.js
git commit -m "feat(qol-theme): retune slate surface tiers for depth"
```

---

### Task 14: Apply elevation tokens consistently

**Files:**
- Modify: shared component CSS only: `styles/card.css`, `styles/common-dialogs.css`, `styles/action-menu.css`, `styles/common-components.css`, `styles/searchable-action-list.css` (final list from the audit below)

- [ ] **Step 1: Audit**

Run: `grep -n "box-shadow" apps/qol-tray/ui/styles/*.css | grep -v elevation | grep -v inset`
Classify each hit: resting card → `var(--elevation-1)`, hover/raised or dropdown/menu → `var(--elevation-2)`, modal/overlay → `var(--elevation-3)`.
Glows and focus rings (accent/success shadows) are NOT elevation; leave them.

- [ ] **Step 2: Replace, verify, commit**

Replace the classified hits; view-specific CSS files are out of scope (surface them, don't touch).
Verify in the gallery under all three themes (elevation uses `--layer-ink-*` which follows `--ink-rgb`, staying theme-correct).

```bash
git commit -m "style(qol-tray): apply elevation tokens to shared surfaces"
```

---

### Task 15: Typography and density pass

**Files:**
- Modify: `styles/theme-tokens.css` (scale defs), gallery component CSS (`list-row.css`, `table.css`, `common-controls.css`, `card.css`)

- [ ] **Step 1: Audit the type scale**

Run: `grep -ohn "var(--fs-[a-z0-9-]*)" apps/qol-tray/ui/styles/*.css | sort | uniq -c | sort -rn` and the same for `font-size:` literals.
Deliverable: every font-size in gallery CSS uses a `--fs-*` token; hardcoded px sizes in shared CSS are migrated to the nearest token; row title / meta / section label use three distinct, documented tiers (`--fs-md` / `--fs-sm` / `--fs-xs` with `--fw-semibold` on titles).

- [ ] **Step 2: Normalize density**

`ListRow`/`TableRow`/config field rows share one vertical rhythm: padding from `--space-2`/`--space-3` only; kill one-off `padding: 7px 9px`-style values in shared CSS.

- [ ] **Step 3: Verify and commit**

Full UI suite + gallery screenshots (Playwright) in all three themes, compared against pre-change screenshots taken before this task; the user judges the result on the gallery page.

```bash
git commit -m "style(qol-tray): normalize type scale and row rhythm"
```

---

### Task 16: End-to-end verification

- [ ] **Step 1: Full gates**

```bash
cargo test -p qol-theme && cargo test -p qol-tray --features dev
cargo fmt --all -- --check && cargo clippy -p qol-theme -p qol-tray --features dev -- -D warnings
cd apps/qol-tray/ui && mapfile -d '' t < <(find . -name '*.test.js' -print0) && node --test "${t[@]}"
```

- [ ] **Step 2: Live pass**

Recompile via the dev endpoint; with Playwright: switch to each theme, reload (persistence), screenshot gallery + plugins page + a plugin config page + logs page per theme; confirm accent picker still recolors under every theme; confirm no console errors.

- [ ] **Step 3: Report**

Present screenshots to the user for the style verdict; the style pass (Tasks 13-15) is expected to iterate on their feedback.
