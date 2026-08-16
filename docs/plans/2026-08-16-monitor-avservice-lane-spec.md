# Spec: monitor plugin macOS DDC backend (IOAVService)

`plugins/monitor/src/platform/macos.rs` returns `StubControl`, so brightness
does nothing on macOS. Implement the Apple Silicon external-display DDC path
(the Lunar/MonitorControl mechanism) per the spec's macOS mitigations
(docs/specs/2026-08-16-monitor-control.md, "macOS bottleneck mitigations").

All work happens in this worktree on branch `monitor-avservice`.
Scope: new file `plugins/monitor/src/monitor/backends/avservice.rs`,
`plugins/monitor/src/monitor/backends/mod.rs` (module registration),
`plugins/monitor/src/monitor/backends/i2c_ddc.rs` (ONLY visibility widening
of existing framing helpers, no behavior change),
`plugins/monitor/src/platform/macos.rs`. Do not touch any other file.
Code comments are banned in this repo; use self-explanatory names.

## avservice.rs

Private-API access hardened per spec: resolve every private symbol with
`dlsym(RTLD_DEFAULT, ...)` at first use after linking the IOKit framework;
a missing symbol is a typed `MonitorError::Unsupported`-style error carrying
the symbol name, never a crash or panic. Needed symbols:
`IOAVServiceCreateWithService`, `IOAVServiceWriteI2C`, `IOAVServiceReadI2C`.
Public IOKit (`IOServiceMatching`, `IOServiceGetMatchingServices`,
`IOIteratorNext`, `IORegistryEntryCreateCFProperty`, `IOObjectRelease`) may
be declared as normal extern "C" with
`#[link(name = "IOKit", kind = "framework")]`; CoreFoundation string/compare
helpers likewise. No new crate dependencies.

Discovery: match `DCPAVServiceProxy` services whose `Location` property is
`External`, in iteration order. Pair them positionally with the non-builtin
displays from `crate::platform`-visible enumeration (`qol_windowing`
DisplayHandle order, skipping connectors ending in `-builtin`). If the
external display count and service count differ, every DDC call returns a
typed error naming both counts; the gamma/policy layer then shows the
failure instead of guessing.

Wire protocol: reuse the verified framing in i2c_ddc.rs by widening
`xor_checksum`, `get_vcp_request`, `set_vcp_request`, `parse_get_vcp_reply`,
`REPLY_LEN`, and the timing constants to `pub(crate)`. Transport semantics:

- Write: `IOAVServiceWriteI2C(service, 0x37, 0x51, payload, len)` where
  payload is the Linux frame minus its leading 0x51 byte (the dataAddress
  argument carries it). Checksums stay exactly as i2c_ddc computes them.
- Read: sleep the 40ms response delay, then
  `IOAVServiceReadI2C(service, 0x37, 0x51, buffer, REPLY_LEN)` and parse
  with `parse_get_vcp_reply`.
- Same settle delay, single retry, and read-back verify semantics as the
  Linux backend: after a set, read back and report a downgrade signal the
  same way `I2cDdcBackend` does (mirror its `DdcStatus` bookkeeping so
  `PolicyControl` can downgrade).

`MacAvServiceBackend` implements `DisplayControl` (get/set brightness via
VCP 0x10; gamma/modes/hdr return the same typed unsupported errors the Linux
DDC backend returns) and `DdcStatus`. Structure the FFI behind a
`trait AvTransport` seam (list_external_services, write, read) so unit tests
run with a fake transport on any OS; the real transport is
`#[cfg(target_os = "macos")]`.

## macos.rs

Build the same shape linux.rs builds: `PolicyControl::new(backend, gamma)`.
For gamma use a minimal `UnsupportedGamma` type (place it in avservice.rs or
macos.rs) implementing the trait bounds PolicyControl needs
(DisplayControl delegating enumerate to qol_windowing and returning typed
unsupported for everything else, GammaStateControl and LutProvider returning
typed unsupported / None), so `auto` resolves to DDC and a forced `gamma`
policy surfaces a clear unsupported error.

## Tests (fake transport, any OS)

- Canonical bytes: get request payload written for feature 0x10 is exactly
  `[0x82, 0x01, 0x10, 0xac]`, set 50% (raw scaling as i2c_ddc does) is
  `[0x84, 0x03, 0x10, 0x00, 0x32, 0x9a]`.
- A fake transport returning a valid 11-byte reply round-trips
  get_brightness to the expected percent.
- Count mismatch (2 services, 1 external display) yields the typed error
  naming both counts.
- Missing private symbol path yields the typed error carrying the symbol
  name (exercise by injecting a resolver fake, not by dlopen tricks).
- Read-back mismatch after set triggers the same downgrade signal shape as
  the Linux backend tests assert.

## Gate before committing

Run from the worktree root and paste real output in your report:

```
cargo test -p plugin-monitor
cargo fmt --check -p plugin-monitor
cargo clippy -p plugin-monitor --all-targets -- -D warnings
cargo check --target x86_64-unknown-linux-gnu -p plugin-monitor
```

If the linux target is not installed, report that instead of installing it.

Commit on this branch with a conventional message like
`feat(monitor): drive external displays over IOAVService DDC on macOS`.
NEVER add Co-Authored-By, "Generated with", or any AI attribution.
