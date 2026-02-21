# Platform Module Conventions

This repository uses a strict platform layout for OS-specific code.

## Required layout

For any feature with platform-specific behavior:

- Put code under `<feature>/platform/`
- Require a `platform/mod.rs`
- Use these files when applicable:
  - `platform/linux.rs`
  - `platform/macos.rs`
  - `platform/windows.rs`
  - `platform/unsupported.rs`
- If Linux and macOS share internals, keep shared logic in:
  - `platform/unix_common.rs`

## Wiring rules

- `platform/mod.rs` is the only cfg-switchboard.
- Parent module calls into `platform::...` only.
- Do not keep flat siblings like `unix.rs`/`windows.rs` next to parent `mod.rs`.
- Platform leaf modules should stay small and delegate to `unix_common.rs` when possible.

## Why

- Predictable tree shape across the repository.
- Easier onboarding and code navigation.
- Lower risk of duplicated cfg logic spread across files.
