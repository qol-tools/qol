# Spec: make macOS brightness actually work (DDC parity + gamma fallback)

Live verification against real monitors (Samsung Odyssey G5, ASUS VG248)
showed the avservice backend deviates from the battle-tested references
(MonitorControl Arm64DDC.swift, m1ddc i2c.m) in ways that broke or
endangered writes. One write wedged and later restarted the G5. Bring the
backend to reference parity, gate writes on transport safety, and add a
CoreGraphics gamma fallback so every display gets working brightness.

All work happens in this worktree on branch `monitor-macos-brightness`.
Scope: `plugins/monitor/src/monitor/backends/avservice.rs`,
`plugins/monitor/src/monitor/backends/i2c_ddc.rs` (only if a shared helper
needs a new pub(crate) item), `plugins/monitor/src/platform/macos.rs`, and
a new `plugins/monitor/src/monitor/backends/cg_gamma.rs`. Tests live with
their modules. Do not touch any other file. Code comments are banned; use
self-explanatory names.

## 1. DDC wire parity (avservice.rs)

Reference facts, verified from both reference sources:

- GET request payload: `[0x82, 0x01, vcp, chk]` where
  `chk = 0x6e ^ 0x82 ^ 0x01 ^ vcp` - the 0x51 data address is NOT part of
  the single-byte-request checksum. For vcp 0x10 the payload is
  `[0x82, 0x01, 0x10, 0xfd]`. Our current 0xac variant is wrong per both
  references; replace it. Keep set requests exactly as they are
  (`[0x84, 0x03, vcp, hi, lo, chk]`, chk seeded 0x6e ^ 0x51).
- Reads pass dataAddress/offset 0 to IOAVServiceReadI2C (MonitorControl),
  not 0x51.
- Pacing (MonitorControl defaults): sleep 10ms BEFORE every write; issue
  every write packet twice back-to-back (two write cycles); sleep 50ms
  before the read; up to 5 attempts with 20ms between attempts. Model
  this inside the backend, not the transport; keep Duration fields
  injectable so tests use zero.

Since the GET checksum no longer matches the Linux i2c framing, build the
avservice request bytes locally in avservice.rs (small free fns with the
canonical-byte tests below) instead of bending i2c_ddc.rs; keep reusing
parse_get_vcp_reply and percent_from_raw from i2c_ddc.

## 2. Registry-matched pairing (avservice.rs)

Replace positional pairing entirely; IOKit iteration order is unstable
across runs (observed live). Extend the AvTransport seam so discovery
returns, per service, the identity of the display it belongs to and the
transport class:

- Walk the io registry the way MonitorControl does: iterate services named
  `AppleCLCD2` / `IOMobileFramebufferShim` (each carries `DisplayAttributes`
  -> `ProductAttributes` with `ManufacturerID` string, `ProductID`,
  `SerialNumber`) followed by their `DCPAVServiceProxy` (Location
  External -> IOAVServiceCreateWithService).
- Pair a service to a DisplayHandle by matching (vendor, model, serial)
  from CGDisplayVendorNumber/CGDisplayModelNumber/CGDisplaySerialNumber of
  the CG display id parsed from the handle connector (`cg-{id}`), against
  ProductAttributes (`ManufacturerID` is the 3-letter EDID PNP string;
  convert it to the numeric vendor id: (((c1-'A'+1)<<10)|((c2-'A'+1)<<5)|
  (c3-'A'+1))). No positional fallback: an unmatched display gets a typed
  error naming its identity tuple.
- Transport class: walk the DCPAVServiceProxy parent chain for
  `EPICProviderClass` values. `AppleDCPMCDP29XX` or `AppleDCPPS190`
  anywhere in the chain marks the service ConverterRouted (the Mac's HDMI
  port); otherwise DirectDp. This host has two `AppleDCPPS190` entries, so
  the walk is testable live.

## 3. Write gating

DDC reads are allowed on every matched service. DDC WRITES are refused
with a typed error on ConverterRouted services (message: converter-routed
HDMI DDC writes are disabled because they can crash the display; the gamma
fallback owns brightness there). PolicyControl auto then falls through to
gamma for sets on those displays. This encodes the live incident: a set
wedged and then restarted the G5.

## 4. CG gamma fallback (cg_gamma.rs)

Implement `CgGammaControl` replacing UnsupportedGamma in macos.rs:

- DisplayControl: enumerate delegates to qol_windowing; get/set brightness
  scale the display's gamma table via CGGetDisplayTransferByTable /
  CGSetDisplayTransferByTable (link CoreGraphics in-file). set(value)
  captures the pristine table once per display (first touch), then writes
  pristine * (value/100) on the output entries; get returns the last set
  value (default 100) with BrightnessSource::Gamma. Clamp to a 10 percent
  floor so a gamma set can never black a display out.
- GammaStateControl + LutProvider: mirror the Linux gamma backend's shape
  (capture returns the pristine table, write_guarded restores it,
  restore() rewrites the pristine table and reports the outcome). Keep the
  state in-memory per process; persistence stays out of scope.
- Table math is pure and unit-tested (scaling, floor clamp, restore
  round-trip) behind a trait seam over the CG calls so tests run anywhere.

## 5. Tests

- Canonical bytes: get payload `[0x82, 0x01, 0x10, 0xfd]`; set payload
  unchanged `[0x84, 0x03, 0x10, 0x00, 0x32, 0x9a]`.
- Fake transport observes: every logical write issued twice, read happens
  after the response delay, retries stop on first valid reply.
- Pairing: services carrying identity tuples match displays regardless of
  order; swapped service order still pairs correctly; unmatched display
  errors with its tuple; PNP string conversion (SAM -> 0x4c2d).
- Gating: ConverterRouted service refuses set_brightness with the typed
  error and probe reports brightness_ddc false; DirectDp allows it.
- Gamma: scaling math, floor clamp, restore round-trip via fake CG seam.

## Gate before committing

Run from the worktree root and paste real output in your report:

```
cargo test -p plugin-monitor
cargo fmt --check -p plugin-monitor
cargo clippy -p plugin-monitor --all-targets -- -D warnings
cargo check --target x86_64-unknown-linux-gnu -p plugin-monitor
```

If the linux cross toolchain is missing, report how you verified instead.
Commit everything on this branch as ONE commit with a conventional message
like `fix(monitor): pair, pace, and gate macOS DDC and fall back to gamma`.
NEVER add Co-Authored-By, "Generated with", or any AI attribution.
Do NOT run the binary against real displays; the architect verifies live.
