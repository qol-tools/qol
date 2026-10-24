# qol as host defaults

Goal: every job the host OS hands to a default app or a built-in tool, and that a qol plugin can do, can be handed to that plugin instead, on Linux, Windows and macOS.
Opening folders in qol explorer is one case; pressing Super for the qol launcher or PrtSc for qol shot is the same idea.
This is the design; nothing is built yet.
The evidence behind every claim is in `2026-10-24-default-apps-research/` (one file per OS plus prior art), each claim tagged verified or inferred with its source.

## What the research changed

1. No OS has one "default" per job. Each job is a set of stores that different callers read: on this Mint box the default terminal has four stores with three answers, and "open a folder" and "show a file in its folder" are separate stores on all three OSes.
2. Taking a system key is always two steps on every OS: free the OS binding (a stored setting qol must restore), then answer the key (a live intercept that ends with the process).
3. Whether qol may take a job differs by OS version, desktop and granted permissions, so the answer is probed live, never a static table.
4. Some handler changes cannot be made or undone silently: Windows requires a click in Settings for file types and URL schemes, and macOS 26.4+ asks for consent on every content-type change, including the change back. Portable mode cannot promise a silent restore for those, so they are Resident only.
5. Corrections to the first draft: Cmd+Tab is a symbolic hotkey (ids 1 and 2) and is freed like Spotlight, not only through an event tap; macOS refuses to bind `public.folder` to anything but Finder on 26.6.2; `PrintScreenKeyForSnippingEnabled` rests on forum reports and needs a guest test.

## Roles

A role is one job, named the same on every OS. Business code, the settings page and plugin manifests only ever name roles.

| Role | Kind | qol plugin |
|---|---|---|
| `folder.open` | handler | explorer |
| `folder.reveal` | handler | explorer |
| `file.type(<type>)` | handler | a future viewer or editor |
| `url.scheme(<scheme>)` | handler | `qol://` deep links, future plugins |
| `url.web`, `mail` | handler | none today |
| `terminal` | handler | none today |
| `slot.launcher` | slot | launcher |
| `slot.window_switcher` | slot | alt-tab |
| `slot.screenshot` | slot | shot |
| `slot.clipboard_history` | slot | clipboard |
| `slot.dictation` | slot | voice |
| `slot.window_snap` | slot | window-actions |
| `slot.tray_applet` | additive | bluetooth, sound |
| `slot.lock` | route only | none; qol's key calls the host lock |

`slot.tray_applet` adds qol's applet beside the host's and hides the host's where the desktop allows it (the Blueman takeover today); `slot.lock` is never claimed.

## Report: what the host allows, per role

Each OS backend answers `probe(role)` with one report:

- `state`: `Silent` (write and read back with no OS UI), `NeedsConfirmation` (the OS shows its own consent or Settings page, or a Wayland portal dialog), `NeedsPermission` (a one-time grant such as macOS Accessibility; then `Silent`), `Impossible` (with the reason), or `Unknown` (the probe failed; never treated as success, shown as a doctor warning).
- `reach`: `Full`, or `Partial` with the callers that bypass the default (Windows: `explorer.exe` launched directly, Control Panel, file dialogs).
- `restore`: `Exact` (previous value recorded and written back), `OnExit` (a live intercept that ends with the process), or `NeedsConfirmation` (the change back also asks the user).
- `lifetimes`: which of `PortableSession` and `ResidentPolicy` the role can honour. Portable requires `restore` to be `Exact` or `OnExit`.

The settings page offers only what the report allows and prints the residual in place: "Nemo still opens folders from the desktop", "Windows asks you to confirm in Settings", "only when kept on this computer".
This is Chromium's model (`SET_DEFAULT_UNATTENDED`, `SET_DEFAULT_INTERACTIVE`, `UNKNOWN_DEFAULT`) extended with reach, restore and lifetime.

## Best-case report per OS

To be confirmed by the live probe; `I` marks a cell that needs a guest test first.

