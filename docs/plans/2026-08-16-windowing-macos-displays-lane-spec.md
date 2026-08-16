# Spec: qol-windowing macOS display enumeration

`libs/qol-windowing/src/display/platform/macos.rs` is a stub returning
`DisplayError::UnsupportedPlatform`, which blocks all monitor-plugin work on
macOS. Implement real enumeration.

All work happens in this worktree on branch `windowing-macos-displays`.
Scope: `libs/qol-windowing/src/display/platform/macos.rs` and, only if a
framework link stanza is required, `libs/qol-windowing/Cargo.toml` /
`build.rs`. Prefer `#[link(name = "CoreGraphics", kind = "framework")]`
extern "C" declarations inside macos.rs; add NO new crate dependencies.
Do not touch any other file.
Code comments are banned in this repo; use self-explanatory names.

## Behavior

`DisplayEnumerator::enumerate` returns one `DisplayHandle` per online display:

- Enumerate with `CGGetOnlineDisplayList` (u32 display ids, cap 16).
- Per display read `CGDisplayVendorNumber`, `CGDisplayModelNumber`,
  `CGDisplaySerialNumber`, `CGDisplayIsBuiltin`.
- `id`: `format!("mac-{:016x}", hash)` where hash is the first 8 bytes of
  sha256 over the exact byte string `"{vendor}:{model}:{serial}"` (decimal).
  The workspace already depends on sha2; use it if qol-windowing already has
  it, otherwise add nothing and use a small local fnv-free implementation is
  NOT allowed - in that case put sha2 in qol-windowing's Cargo.toml only if
  sha2 is already a workspace dependency elsewhere (it is; match the
  workspace version style used by sibling libs).
- `connector`: `format!("cg-{display_id}")` (the live CGDirectDisplayID; it
  is a runtime address, not identity).
- `edid_sha256`: `None` for now.
- `identity_unstable`: true when `CGDisplaySerialNumber` returns 0.
- Built-in displays are included, with connector `format!("cg-{id}-builtin")`.
- Zero displays (headless session) is `Ok(vec![])`, not an error.

Keep the extern declarations and unsafe blocks minimal and contained in this
one file. The pure id/connector derivation must be a separate function taking
(vendor: u32, model: u32, serial: u32, display_id: u32, builtin: bool) and
returning DisplayHandle, so it is testable without CoreGraphics.

## Tests (same file, existing style)

- Derivation: fixed (vendor, model, serial) tuple produces a stable `mac-`
  id (pin the exact expected string), serial 0 sets identity_unstable,
  builtin flag changes the connector suffix, same tuple twice is equal,
  differing serial changes the id.
- `enumerate_returns_ok_on_this_host`: `#[cfg(target_os = "macos")]` test
  asserting `Platform.enumerate().is_ok()` only.
- Replace the old `stub_returns_typed_error` test.

## Gate before committing

Run from the worktree root and paste real output in your report:

```
cargo test -p qol-windowing
cargo fmt --check -p qol-windowing
cargo clippy -p qol-windowing --all-targets -- -D warnings
cargo check --target x86_64-unknown-linux-gnu -p qol-windowing
```

If the linux target is not installed, report that instead of installing it.

Commit on this branch with a conventional message like
`feat(qol-windowing): enumerate macOS displays with stable identity`.
NEVER add Co-Authored-By, "Generated with", or any AI attribution.
