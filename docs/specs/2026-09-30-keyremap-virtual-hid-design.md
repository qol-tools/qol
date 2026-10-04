# Key Remap survives Secure Input on macOS

Status: design, approved in conversation on 2026-09-30.

## What changes for the user

Key Remap keeps working while any app holds macOS Secure Input.
On 2026-09-30 kitty held Secure Input and every keyremap rule and every qol hotkey went dead, with no error anywhere.
After this change, keyboard rules run below Secure Input, so a stuck password field or a terminal's Secure Keyboard Entry no longer switches them off.

The user installs one extra thing once: the pqrs virtual keyboard driver.
`qol-keyremap doctor` says whether it is present and prints the exact command that fixes what is missing.
Without the driver, keyremap behaves exactly as it does today.
Either way, when an app turns on Secure Input, keyremap shows a warning that names the app and says what stopped working.

## Why the event tap cannot do this

Secure Input hides the keyboard event stream from every event tap in the session.
It does this on purpose, and no API lets one app switch off another app's Secure Input.
Only a process that reads the keyboard hardware directly, below the window server, still sees the keys.

## Evidence

A throwaway probe ran on this Mac on 2026-09-30 with kitty's Secure Keyboard Entry on.

- A root process opened the physical keyboards through `IOHIDManager` and received all 40 key events the user typed.
- The same process connected to the pqrs daemon, created the virtual keyboard, and typed `q`, which appeared in the focused window.
- The company MDM profile allows system extensions from team `G43BCU2T37` (pqrs.org), and the driver activated without a prompt.
- Every character in the current `char_rules` resolves in the Danish layout: 11 as one key combination, and `~` as a dead key followed by space.

## The dependency

Karabiner-DriverKit-VirtualHIDDevice, package 8.6.0, driver 1.8.0, client protocol 7.
The package is Unlicense and the client headers are Boost Software License 1.0.
It is Apple-notarized and signed by Developer ID Installer: Fumihiko Takayama (G43BCU2T37).
It is not Karabiner-Elements, and keyremap never installs or talks to Karabiner-Elements.

The driver only accepts commands from a root process, through the pqrs daemon's socket at `/Library/Application Support/org.pqrs/tmp/rootonly/karabiner_virtual_hid_device_service.sock`.
That is why keyremap gains a root helper.

## Strategies

Key input is a strategy, chosen at daemon start and again whenever the helper appears or disappears.

| Strategy | Chosen when | Survives Secure Input |
| --- | --- | --- |
| `virtual_hid` | The helper answers on the current protocol version | Yes |
| `event_tap` | Anything else | No |

`event_tap` is today's implementation, unchanged.
Both strategies run the same `process_key_event` engine, so a rule means the same thing under either.

## Secure Input warning

The daemon checks `IsSecureEventInputEnabled()` once a second.
When it turns on, the daemon reads the holder's pid from `CGSessionCopyCurrentDictionary()`, because the `ioreg` value can be stale, and resolves the app name.
It then pushes one warning toast through `qol_runtime` `send_notification` at level `Warn`:

- Under `event_tap`: "Key Remap and qol hotkeys are paused: <App> has turned on Secure Input." The body says to quit the app or turn off its secure entry, and that installing the virtual keyboard driver stops this happening.
- Under `virtual_hid`: "qol hotkeys are paused: <App> has turned on Secure Input." Key Remap keeps working, and the body says so.

It warns once per episode.
A new episode starts only after Secure Input has been off, or when the holder changes.
Doctor's `secure_input` check reports the same state on demand.

A layout-file fallback was tried on 2026-09-30 and rejected.
Switching to a generated layout needed manual steps in System Settings and broke typing in the qol launcher.

## Architecture

```text
physical keyboard
  -> hid_helper (root, LaunchDaemon)      seizes keyboards, forwards raw key events
  -> keyremap daemon (user)               rules, frontmost app, layout lookup
  -> hid_helper                           holds the virtual keyboard report
  -> pqrs daemon (root) -> DriverKit virtual keyboard -> window server
```

The helper is deliberately thin.
It owns no rules and knows nothing about apps.
All remapping stays in the existing engine, `process_key_event` in `app/remap.rs`, which already takes a modifier set, a keycode and a bundle id.