| Role | Linux (Cinnamon, X11) | Windows 11 | macOS 13 to 26 |
|---|---|---|---|
| `folder.open` | Silent, Partial (Nemo and the desktop open folders themselves) | Silent via `Directory`/`Drive` verbs, Partial (direct `explorer.exe`, Win+E, Control Panel, Desktop, file dialogs) | Impossible (Finder owns `public.folder`) |
| `folder.reveal` | Impossible while `nemo-desktop` owns `org.freedesktop.FileManager1`; activation-file shadow is `I` | Impossible (callers launch Explorer directly) | Silent through `NSFileViewer`, Partial (cooperating Cocoa apps), `I` on 15+ |
| `file.type`, `url.web`, `mail` | Silent (`mimeapps.list`) | NeedsConfirmation (Settings page), Resident only | NeedsConfirmation on 26.4+ and for http, Resident only |
| `url.scheme` (qol-owned scheme) | Silent | Silent (HKCU classes) | Silent if the scheme is in the bundle's Info.plist |
| `terminal` | Silent across 4 stores, Partial | Silent (`HKCU\Console\%%Startup`), `I` | Impossible (no system default) |
| `slot.launcher` | Silent (menu applet overlay key, exists) | Silent via the keyboard hook; Partial (Start button click) | Silent (symbolic hotkey 64) |
| `slot.window_switcher` | Silent (exists) | Silent via the hook | Silent (symbolic hotkeys 1 and 2, private SkyLight call) |
| `slot.screenshot` | Silent (media-keys) | Silent (PrtSc toggle `I`, Win+Shift+S hook only) | Silent (symbolic hotkeys 28 to 31, 184) |
| `slot.clipboard_history` | Silent (no host tool) | Silent via the hook (Win+V) | Silent (no host key) |
| `slot.dictation` | Silent (no host tool) | Silent via the hook (Win+H) | `I` (preference key not found) |
| `slot.window_snap` | Silent (wm keybindings, exists) | Silent via the hook (FancyZones precedent) | `I` on 15+ |

Other desktops: GNOME (mutter and media-keys schemas) is verified; KDE (kglobalaccel) and XFCE (xfconf) are partly forum-sourced.
On Wayland no client can grab a key: qol frees the compositor binding, then binds through the GlobalShortcuts portal (GNOME 48+, KDE, Hyprland), which makes every slot `NeedsConfirmation`, or `Impossible` where the compositor lacks the portal.
On Windows the hook loses to elevated windows and the secure desktop, and Win+L and Ctrl+Alt+Del are never interceptable.
On macOS secure event input (password fields, a leaked holder such as 1Password) stops event taps system-wide, so slots answer through Carbon hotkeys, which need no permission, and an event tap only for what Carbon cannot express; doctor warns while secure input is held.

## One opener per OS for handlers

qol registers one handler identity per host instead of one per plugin: `qol-open.desktop` on Linux, ProgID `qol.open` with `Capabilities` and `RegisteredApplications` under HKCU on Windows, and a small `qol-open.app` bundle on macOS whose Info.plist declares every type and scheme qol can handle.
The opener forwards the request to the tray, which routes it to the plugin holding the role.
Switching `folder.open` from explorer to another plugin is then internal to qol and never touches host state or shows a Windows or macOS prompt.
The registration itself is a claim with a lifetime; removing it is the restore Microsoft recommends (Windows ignores a default whose ProgID is gone).
A bare plugin binary can never be a macOS handler, so the bundle is required there.

## Claim protocol

Every write to a host store is a claim; live intercepts (the Windows hook, Carbon hotkeys, X11 grabs, portal bindings) are not, because they end with the process.

1. Probe; refuse unless the report allows the requested lifetime.
2. Snapshot the store: the exact previous value, or "absent" (a schema default, a missing plist id, a missing registry value).
3. Record the claim before writing: role, store, key, previous, applied, plugin, lifetime.
4. Write, then read back. A mismatch drops the claim and reports `Refused` with what the OS kept (UCPD, MDM, a Group Policy association file).
5. For `NeedsConfirmation`, the claim is `Pending` with a watch on the exact key or plist and a deadline (Chromium uses two minutes); it becomes `Applied` only when the read-back matches.
6. Release compares the current value with `applied`: equal restores `previous` (or deletes the key when it was absent); different means the user changed it, so the claim is dropped and the value left alone.
7. At start, outstanding `PortableSession` claims from a crash are replayed through step 6; `ResidentPolicy` claims are reconciled through the policy journal.

This generalizes `apps/tray/src/hotkeys/takeover/ledger.rs` (previous, applied, Rewrite, Abandon, Quarantine) and fixes the gap in `qol_host_fixes::takeover`, which today stores only a marker and a restore hint.
It never restores to a constant baseline, unlike alt-tab-macos, which re-enables Cmd+Tab on quit even when the user had it off.

## Code layout

One shared capability library, `libs/host-defaults` (`qol-host-defaults`), headless first, consumed by the tray.

