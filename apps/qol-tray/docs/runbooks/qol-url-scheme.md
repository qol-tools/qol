# Runbook: `qol://` URL scheme manual click-test

The `qol://` scheme (deep-linking Phase 4) is a GUI OS integration. It cannot
run in CI or Playwright - the steps below are the manual acceptance test. All
pure logic (URL parse, plist/desktop content, courier forwarding) is unit-tested
under `cargo test`; this runbook covers only the LaunchServices / openURLs path.

## How it works

- The installed bundle declares the scheme: macOS `CFBundleURLTypes` (Info.plist),
  Linux `MimeType=x-scheme-handler/qol;` (`.desktop`).
- A clicked `qol://<route>` is delivered:
  - **Linux**: as argv `%u` -> `main::try_url_courier`.
  - **macOS**: via AppKit `application:openURLs:` (the `QolUrlDelegate` in
    `src/tray/platform/macos.rs`), which re-execs the binary with the URL,
    re-entering `try_url_courier`.
- The courier forwards: if the daemon is running it navigates the open tab
  (`POST /api/navigate`, Phase 3) or opens the prefilled route; on a cold launch
  it opens the prefilled route once the server is listening.

## macOS

1. Install: build + place `QoL Tray.app` in `~/Applications` (or `/Applications`).
   `register_application` writes the plist and runs `lsregister -f`.
2. Confirm the scheme is bound:
   ```
   /System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister -dump | grep -A3 'qol:'
   ```
   Expect a `bindings: qol:` line pointing at `com.qol-tools.qol-tray`.
3. **Warm test** (qol-tray running, a tab open at any page):
   `open 'qol://shortcuts/add?type=url&url=https://example.com&name=Click%20Test'`
   Expect: the open tab live-navigates into the prefilled Add Shortcut form, no
   new tab, no reload.
4. **Warm test, no tab open**: same command. Expect: a browser tab opens on the
   prefilled form.
5. **Cold test** (quit qol-tray fully first): same command. Expect: qol-tray
   launches, then a browser tab opens on the prefilled form. The URL is not lost
   to the single-instance check.

## Linux

1. Install (writes `~/.local/share/applications/qol-tray.desktop` with
   `MimeType` + `Exec=... %u`, runs `update-desktop-database` + `xdg-mime`).
2. Confirm: `xdg-mime query default x-scheme-handler/qol` -> `qol-tray.desktop`.
3. Warm / cold tests: `xdg-open 'qol://shortcuts/add?type=url&url=https://example.com&name=Click%20Test'`
   with the same expectations as macOS.

## Notes

- LaunchServices may route a warm URL to the live instance (delegate) OR spawn a
  fresh process (argv courier). Both forward to the daemon and are safe.
- A `qol://` link may navigate or open the prefilled creation form; it never
  auto-runs a shortcut or fires a plugin action. Headless create + toast (create
  the record without ever showing the form) is a deferred follow-up - it needs a
  Rust id-derivation that stays in parity with `ui/views/shortcuts/derive-id.js`.