Mouse and scroll rules stay on the event tap.
Secure Input only hides keyboard events, so the tap still sees clicks and scrolls, and their modifier flags come from the virtual keyboard's state.
While the virtual HID backend is active, the tap stops remapping key events so no key is remapped twice.
It still stamps the remap marker (`qol_runtime::keyremap_marker`) on the key events the daemon produced, because qol-tray matches hotkeys against the combo the user pressed, not the one keyremap sent.
The daemon records each remapped output key with the combo that caused it, and the tap looks the event's keycode up in that record.

### Keyboard identity

The virtual keyboard is created with the HID country code of the first seized physical keyboard, so macOS gives it the same ANSI, ISO or JIS type.
Otherwise the key left of 1 and the key left of Z swap places on an ISO keyboard.
The daemon's HID usage to keycode lookup applies the same ISO swap macOS applies, decided by `KBGetLayoutType(LMGetKbdType())`.

### Top row and Caps Lock

Seizing an Apple keyboard bypasses the Apple driver that turns F1 to F12 into brightness, media and volume keys.
For devices with Apple's vendor id (0x05AC), the daemon does that translation itself and honours the "Use F1, F2, etc. keys as standard function keys" setting (`com.apple.keyboard.fnState`).
It uses the same table Karabiner-Elements ships for Apple Silicon keyboards.
Other keyboards' function keys pass through as function keys, as they do today.

The seized keyboard's Caps Lock light no longer follows macOS on its own.
After each Caps Lock press the daemon reads the system Caps Lock state and tells the helper, which sets the light on every seized keyboard.

### Source layout

Everything is macOS-only and lives under `plugins/keyremap/src/platform/macos/`, following `qol-arch-code`.

| Path | Owns |
| --- | --- |
| `input/mod.rs` | The key input capability: selects `virtual_hid` when the helper answers, otherwise `event_tap` |
| `input/backends/event_tap.rs` | Today's key handling, moved out of `tap.rs`; the tap keeps mouse and scroll |
| `input/backends/virtual_hid.rs` | The user side: talks to the helper, feeds the engine, emits output keys |
| `hid_helper/` | The root side: seize, forward, virtual keyboard report, watchdog |
| `virtual_hid/client/` | A Rust port of the pqrs client protocol 7 and its socket framing |
| `layout/` | Reverse lookup from a character to key presses through the current input source |

The HID usage to macOS keycode table goes into `qol_hotkeys::macos_keycode`, which already owns macOS key identity.

`PlatformAdapter` gains `hid_helper`, `install_hid_helper` and `uninstall_hid_helper`.
Linux, Windows and the fallback adapter return a typed "macOS only" error.

### Root helper

It is the same `qol-keyremap` binary, run as `qol-keyremap hid-helper`.

`sudo qol-keyremap install-hid-helper` does the privileged setup:

1. Copies the running binary to `/Library/PrivilegedHelperTools/com.qol-tools.keyremap.hid-helper`, owned by `root:wheel`, mode 0755.
   A root job must never run the plugin binary in place, because that file sits in a user-writable directory.
2. Writes a LaunchDaemon for that copy.
3. Writes a LaunchDaemon for the pqrs daemon, unless one is already registered.
4. Loads both and activates the driver extension.

`sudo qol-keyremap uninstall-hid-helper` reverses steps 1 to 3.
It leaves the driver package installed, since other software may use it.

The helper and the keyremap daemon share a protocol version.
After a keyremap update, a mismatched helper is refused, keyremap falls back to the event tap, and doctor names `install-hid-helper` as the fix.

### Helper socket

The helper listens on `/var/run/com.qol-tools.keyremap.hid-helper.sock`.
The socket is owned by the console user with mode 0600, and every connection's peer uid (`getpeereid`) must equal the console user's uid.
When the console user changes, the helper drops the connection and re-creates the socket for the new user.

Messages, both directions:

- Helper to daemon: `key { usage_page, usage, pressed, apple }`, one per HID value change on the keyboard and Apple top case pages. `apple` is true when the device has Apple's vendor id.
- Daemon to helper: `emit { usage_page, usage, pressed }`, `caps_lock_light { on }` and `heartbeat`.
- Either side may open a short `status` connection instead of a session; the helper answers with its protocol version, whether the virtual keyboard is ready, the seized devices and any conflicts. Doctor uses this, because the pqrs socket is root-only.

### Safety rule

The keyboard must never die.

