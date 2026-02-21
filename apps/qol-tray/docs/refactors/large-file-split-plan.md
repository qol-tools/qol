# Large File Split Plan

This plan targets three oversized files:

- `src/features/plugin_store/server.rs`
- `src/dev/build.rs`
- `ui/styles.css`

The goal is to split by responsibility, reduce cross-coupling, and make future changes local.

## 1) `src/features/plugin_store/server.rs`

## Why it grew

`server.rs` currently owns too many layers at once:

- API router composition
- DTOs + state types
- HTTP handlers for install/update/config/hotkeys/tokens
- Dev-only build orchestration + mock target runtime
- Build-state global synchronization and event fanout

## Target structure

- `src/features/plugin_store/server/mod.rs`
- `src/features/plugin_store/server/types.rs`
- `src/features/plugin_store/server/assets.rs`
- `src/features/plugin_store/server/reload.rs`
- `src/features/plugin_store/server/handlers/`
- `src/features/plugin_store/server/handlers/plugins.rs`
- `src/features/plugin_store/server/handlers/actions.rs`
- `src/features/plugin_store/server/handlers/config.rs`
- `src/features/plugin_store/server/handlers/token.rs`
- `src/features/plugin_store/server/handlers/hotkeys.rs`
- `src/features/plugin_store/server/handlers/updates.rs`
- `src/features/plugin_store/server/dev/`
- `src/features/plugin_store/server/dev/build_state.rs`
- `src/features/plugin_store/server/dev/links.rs`
- `src/features/plugin_store/server/dev/discovery.rs`
- `src/features/plugin_store/server/dev/reload.rs`
- `src/features/plugin_store/server/dev/mock_runtime.rs`

## Notes

- Keep `AppState` in one place and pass it into handlers with thin wrappers.
- Build-state and mock runtime should become dedicated services, not free functions.
- Router wiring should be declarative and feature-gated once (public routes vs dev routes).

## 2) `src/dev/build.rs`

## Why it grew

`build.rs` mixes:

- plugin build planning and fingerprint state persistence
- plugin cargo execution and output capture
- self-recompile cargo execution
- cargo progress parser + progress estimation model
- test suite

## Target structure

- `src/dev/build/mod.rs`
- `src/dev/build/types.rs`
- `src/dev/build/plan.rs`
- `src/dev/build/fingerprint.rs`
- `src/dev/build/plugin_runner.rs`
- `src/dev/build/self_runner.rs`
- `src/dev/build/progress_parser.rs`
- `src/dev/build/progress_estimator.rs`
- `src/dev/build/tests.rs` (or `tests/*` split by concern)

## Notes

- Keep stable public API from `mod.rs` for callers (`build_linked_plugins_with_progress`, `build_qol_tray_self_with_progress`, etc.).
- Parser and estimator should be pure modules with focused tests.
- Runner modules should own process/pipe/thread orchestration only.

## 3) `ui/styles.css`

## Why it grew

Single-file CSS currently combines:

- tokens + reset
- app shell layout
- shared components (buttons, badges, modals)
- view-specific styling (plugins/store/hotkeys/dev)
- animation definitions

This increases cascade collisions and makes visual changes hard to isolate.

## Target structure

- `ui/styles/index.css`
- `ui/styles/tokens.css`
- `ui/styles/base.css`
- `ui/styles/layout.css`
- `ui/styles/components/buttons.css`
- `ui/styles/components/cards.css`
- `ui/styles/components/forms.css`
- `ui/styles/components/modals.css`
- `ui/styles/components/badges.css`
- `ui/styles/components/progress.css`
- `ui/styles/views/plugins.css`
- `ui/styles/views/store.css`
- `ui/styles/views/hotkeys.css`
- `ui/styles/views/dev.css`

## Notes

- Use CSS layers in `index.css`:
  - `@layer tokens, base, layout, components, views, overrides;`
- Move duplicated selector intent into component files first, then tune visuals.
- Keep view-local selectors in view files to prevent global regressions.

## Migration order (recommended)

1. Split `server.rs` into internal modules while preserving routes/behavior.
2. Split `build.rs` into build package modules with unchanged external API.
3. Split CSS into layered files and update `ui/index.html` to load `styles/index.css`.

## Safety rails

- One concern per commit, `wip:` commit messages.
- Compile after each Rust split (`cargo check --features dev`).
- UI syntax check after each JS/CSS step (and quick manual smoke pass once all splits land).
- No behavior changes mixed into structural moves unless explicitly called out.