```
libs/host-defaults/src/
  lib.rs                 facade: Role, Report, Claim, HostDefaults trait
  role/                  role catalog and manifest parsing of [[default_for]]
  report/                state, reach, restore, lifetime types
  claims/                the claim protocol over qol_host_fixes::takeover records
  handler/
    mod.rs               handler roles over the stores below
    backends/
      mimeapps.rs        XDG mime-apps cascade, desktop-specific lists
      dbus_activation.rs FileManager1 activation file
      shell_verb.rs      Windows Directory/Drive verbs
      registered_app.rs  ProgID, Capabilities, RegisteredApplications, Settings deep link
      launch_services.rs NSWorkspace setDefaultApplication with consent
      file_viewer.rs     NSFileViewer
  slot/
    mod.rs               free-then-answer for slot roles
    backends/
      dconf.rs           Cinnamon, GNOME keybindings
      spices.rs          Cinnamon applet JSON
      kglobalaccel.rs    KDE
      xfconf.rs          XFCE
      registry_value.rs  PrtSc toggle, DisabledHotkeys
      symbolic_hotkeys.rs macOS ids 1, 2, 28 to 31, 64, 184
      global_shortcuts.rs Wayland portal
  platform/
    mod.rs               cfg wiring only
    linux/mod.rs         desktop and session detection, role to store routes
    windows/mod.rs       build detection, role to store routes
    macos/mod.rs         version detection, role to store routes
    fallback/mod.rs      every role Impossible("unsupported OS")
```

The platform modules only pick routes: for a role they return the ordered stores to free and the intercept that answers.
Pure file and plist parsing in the backends compiles and is tested on every host; OS calls (registry, SkyLight, NSWorkspace, D-Bus) sit behind each backend's own `platform/` facade.
The intercepts stay where they live: the Windows hook in `libs/platform/src/native/platform/windows/hooks.rs`, Carbon hotkeys and taps with the hotkey registry, X11 grabs and evdev in the tray's hotkey backends.
`apps/tray/src/hotkeys/takeover/` becomes a consumer: its dconf and spices code moves into `slot/backends/`, its ledger into `claims/`.

## Surfaces

- Plugins declare roles in `plugin.toml`: `[[default_for]] role = "slot.window_switcher"`. A plugin never writes host state; the tray claims on its behalf. Two plugins declaring one role: the user picks on the Defaults page.
- `qol-tray defaults list | probe <role> | claim <role> --plugin <id> | release <role>`, each with `--json`, mirroring `qol-tray resident-policy`.
- `qol-tray doctor` gains a `host_defaults` check: every claim with its state, claims whose plugin or opener is gone, values the user changed since, policy that will undo a claim (Windows `DefaultAssociationsConfiguration`, a macOS MDM profile), Wayland without the portal, macOS secure input held.
- The Defaults settings page (design round 154) renders the reports; rows for `NeedsConfirmation` roles say what the OS will ask, and Resident-only rows say so.
- `ResidentPolicy` claims add a `host_defaults` variant beside `nvidia` in `libs/host-fixes/src/policy`.

## Order

1. Library skeleton with roles, reports, the claim protocol and every OS answering honestly (fallback and unbuilt roles report `Impossible` or `Unknown`), plus `qol-tray defaults` and the doctor check.
2. Linux Cinnamon: move the existing takeover in as `slot.*` roles, add `folder.open` through `mimeapps.list` and `qol-open.desktop`.
3. Windows slots through the hook and the PrtSc toggle, then `folder.open` verbs once explorer runs on Windows.
4. macOS slots through symbolic hotkeys and Carbon hotkeys.
5. Resident-only handlers with confirmation: the Windows opener registration and Settings flow, the macOS `qol-open.app` and consent flow.
6. GNOME, KDE and XFCE routes, then Wayland through the portal.

## Guest tests before each step locks

- Linux: does a user D-Bus activation file for `FileManager1` beat the system one on dbus-daemon and dbus-broker; can qol own it on Cinnamon while `nemo-desktop` runs; Plasma 6 shortcut conflicts; GLib's `xdg-terminal-exec` path on Mint.
- Windows 11 24H2 and 25H2: the `PrintScreenKeyForSnippingEnabled` name and effect; `RegisterHotKey` results for Win+V, Win+H, Win+Shift+S, Win+Arrow, Alt+Tab; whether removing qol's ProgID raises "An app default was reset"; `AssocQueryStringW` against `UserChoiceLatest` after a Settings change.
- macOS 13, 14, 15 and 26: `activateSettings -u` applies without logout; the dictation and tiling keys; `NSFileViewer` honoured by reveal calls.