- The daemon sends a heartbeat every 50 ms.
- If the helper hears nothing for 200 ms, it releases every seized keyboard, so the physical keys reach macOS untouched.
- It seizes again only after the daemon reconnects and a heartbeat arrives.
- If the pqrs daemon or the virtual keyboard goes away, the helper releases the keyboards at once.
- If another process already seized a keyboard, for example Karabiner-Elements, the helper leaves it alone and reports it; doctor shows a warning naming the conflict.

### Which keys are forwarded

The helper seizes whole keyboard devices, so it must forward everything they produce, not only letters.
That covers the keyboard page, the consumer page (media keys) and the Apple vendor pages (Fn and the top-case keys).
The consumer page and the Apple vendor keyboard page go straight back out through the matching pqrs report type.
The keyboard page and the Apple top case page (which carries Fn) go to the daemon.

## Character output

The virtual keyboard can press keys but cannot insert text.
A `Char` action (from `char_rules`, `char_swaps` or a key rule's `to_char`) is turned into key presses by the `layout` module:

1. Search the current input source with `UCKeyTranslate` for a single key plus modifiers that produces the text.
2. If none exists, search for a dead key followed by space.
3. Emit that sequence with the user's physical modifiers released for its duration, then restore them.

The lookup table is rebuilt when the input source changes.
A character with no key sequence in the active layout is skipped, logged as a warning once per character, and reported by doctor with the rule that names it.

`char_swaps` needs the character a key would type.
The daemon computes it with the same `UCKeyTranslate` call from the incoming usage and modifiers.

## Formalizing the dependency

`plugin-api` gains `[[dependencies.system]]` next to `[[dependencies.binaries]]`:

```toml
[[dependencies.system]]
name = "Karabiner-DriverKit-VirtualHIDDevice"
platforms = ["macos"]
min_version = "8.6.0"
license = "Unlicense"
url = "https://github.com/pqrs-org/Karabiner-DriverKit-VirtualHIDDevice"
```

Manifest validation checks the fields are present and `min_version` parses as a semver version.
keyremap's doctor reads its own manifest entry, so the minimum version has one source of truth.
Showing system dependencies in the tray's plugin page is out of scope for this change.

## Doctor

`qol-keyremap doctor` gains these checks:

| Check | Fails when | Fix it names |
| --- | --- | --- |
| `virtual_hid_driver` | The package is missing, older than `min_version`, or the extension is not activated | The package URL, then `install-hid-helper` |
| `virtual_hid_daemon` | The pqrs daemon socket does not answer | `sudo qol-keyremap install-hid-helper` |
| `hid_helper` | The helper is not installed, not running, or on another protocol version | `sudo qol-keyremap install-hid-helper` |
| `secure_input` | Warns when an app holds Secure Input, and says which strategy is active | Names the app and its pid |
| `layout_characters` | A character rule has no key sequence in the active layout | Names the rule |

A missing driver is a warning, not a failure, because the event tap still works outside Secure Input.

## Testing

Unit tests, all on macOS CI unless noted:

- pqrs frame encoding and decoding, including partial reads and oversized frames (any OS).
- The HID usage to keycode table: every usage the engine names maps back to itself (any OS).
- Helper watchdog as a pure state machine: heartbeat, timeout, reconnect, virtual keyboard loss (any OS).
- Layout lookup against the built-in US and Danish layouts: all 12 current characters, a dead-key character, and a missing character.
- Backend selection: helper answers, helper absent, protocol mismatch.
- Secure Input warning as a pure state machine: one toast per episode, a new toast when the holder changes, and the text for each strategy (any OS).
- `[[dependencies.system]]` parsing and validation.

End to end, by hand on this Mac, repeating the 2026-09-30 probe through the real code:
turn on kitty's Secure Keyboard Entry, then check Ctrl+C becomes Cmd+C, Right Option+2 types `@`, and the `rightbracket` rule types `~`.
Then kill the keyremap daemon and check the keyboard keeps typing within 200 ms.
Then stop the helper, turn Secure Keyboard Entry on in kitty, and check the toast names kitty and says Key Remap is paused.

## Out of scope

- Showing system dependencies in the tray UI.
- Moving mouse and scroll rules onto the virtual pointing device.
- The tray's own hotkeys, which also die under Secure Input; they can adopt the helper later.
- Linux and Windows, where Secure Input does not exist.
