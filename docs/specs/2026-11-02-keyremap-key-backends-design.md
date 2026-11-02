# Key Remap key backends, with Karabiner-Elements as one of them

Status: design, 2026-11-02. Not built yet.

## What changes for the user

On macOS, Key Remap gets a setting, Key backend, with four choices:

| Choice | What does the remapping | Survives Secure Input | Installs anything privileged |
| --- | --- | --- | --- |
| `auto` (default) | The best backend that is already installed and answering | Depends on what it picks | No |
| `event_tap` | Today's CGEventTap in the keyremap daemon | No | No |
| `hid_helper` | Our root helper plus the pqrs virtual keyboard driver (#43) | Yes | Yes, our own root LaunchDaemon |
| `karabiner_elements` | Karabiner-Elements, running rules that keyremap writes for it | Yes | No, Karabiner-Elements is installed and approved separately |

The user edits rules in Key Remap's settings exactly as today, whichever backend runs them.
`qol-keyremap doctor` says which backend is active, why it was chosen, and what is missing for the others.

## Why

On the managed work Mac, Secure Input is held often: kitty, `loginwindow` after unlock, and `PlatformSSOUIAgent` (the managed sign-in prompt) all show up in the logs.
While it is on, the event tap sees no keys, so every remap and qol hotkey dies until it turns off.

The `hid_helper` backend fixes that, but it adds an unsigned root binary that seizes the keyboards, and the company's endpoint protection flagged it and locked the account.
IT has approved Karabiner-Elements.
Karabiner-Elements already does everything `hid_helper` does below Secure Input: it seizes the keyboards as root, runs rules, and types through its own virtual keyboard.
If keyremap hands it the rules instead of running them itself, no qol code runs as root, reads the keyboard, or installs a driver.
All keyremap does is write a JSON file the user owns and call Karabiner's own CLI.

## Two kinds of backend

| Kind | Backends | Where `process_key_event` runs |
| --- | --- | --- |
| In-process | `event_tap`, `hid_helper` | In the keyremap daemon, on each key event |
| Compiled | `karabiner_elements` | Nowhere at runtime. The rules are compiled ahead of time into another remapper's format |

Both kinds sit behind one trait, so a future backend of either kind, for example a Windows or Linux remapper, plugs in the same way.

```rust
pub(crate) trait KeyBackend: Send + Sync {
    fn id(&self) -> BackendId;
    fn capabilities(&self) -> Capabilities;
    fn probe(&self) -> Availability;
    fn start(&self, environment: Arc<dyn KeyEnvironment>) -> Result<()>;
    fn apply(&self, config: &ResolvedConfig) -> Result<ApplyReport>;
    fn set_enabled(&self, enabled: bool) -> Result<()>;
    fn stop(&self) -> Result<()>;
    fn doctor(&self) -> Vec<DoctorCheck>;
}

pub(crate) struct Capabilities {
    pub survives_secure_input: bool,
    pub owns_key_events: bool,
    pub stamps_hotkey_markers: bool,
}

pub(crate) enum Availability {
    Ready,
    Missing { what: String, fix: String },
    Conflict { with: String },
}
```

- `probe` is cheap, needs no root, and never changes anything.
- `apply` is called on start and on every reload. In-process backends swap the resolved config; the compiled backend recompiles and rewrites. `ApplyReport` lists rules the backend could not express.
- `set_enabled` backs the tray's Toggle Remapping action.
- `stop` must undo the backend's effect on the keyboard before it returns, so two backends never remap the same key.
- `owns_key_events` tells the event tap to pass key events through untouched. The tap keeps mouse and scroll rules under every backend.

`KeyEnvironment`, which `hid_helper` already uses, becomes the shared environment: config, frontmost bundle id, `InputState`, layouts.
`StrategyCell` becomes a `BackendCell` holding a `BackendId`.

## Selection

Selection is a pure function, `select(preference, probes) -> (BackendId, Reason)`, tested as a table.

- `auto` picks the first `Ready` backend in this order: `karabiner_elements`, `hid_helper`, `event_tap`.
  `auto` never picks a backend that needs something installed; it only uses what is already there.
- An explicit choice that is not `Ready` falls back to `event_tap`, and the reason says so.
  Doctor warns and a one-time toast names what is missing and the fix.
- `hid_helper` and `karabiner_elements` both seize keyboards. If Karabiner-Elements is running, `hid_helper` probes as `Conflict`, which its helper already detects today.

Selection runs at daemon start, on reload, and on the existing 2-second retry loop whenever a probe result changes.
A switch is strictly ordered: `stop` the old backend, then `start` and `apply` the new one, then set `BackendCell`.

## The Karabiner-Elements backend

### What keyremap touches

| Thing | How | Root needed |
| --- | --- | --- |
| `~/.config/karabiner/karabiner.json` | Read, merge our block, write atomically | No |
| `/Library/Application Support/org.pqrs/Karabiner-Elements/bin/karabiner_cli` | `--version-number`, `--show-current-profile-name`, `--set-variables` | No |
| A datagram socket in keyremap's runtime dir | Receives hotkey markers from Karabiner's `send_user_command` | No |

Karabiner-Elements watches `karabiner.json` and reloads it on change, so a write is enough to apply new rules.
keyremap never installs, starts, updates or configures Karabiner-Elements itself, and never touches its devices, permissions or other profiles.

### Owning part of someone else's file

The user may have their own Karabiner rules, so keyremap owns exactly one block and nothing else.

- The file is parsed as untyped JSON (`serde_json::Value`), so fields keyremap does not know survive a round trip.
- keyremap writes one entry in `profiles[selected].complex_modifications.rules`, with `description` `"qol-keyremap (managed, edit rules in qol Key Remap)"`. It finds its block by that description and replaces it whole.
- Modifier key changes (`modifier_keys`, such as Caps Lock to Escape) go into the same profile's `simple_modifications`, because Karabiner applies those before complex rules, which is what the engine does today.
  `simple_modifications` entries carry no description, so a sidecar file in keyremap's state dir records which entries keyremap wrote. keyremap only removes entries the sidecar lists, and refuses to overwrite an entry the user wrote for the same key; doctor names the conflict.
- The sidecar also stores a hash of the last block written. If the block in the file no longer matches, someone edited it in Karabiner's UI; keyremap overwrites it and logs that once.
- Writes go to a temp file in the same directory, then rename. A symlinked `karabiner.json` (dotfiles) is written through to its target.
- Before the first write ever, keyremap copies the file to `karabiner.json.qol-keyremap.bak`.
- The selected profile is the one `--show-current-profile-name` reports. If the user switches profile, keyremap moves its block on the next sync and removes it from the old profile.

Karabiner's `assets/complex_modifications` import folder was rejected: every rule there must be enabled by hand in Karabiner's UI, so reloads would not apply.

### Turning rules on and off without rewriting

Every manipulator keyremap writes has the condition `variable_if qol_keyremap_active == 1`.

- The daemon sets `qol_keyremap_active` to 1 on start and on toggle-on, and to 0 on toggle-off and clean shutdown.
- Karabiner forgets variables when its core service restarts, so the rules go quiet until keyremap sets the variable again. The backend watches the core service's pid once a second and re-sets the variable when it changes.
- The result matches today: rules run only while the keyremap daemon is running and enabled.
  If the daemon crashes, the rules keep running until Karabiner restarts; the keyboard never dies.
- `stop` (switching to another backend) removes the block and keyremap's `simple_modifications` entries, then sets the variable to 0.
  `qol-keyremap karabiner-remove` does the same by hand.

### Compiling the rules

`compile(config: &ResolvedConfig, layout: &LayoutSnapshot) -> Compiled` is pure and returns the Karabiner JSON plus the rules it could not express.
Manipulators are emitted in the engine's phase order, because Karabiner, like `process_key_event`, uses the first manipulator that matches:

1. Global char swaps, 2. global char rules, 3. global key rules,
4. then, behind `frontmost_application_unless` with the excluded bundle ids: char swaps, char rules, key rules.

| keyremap | Karabiner |
| --- | --- |
| Key rule (single or batch) | One `basic` manipulator per key: `from.key_code` + `from.modifiers.mandatory`, `to` the target key and modifiers |
| Exact modifier match (`Modifiers::matches`) | `mandatory` with no `optional`, except `optional: ["caps_lock"]`, because Karabiner counts Caps Lock as a modifier and the engine ignores it |
| `alt` / `ralt` in `from_mods` | `option` (either side) / `right_option` |
| `global = false` | `frontmost_application_unless` with each excluded bundle id as an anchored, escaped regex |
| Char rule by key | `from` the key; `to` the key strokes `LayoutSnapshot::sequence_for` returns, dead key and space included |
| Char rule by character, char swap | `from` the key strokes that type the source character in the current layout; `to` as above |
| `modifier_keys` | `simple_modifications`, as above |
| Mouse and scroll rules | Not compiled. They stay on the event tap, which reads the modifiers from Karabiner's virtual keyboard |

Character rules depend on the active input source.
Each character manipulator carries `input_source_if` with the current input source id, and the existing one-second layout poll recompiles and rewrites when the input source changes.
A character with no key sequence in the layout is left out and reported, exactly as `layout_characters` reports it for `hid_helper`.

### qol hotkeys and the remap marker

qol-tray matches hotkeys against the combo the user pressed, not the one keyremap sent, using the marker keyremap stamps on its output events.
Under Karabiner, keyremap never produces the output event, so the tap cannot stamp it from its own record.

Each manipulator's `to` therefore starts with a `send_user_command` whose payload is the pressed combo and the output keycode, sent to keyremap's socket through the `endpoint` field.
The backend writes each payload into the existing `MarkerBook`, and the tap stamps the marker when the output key arrives, as it does for `hid_helper`.

This depends on the datagram reaching keyremap before the key event reaches the tap. The probe below measures it.
If it is not reliably first, the tap holds a key-down whose keycode is the output of some compiled rule for at most 5 ms while it waits for the marker.

## Settings, CLI and manifest

- `qol-config.toml`: `key_backend`, a `select` in General, macOS only, default `auto`, plus a `status` field showing the active backend and its reason.
- `qol-keyremap backend` prints the active backend, the reason, and every backend's probe.
- `qol-keyremap karabiner-rules` prints the compiled block without writing. `karabiner-sync` writes now. `karabiner-remove` removes keyremap's block and entries.
- `plugin.toml` gains a second `[[dependencies.system]]` entry, Karabiner-Elements, with the minimum version the probe shows supports `send_user_command` with `endpoint`.

## Doctor

| Check | Warns when | Fix it names |
| --- | --- | --- |
| `key_backend` | The chosen backend is not the active one | The missing piece and its fix |
| `karabiner_installed` | The app or `karabiner_cli` is missing or older than the minimum | Install Karabiner-Elements |
| `karabiner_running` | The core service is not running or its driver is not active | Open Karabiner-Elements and finish its setup |
| `karabiner_rules` | The block is missing from the selected profile or is out of date | `qol-keyremap karabiner-sync` |
| `karabiner_coverage` | A rule could not be compiled | Names the rule and why |
| `karabiner_conflicts` | A user `simple_modifications` entry sits on a key keyremap wants | Names the key |

Karabiner checks only run when the chosen backend is `karabiner_elements`, or under `auto` when Karabiner-Elements is installed.
The Secure Input warning keeps its two texts: under a backend that survives Secure Input it says only qol hotkeys are paused.

## Source layout

```text
plugins/keyremap/src/platform/macos/input/
  mod.rs                     BackendId, BackendCell, InputState, MarkerBook
  selection.rs               select(preference, probes), pure
  backends/
    mod.rs                   KeyBackend, Capabilities, Availability, the registry
    event_tap.rs             unchanged behaviour, behind the trait
    hid_helper/              today's backends/virtual_hid, renamed
    karabiner/
      mod.rs                 the backend: probe, start, apply, toggle, stop
      compile.rs             ResolvedConfig + LayoutSnapshot -> Compiled, pure
      document.rs            karabiner.json merge, sidecar, atomic write, pure apart from io
      cli.rs                 karabiner_cli wrapper
      markers.rs             send_user_command receiver -> MarkerBook
```

The root-side `hid_helper/` and `virtual_hid/client/` stay where they are; only the user-side backend moves.
`PlatformAdapter` gains `backend()`, `karabiner_sync()` and `karabiner_remove()`; other OSes return the typed "macOS only" error.

## Probe first

Before any code, a throwaway probe on this Mac with Karabiner-Elements installed by IT:

1. Hand-write one rule (Ctrl+C to Cmd+C, behind `variable_if`) into `karabiner.json`, turn on kitty's Secure Keyboard Entry, and confirm it works and the variable toggles it.
2. Add `send_user_command` with a custom `endpoint` in a user directory, and measure over 200 presses whether the datagram reaches a listener before the key event reaches a CGEventTap.
3. Confirm a rename-over write is picked up, and that Karabiner's UI keeps unknown fields and our block when the user edits other rules.
4. Record the Karabiner-Elements version, which becomes `min_version`.

## Testing

Unit tests, on any OS unless noted:

- `compile`: golden JSON for the default config; phase order; `caps_lock` optional; `right_option`; exclusion regex escaping; a character rule through US and Danish layout fixtures; a character with no sequence lands in the report.
- `document`: foreign rules and unknown fields survive; our block is replaced, not duplicated; a second sync with no change writes nothing; profile switch moves the block; a user `simple_modifications` entry is never overwritten; missing file and missing profile.
- `select`: every preference against every probe combination, including `Conflict`.
- The variable lifecycle as a state machine: start, toggle, clean shutdown, core-service restart.
- A fake backend proving a switch always calls `stop` on the old one before `start` on the new one.

By hand on this Mac, after the probe: set Key backend to Karabiner-Elements, turn on Secure Keyboard Entry in kitty, then check Ctrl+C becomes Cmd+C in a browser, Right Option+2 types `@`, the `rightbracket` rule types `~`, Toggle Remapping stops and restarts them, quitting qol-tray stops them, and switching back to `event_tap` removes the block from `karabiner.json`.

## Delivery

1. The probe.
2. The trait, `BackendCell` and `select`, with `event_tap` and `hid_helper` behind them and no behaviour change.
3. `compile` and `document`, with `karabiner-rules` to inspect output.
4. The Karabiner backend, the setting, doctor checks and `sync`/`remove`.
5. Hotkey markers through `send_user_command`.
6. The Key Remap skill and README describe the backends.

## Out of scope

- Compiling mouse and scroll rules for Karabiner.
- Pre-compiling character rules for every enabled input source instead of recompiling on switch.
- qol-tray's own hotkeys under Secure Input. They could later be compiled into Karabiner rules that `send_user_command` straight to the tray.
- Removing `hid_helper`. It stays for Macs without Karabiner-Elements.
