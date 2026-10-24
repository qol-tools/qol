# Prior art: cross-platform default-app and slot takeover

Research for the qol "host defaults" facade. Scope: how other software models and changes OS defaults, how it detects what the OS refuses, how it restores, and where it fails.
Every claim is tagged **V** (verified: official docs, source code I read, or a reproducible report) or **I** (inferred or reported secondhand), with a source number from the list at the end.
Not researched: Flutter (no source read; assume it only opens URLs, tag **I**), Alfred (no primary source read), Ditto internals (no primary source read), Wayland compositors beyond what the portal docs say.

## 1. The main findings

1. The best prior art for the facade's states already exists in Chromium. It models "can I set this?" as three values (`SET_DEFAULT_NOT_ALLOWED`, `SET_DEFAULT_UNATTENDED`, `SET_DEFAULT_INTERACTIVE`) and "is it mine?" as four (`NOT_DEFAULT`, `IS_DEFAULT`, `UNKNOWN_DEFAULT`, `OTHER_MODE_IS_DEFAULT`). It also lets the caller forbid prompting (`set_interactive_permitted(false)`). **V** [1]
2. No cross-platform library both sets and restores defaults. Electron comes closest and its return values are weak: a bare boolean, `true` on Windows Store apps even when the key is unusable, and `removeAsDefaultProtocolClient` unimplemented on Linux. **V** [2][3][4]
3. Every serious replacement tool either asks the user to do the last step (Raycast, Flameshot, ShareX, Firefox, Chromium on Windows and macOS) or rewrites state the OS protects and breaks on updates (SetUserFTA, Directory Opus Win+E, Files, Default Folder X). **V** [5][6][7][8][9][10]
4. The draft plan says Cmd+Tab "is no symbolic hotkey" on macOS. alt-tab-macos' source says the opposite: symbolic hotkey ids 1 (Cmd+Tab) and 2 (Cmd+Shift+Tab) exist and the Dock matches them in the WindowServer layer before any app's hotkey. Only Cmd+` is not a symbolic hotkey in practice. **V** [11]
5. Disabling a macOS symbolic hotkey outlives the process. alt-tab-macos re-enables it in `applicationWillTerminate` and in a crash handler, which means a SIGKILL or power loss leaves Cmd+Tab dead. The facade needs a persistent ledger and a next-start replay, which `qol_host_fixes::takeover` already is. **V** [11]
6. Windows protects exactly the handlers that matter most (http, https, .pdf) with a kernel driver (UCPD.sys) and a hash. Prior art converges on "register the app properly, then send the user to Settings". **V** [6][12][13]

## 2. Handler defaults: how libraries model them

| Library or tool | Abstraction | Read | Write | Report of refusal | Restore |
|---|---|---|---|---|---|
| Electron `app.*DefaultProtocolClient` | Per scheme, current executable only | `isDefaultProtocolClient(scheme, path, args)` bool. Linux compares `g_app_info_get_default_for_uri_scheme` id with `CHROME_DESKTOP`. Windows compares the `HKCU\Software\Classes\<scheme>\shell\open\command` value. macOS uses `LSCopyDefaultHandlerForURLScheme` | `setAsDefaultProtocolClient` bool. Windows writes HKCU classes keys, not UserChoice. macOS uses `LSSetDefaultHandlerForURLScheme` and only works for schemes already in Info.plist. Linux calls `xdg-settings` | One bool. Windows Store apps get `true` always while the key is unreachable by others | `removeAsDefaultProtocolClient` (macOS, Windows). Windows deletes the key only if the command still equals this exe, otherwise returns `true` and leaves it. Linux returns `false` ("Todo implement") |
| Chromium `shell_integration` | Browser and scheme, per install mode | `DefaultWebClientState` 4 values. Linux: `xdg-settings get`. Windows: `IApplicationAssociationRegistration::QueryCurrentDefault` and `AssocQueryString`. macOS: `LSCopyDefault...` | Workers with `StartSetAsDefault`. Linux `xdg-settings set`. Windows opens Settings (`ShowMakeChromeDefaultSystemUI`). macOS `NSWorkspace setDefaultApplication...` | `UNKNOWN_DEFAULT` when `xdg-settings` fails. Windows watches `...\UrlAssociations\<scheme>\UserChoice` for change with a 2 minute timeout to know the user finished | None. Chrome does not restore other browsers |
| Firefox `nsIShellService` and `nsIDefaultAgent` | Browser, plus optional extra file types | Windows: UserChoice `ProgId`. | Tries `setDefaultBrowserUserChoice` (writes the hash itself, "must only be used with user consent"), falls back to the Settings UI and records `deeplinkedToWindowsSettingsUI` | Throws if the hash write fails, caller decides to open Settings | None |
| Qt `QDesktopServices` | In-process only | none | `setUrlHandler` routes a scheme to a QObject inside the app. No API sets the OS default | none | `unsetUrlHandler` is in-process only |
| GLib/GIO `GAppInfo` | MIME type or URI scheme to desktop entry | `g_app_info_get_default_for_type`, `..._for_uri_scheme` | `g_app_info_set_as_default_for_type`, writes user `mimeapps.list` | returns error or false | `g_app_info_reset_type_associations` clears the user's entries (does not restore a previous value) |
| xdg-utils | `xdg-settings get/set default-web-browser`, `default-url-scheme-handler <scheme>`; `xdg-mime query/default` | text on stdout | file edits | exit code only. Known quirk: on GNOME 3 `set default-url-scheme-handler` also sets `text/html` | none |
| Rust `opener` 0.9 | Open only: `open`, `open_browser`, `reveal` (feature) | none | none | `OpenError`. Linux `open` ignores xdg-open's exit status because it may not exit until the app does. `reveal` tries D-Bus `FileManager1`, then falls back to opening the parent folder with the default handler | n/a |
| Rust `handlr`, `handlr-regex` | Linux `mimeapps.list` manager | yes | yes | n/a | n/a |
| Rust `xdg-mime` (ebassi) | Reads the shared MIME database only. It does not read or set default apps | | | | |
| `duti` (C, moretension) | `duti -s <bundle-id> <uti-or-scheme> <role>` | `duti -x`, `duti -d` | LaunchServices; `dutix` notes `public.html` cannot be set and http-type defaults need a Settings confirmation | exit code | none |
| SetUserFTA / SetDefaultBrowser (Kolbicz) | Windows file type and protocol | UserChoice | Reimplements the UserChoice hash (SID, ProgId, last-write time) | "could not open registry key", error 5 for http, https, .pdf after UCPD | none |

Sources: Electron [2][3][4], Chromium [1][14][15][16], Firefox [17], Qt [18], GLib [19], xdg-utils [20][21], opener [22][23], crates.io search [24], duti [25], SetUserFTA [12][13].

Takeaways for the facade:

- The only honest return type is a state enum plus a read-back, not a bool. Electron's Windows Store case shows a write can succeed and still have no effect. **V** [2]
- "Is it mine?" must compare what qol wrote with what is there now. Electron's Windows remove already does this (delete only if the command still equals our exe). **V** [3]
- Chromium's `OTHER_MODE_IS_DEFAULT` maps to qol's dev versus installed binary: a claim made by another qol install must not be treated as the user's. **V** [1]
- Chromium's registry-watch-with-timeout is the one worked example of "the OS made the user do it, how do I know they finished". **V** [15]

## 3. Handler defaults per OS

### Linux

- Storage is `mimeapps.list`, lookup cascades user config, desktop-specific (`cinnamon-mimeapps.list`) before generic, then system. No protection, no admin. **V** [26]
- Unset versus set by someone: absent from every file in the cascade is "unset" (the desktop picks the first `MimeType=` match, which may differ by desktop). Set by someone is not distinguishable, files carry no author. **I** [26]
- Desktop entry needs `MimeType=` or `x-scheme-handler/<scheme>`; qol-tray installer already writes desktop entries (plan).
- Failure modes: the `xdg-settings` GNOME quirk above; `FileManager1` dispatches by D-Bus bus name, not `inode/directory`, so "reveal in folder" ignores the MIME default. **V** [27] Implementations vary: PCManFM-Qt treats `ShowFolders` URIs as generic and opens files (CVE-2026-48700), so an implementer of `FileManager1` should validate that each URI is a directory. **V** [28]
- A user-level D-Bus service file can claim `org.freedesktop.FileManager1`, reported to depend on filename order versus the system file. Single report, not verified. **I** [27]
- Wayland versus X11 makes no difference for handlers; it matters only for slots (section 5).

### Windows

- Handler of record is `HKCU\Software\Microsoft\Windows\Shell\Associations\{UrlAssociations|FileExts}\<x>\UserChoice` with `ProgId` and `Hash`. **V** [17]
- The app must register a ProgID, `Capabilities` (with `ApplicationDescription`, required or the app is not listed), and a `RegisteredApplications` value, then call `SHChangeNotify(SHCNE_ASSOCCHANGED)`. **V** [29]
- The Win32 `IApplicationAssociationRegistration` write and restore methods are deprecated since Windows 8; `QueryAppIsDefault` is "no longer supported" but `QueryCurrentDefault` is still used by Chromium to read. **V** [29][16]
- Microsoft's own guidance: "The choice of default should be user driven. An application should never reclaim a default without asking the user." **V** [29]
- Protection: UserChoice hash. UCPD.sys (Feb 2024, KB5034763 and KB5034765) denies non-Microsoft processes writes to http, https and .pdf UserChoice. Microsoft-signed processes on a deny list are still blocked. Firefox documents that the lock is path-based and now covers all ancestors of `UserChoice`. **V** [12][17] The draft plan adds `UserChoiceLatest` with a machine id in the hash; I could not open the source (403), so **I** [30].
- User prompt: none through the API. `ms-settings:defaultapps?registeredAppUser=<name>` (also `registeredAppMachine`, `registeredAUMID`) opens the app's own page, available from Windows 11 21H2/22H2 with the April 2023 cumulative update. `ms-settings:defaultbrowsersettings` is deprecated in Windows 11. **V** [31]
- Custom schemes are a different, unprotected path: `HKCU\Software\Classes\<scheme>\shell\open\command` is what Electron writes. It works because no UserChoice exists for a scheme nobody else claims. **V** [3] If two apps claim the same custom scheme, Windows will show its chooser. **I**
- Uninstall: Microsoft says an app that took a file type should not delete the Default value; Windows ignores a Default whose ProgID is not registered. So removing the ProgID is a safe, self-healing restore for the Resident uninstall case. **V** [32]
- Folder open verbs: third-party managers write `Directory\shell\open\command` and, for Win+E, a CLSID under HKCU classes with `shell\opennewwindow\command` and an empty `DelegateExecute`. No protection, but results are "inconsistent" and windows updates revert it. **V** [33][34] Directory Opus documents that programs invoking Explorer directly are not intercepted, Control Panel pages always open in Explorer, Win+E is handled by a separate process on Windows 10 and later, and a Windows update can revert the registry entries (fix: toggle the setting off and on). **V** [9]
- Files 2.0.33 added "Set Files as the default file manager" behind an experimental flag. I could not find its exact keys or revert path. **I** [8]

### macOS

- API: `NSWorkspace.setDefaultApplication(at:toOpenURLsWithScheme:)`, `...toOpenContentType:`, `...toOpenFileAt:` from macOS 12; older `LSSetDefaultHandlerForURLScheme` is the fallback. **V** [16][35]
- Prompt: for http and https a system dialog from `CoreServicesUIAgent` asks the user to confirm. Chromium has to activate that agent because the dialog is sometimes ordered behind the app window. **V** [16] Declining the first dialog makes a second call prompt again (crbug 496408111), so chain the calls and stop on error. **V** [16] On macOS 26.4 and later, setting the HTML content type adds a second Finder confirmation, so Chromium skips it. **V** [16]
- Chromium still marks the custom scheme case `SET_DEFAULT_UNATTENDED` for the Mac but its comment says that is a compatibility lie ("actually requires interaction (a TODO)"). **V** [1]
- Target must be a real `.app` with `CFBundleURLTypes` or `CFBundleDocumentTypes` in Info.plist; Electron states the plist "can't be changed at runtime". **V** [2]
- Sandboxed apps could not use the older call; the claim comes from a 2018 blog, now outdated. **I** [36]
- `duti`/`dutix`: system-protected defaults such as http need a Settings confirmation, `public.html` cannot be set programmatically. **V** [25]
- Folder open and reveal: `NSFileViewer` global default (`defaults write -g NSFileViewer -string <bundle id>`, `defaults delete -g NSFileViewer` to undo) is the only documented mechanism for Cocoa apps that reveal in Finder; it makes other apps use that viewer too, plus an unreliable `LSHandlers` `public.folder` edit that works on some reboots and not others. Evidence is from 2018 and 2020 (Catalina). **V** for the commands, **I** for current behaviour on macOS 15 and 26 [37][38]. Default Folder X had to ship 6.2.8 in May 2026 because Path Finder 26 changed behaviour. **V** [39]
- Finder itself owns desktop double-click, Dock folder stacks, and the Open dialog; no setting redirects those. **I**

## 4. Slot defaults: replacement tools

### alt-tab-macos (window switcher)

- Mechanism: carbon hotkey registration for its own shortcut, a passive `CGEventTap` on the session tap (listen-only) for modifier flags, plus a private SkyLight call `CGSSetSymbolicHotKeyEnabled(id, bool)` to switch off ids 1 and 2 so the Dock stops answering Cmd+Tab. **V** [11]
- Detection of refusal: `CGEvent.tapCreate` returns null when Accessibility is not granted; it logs `tapDisabledByTimeout` and `tapDisabledByUserInput` and re-enables the tap; the Esc tap is created disabled and only enabled while a session is open, to keep an active HID tap out of the way. **V** [11]
- Restore: re-enable all ids at `applicationWillTerminate` and in an emergency handler for uncaught exceptions and signals, using `_exit` after. The source comment says "the effect of enabling/disabling persists after the app is quit". No previous value is read: it assumes ids 1 and 2 were enabled before. If the user had disabled native Cmd+Tab themselves, quitting re-enables it. **V** [11]
- Known failure modes: GitHub issues where native Cmd+Tab still fires on macOS 13; a dictionary-order bug (#5653) where both ids were not disabled; macOS 26 Game Overlay eats Cmd+Esc before app taps (#5585). **V** [11]
- Use of private API: not App Store compatible, may break across releases. **I**

### Raycast and Alfred (Cmd+Space)

- Raycast does not take Cmd+Space. Its manual tells the user to untick "Show Spotlight Search" in System Settings, Keyboard, Keyboard Shortcuts, Spotlight, then record Cmd+Space in Raycast. Siri's hold-Cmd-Space and input source shortcuts are other conflicts. **V** [40]
- Hence the abstraction is a user instruction, with the app's own hotkey field as the only state. Alfred: no primary source read. **I**
- The programmatic route that exists: symbolic hotkey id 64 is "Spotlight search field", 65 "Spotlight window"; screenshot ids 28 to 31. **V** [41] Writing `com.apple.symbolichotkeys` with `defaults write ... -dict-add` or the private CGS call both work; `activateSettings -u` makes `defaults` apply. The `defaults` route is **I** from [42], the CGS route **V** from [11].

### Flameshot and ShareX (PrtSc)

- Flameshot does not bind Print Screen itself on Linux. The docs tell the user to unbind the built-in capture in GNOME (Settings, Keyboard, Shortcuts, Backspace) or disable Spectacle's shortcuts in KDE first, then add a custom shortcut running `flameshot gui`. **V** [43]
- ShareX registers PrtSc itself and shows per-hotkey status (red or green) in its hotkey settings. On Windows 11 the key belongs to Snipping Tool until the user turns off "Use the Print screen button to open screen capture" (Settings, Accessibility, Keyboard). OneDrive screenshot backup can also take the key. The fix does not persist across OneDrive or Snipping Tool updates. **V** [44] I did not confirm the registry value behind that toggle, so the plan's `PrintScreenKeyForSnippingEnabled` name is **I**.

### Ditto (Win+V)

- No primary source read. A search found guides that rebind with PowerToys Keyboard Manager or AutoHotkey (Win+V remapped to Ditto's own hotkey). The built-in clipboard history stays enabled, only the key goes elsewhere. **I** [45]

### PowerToys

| Piece | Mechanism | Can take | Cannot take | Restore |
|---|---|---|---|---|
| Keyboard Manager | Low-level keyboard hook, process-name targeting | Most keys and shortcuts, text, app launch | Win+L and Ctrl+Alt+Del (OS-reserved), Fn (mostly), Win+G still opens Game Bar when reassigned, Pause only sends one key-down, windows running as admin unless PowerToys is elevated | Hook is removed when PowerToys exits or Keyboard Manager is disabled. Nothing persists. **V** [46] |
| Run | Activation shortcut Alt+Space by default, optional "centralized keyboard hook" | Alt+Space | Does not touch the Win key or Start | Stops with PowerToys. Documented problem: not focused when triggered from an elevated window. **V** [47] |
| FancyZones | "Override Windows Snap hotkeys" option; hooks Win+Arrow | Win+Left/Right (zone index mode) or all four arrows (relative mode) | Win+Shift+Arrow stays with Windows. Cannot zone Hyper-V, Citrix, Parsec, RemoteApp, VMware Player and others (they never raise `EVENT_SYSTEM_MOVESIZESTART`) | Setting off, or exit. **V** [48] |

This is the cleanest Portable-mode model on Windows: a hook gives the takeover and process exit gives the restore, with no registry or driver involved. The cost is that nothing works while elevated windows are focused and security software can flag hooks. **V** for the process lifetime claim [46], **I** for the AV remark.

Win-key combos are reserved for the OS: `RegisterHotKey` documents that shortcuts using the Windows key are reserved, and fails if another app registered the same keys. **V** [49] The one documented persistent route is the `DisabledHotkeys` string value under `HKCU\...\Explorer\Advanced` (one letter per Win+letter, applied after Explorer restarts or sign-out) and the `NoWinKeys` policy that disables all. Shifted combos such as Win+Shift+S are not covered by `DisabledHotkeys` in any source I found. **V** for the value, **I** for the Shift gap [50][51].

### Replacement file managers

| Tool | How it takes over | Scope | Failure modes |
|---|---|---|---|
| Directory Opus | "Explorer Replacement" preference writes registry entries; separate handling of Win+E | Opens a Lister wherever Explorer would open a window. Does not replace shell, Taskbar, Start, File Open/Save. Not available for portable copies | Direct `explorer.exe` calls and Control Panel pages bypass it; Windows updates revert Win+E; toggle off and on to rewrite. **V** [9] |
| Files | Settings, Advanced, experimental "Set Files as default file manager" | Windows 10 and 11 | Mechanism undocumented in my sources; **I** [8] |
| XYplorer and Total Commander users | Manual `DelegateExecute` rename or HKCU CLSID | Win+E, folder verbs | Registry cleaners reset it every reboot; desktop click behaviour can break. **V** [33][34] |
| Path Finder, ForkLift | Preference that sets `NSFileViewer` and the `public.folder` handler | Cocoa apps that reveal in Finder | Unreliable across reboots, global side effects. **V** [37][38] |
| Dolphin or Nemo as default on Linux | `inode/directory` plus the `FileManager1` bus name | Apps that use xdg-open honour the MIME default; apps that call `FileManager1` use the bus owner | Dispatch mismatch described above. **V** [27] |

## 5. Linux slots

- The Wayland-safe route is the `org.freedesktop.portal.GlobalShortcuts` portal. `CreateSession`, then `BindShortcuts` which "typically shows a dialog" and may only be attempted once per session; `preferred_trigger` is only a preference; `ListShortcuts` returns what was previously bound; `ConfigureShortcuts` since version 2; `ShortcutsChanged` signal. **V** [52] Which compositors implement it is not stated in the docs. **I**
- It cannot take a key already bound by the desktop. A portal user has to remove the desktop's binding first, as Flameshot's docs instruct. **V** [43]
- X11 route and dconf route (existing qol code in `apps/tray/src/hotkeys/takeover`): claims ledger with `previous`, `applied`, restore decisions `Rewrite`, `Abandon`, `Quarantine`, `Settle`, which is the same compare-and-restore pattern as Electron's remove and is further along than any prior art above. **V** (repo read)

## 6. Patterns across the prior art

**Detecting refusal.** Four patterns appear:
1. Return code or null (`CGEvent.tapCreate` null; Electron bool; SetUserFTA error 5). Cheap, but says nothing about why. [3][11][12]
2. Read-back after write (Chromium `GetIsDefaultWebClient`; Electron `isDefault...`). The only reliable test, since Windows Store Electron returns true regardless. [1][2]
3. Passive watch for the user finishing (Chromium registry watcher with timeout; ShareX per-hotkey red/green). [15][44]
4. Tool-reported unknown (`UNKNOWN_DEFAULT` when `xdg-settings` fails). [1]

**Restore.** Three patterns:
1. Constant baseline: re-enable everything on exit (alt-tab-macos). Simple, wrong if the user had disabled it. [11]
2. Compare then undo: delete only if the value is still ours (Electron Windows). [3]
3. Dangling-reference tolerance: leave the Default, remove the ProgID, Windows ignores it (Microsoft guidance). [32]
The ledger approach in the repo (previous value, applied value, decision on next start) is the only one that also survives crashes.

**Known failure modes seen more than once.**
- OS updates silently revert third-party defaults (Opus Win+E, ShareX/OneDrive/Snipping Tool, UCPD, `NSFileViewer`). **V**
- Bypass by direct callers (Explorer invoked by path, `FileManager1` by bus name, `NSFileViewer` for Cocoa only, Control Panel). **V**
- Persistent OS-side state that outlives the process (macOS symbolic hotkeys). **V**
- Confirmation dialogs that hide behind windows (macOS CoreServicesUIAgent). **V**
- Double prompting (macOS https after http decline; Finder after URL scheme on 26.4). **V**
- Elevation boundary: hooks do not see elevated windows (PowerToys). **V**
- Unexpected side effects of one write (`xdg-settings` also sets text/html on GNOME 3; `NSFileViewer` global). **V**

## 7. Proposed taxonomy

### Capability states

One value per (role, OS, desktop or build), computed by a live probe and not a static table, since Windows 11 builds, GNOME versus Cinnamon, and the granted Accessibility permission all change the answer.

| State | Meaning | UI behaviour |
|---|---|---|
| `Silent` | qol can write the value and read it back with no OS UI and no new grant | Toggle works at once |
| `NeedsConfirmation` | The OS shows its own UI or qol must open it (macOS http confirm, Windows Settings page, Wayland portal dialog); completion detected by read-back or a watch with timeout | Toggle shows the click that will be needed; claim stays `Pending` until read-back matches |
| `NeedsPermission` | A one-time grant the user must give in OS settings (macOS Accessibility or Input Monitoring, Windows admin for machine-wide policy). Once granted it is `Silent` | Toggle opens the permission step; re-probe after |
| `Impossible` | OS or protection forbids it (Win+L, Ctrl+Alt+Del, macOS Finder desktop folder open, FileManager1 on a desktop that owns the bus name, Windows reveal via explorer.exe) | Not offered; the reason is shown |

Two orthogonal fields, so the four states stay clean:
- `Reach`: `Full` or `Partial(list of bypasses)`, e.g. Windows folder open: Partial(explorer.exe direct, Control Panel).
- `Restore`: `Exact` (previous value known and put back), `Baseline` (back to OS default, previous value unknowable), `None`.
A probe failure is a fifth value, `Unknown`, treated like `Impossible` for offering but retried; it mirrors Chromium's `UNKNOWN_DEFAULT`.

### Roles

| Role | Kind | Backing OS construct |
|---|---|---|
| `folder.open` | Handler | `inode/directory` / Directory+Drive verbs / folder UTI (`public.folder`) |
| `folder.reveal` | Handler | `FileManager1.ShowItems` / `explorer.exe /select` and `SHOpenFolderAndSelectItems` / `NSFileViewer` |
| `file.type(<mime, ext, uti>)` | Handler | `mimeapps.list` / UserChoice via Settings / LaunchServices |
| `url.scheme.custom(<scheme>)` | Handler | `x-scheme-handler` / HKCU classes / `CFBundleURLTypes` |
| `url.web` (http, https, html) | Handler | `xdg-settings default-web-browser` / UserChoice (UCPD) / macOS confirm |
| `mail` | Handler | `x-scheme-handler/mailto` / `mailto` UserChoice and Clients\Mail / LaunchServices mailto |
| `terminal` | Handler | Debian `x-terminal-emulator`, desktop setting / none in Windows (default terminal setting in Terminal) / none |
| `slot.launcher` | Slot | desktop binding / lone Win key / symbolic hotkey 64 |
| `slot.window_switcher` | Slot | desktop binding / Alt+Tab in hook / symbolic hotkeys 1 and 2 |
| `slot.screenshot` | Slot | desktop media keys / PrtSc toggle, Win+Shift+S / symbolic hotkeys 28 to 31, 184 |
| `slot.clipboard_history` | Slot | none / Win+V / none |
| `slot.dictation` | Slot | none / Win+H / dictation shortcut |
| `slot.window_snap` | Slot | desktop tiling keys / Win+Arrow / tiling keys on macOS 15 and later |
| `slot.lock` | Route only | host lock command; the key itself is not claimable on Windows |
| `slot.tray_applet` | Additive only | an extra applet or menu-bar item; the OS one cannot be removed |

IDs 28 to 31 and 64 and 65 are **V** [41]; 184 is **I**.

### Per-role states (best-case, to be confirmed by probe)

| Role | Linux | Windows | macOS |
|---|---|---|---|
| `folder.open` | Silent, Partial (desktop file manager opens folders itself) | Silent for verbs, Partial (Explorer calls, Control Panel) | Impossible (Finder owns it) |
| `folder.reveal` | Impossible where the desktop owns `FileManager1`, else Silent | Impossible (direct Explorer calls) | Silent through `NSFileViewer`, but Partial (Cocoa apps only) and unreliable until the calling app relaunches |
| `file.type` | Silent | NeedsConfirmation (Settings page); Silent only for qol-owned extensions | Silent through API for most types; NeedsConfirmation on 26.4 for some |
| `url.scheme.custom` | Silent | Silent (HKCU classes) | Silent if the scheme is in Info.plist, else Impossible |
| `url.web` | Silent (GNOME 3 side effect on text/html) | NeedsConfirmation (UCPD forbids silent) | NeedsConfirmation |
| `mail` | Silent | NeedsConfirmation | NeedsConfirmation (inferred) |
| `slot.launcher` | Silent (existing takeover) | Silent via hook; Partial (Start button, search box click) | NeedsPermission for event tap, or Silent via symbolic hotkey 64 |
| `slot.window_switcher` | Silent | Silent via hook | Silent via symbolic hotkey ids 1 and 2 (private API) plus NeedsPermission for the tap |
| `slot.screenshot` | Silent | Silent via hook and the PrtSc toggle; Win+Shift+S via hook only | Silent via symbolic hotkeys |
| `slot.clipboard_history` | Silent (no host tool) | Silent via hook, Win+V via `DisabledHotkeys` needs Explorer restart | Silent (no host tool) |
| `slot.dictation` | Silent (no host tool) | Silent via hook | Silent via preference write |
| `slot.window_snap` | Silent | Silent via hook (FancyZones precedent) | Silent |
| `slot.lock` | Route only | Impossible to claim | Route only |
| `slot.tray_applet` | Additive | Additive | Additive |

Linux Wayland slots are NeedsConfirmation through the GlobalShortcuts portal and Impossible where the compositor lacks it. **I** (portal coverage not read).

### Facade shape suggested by the prior art

- `probe(role) -> { state, reach, restore, reason }`, recomputed on demand, cached with a short life.
- `read(role) -> { value, origin: Unset | Ours(claim_id) | Foreign }`. `Foreign` means we cannot say whether a person or another program set it.
- `claim(role, plugin, lifetime) -> Applied | Pending(watch) | Refused(reason)`, always followed by read-back before reporting `Applied`.
- `release(claim)`: compare then undo; if the current value differs from the applied one, drop the claim and report `UserChanged`. This is Electron's remove rule plus the repo ledger's `Abandon`.
- Pending claims that need the OS UI carry a deadline (Chromium uses 2 minutes) and a watch on the exact registry key, file, or plist that changes.
- A claim whose target binary or plugin vanished is dangling: on Windows remove the ProgID, on Linux remove the entry, on macOS set back the recorded previous handler.

## 8. Corrections and risks for the draft plan

1. Cmd+Tab is a symbolic hotkey (ids 1 and 2). Use `CGSSetSymbolicHotKeyEnabled`, not only an event tap. Cmd+` needs nothing. [11]
2. The persistence of that disable across quit is the strongest argument for the claim ledger with next-start replay; do not copy alt-tab-macos' constant baseline restore. [11]
3. `PrintScreenKeyForSnippingEnabled` is not confirmed by any source I found; the confirmed user-facing toggle is the Settings, Accessibility, Keyboard option. Verify the value name in a Windows guest before building. [44]
4. `DisabledHotkeys` is documented for Win+letter only; Win+Shift+S probably needs the hook. [50]
5. For `url.web` on Windows, qol must go through the Settings page; there is no silent path and UCPD makes forging a dead end. [12][17]
6. For `folder.reveal` the plan's "impossible on Cinnamon, impossible on Windows" matches what Directory Opus and the Arch thread report. [9][27]
7. macOS 26.4 added double prompts for content types; any macOS handler claim code needs a version gate. [16]
8. Hooks and event taps lose to elevated windows (Windows) and need Accessibility (macOS). Portable mode should report this as `NeedsPermission`, not silently under-deliver. [46][11]

## Sources

1. Chromium shell_integration.h (read): https://raw.githubusercontent.com/chromium/chromium/main/chrome/browser/shell_integration.h
2. Electron app API docs: https://www.electronjs.org/docs/latest/api/app
3. Electron Windows implementation (read): https://raw.githubusercontent.com/electron/electron/main/shell/browser/browser_win.cc
4. Electron Linux implementation (read): https://raw.githubusercontent.com/electron/electron/main/shell/browser/browser_linux.cc
5. Raycast hotkey manual (via search result): https://manual.raycast.com/hotkey
6. Winhelponline on UCPD (via search): https://www.winhelponline.com/blog/set-default-browser-file-associations-command-line-windows-10/
7. Flameshot key bindings: https://flameshot.org/docs/guide/key-bindings/
8. Windows Central on Files default file manager: https://windowscentral.com/you-can-now-set-beautiful-file-explorer-your-default-windows-11
9. Directory Opus Explorer replacement manual and forum: https://github.com/Chaoses-Ib/DirectoryOpus/blob/main/Manual/basic_concepts/explorer_replacement.md and https://resource.dopus.com/t/win-e-not-working-after-few-days-after-installation/24459
10. Default Folder X NSFileViewer note: https://stclairsoft.com/blog/?p=1182
11. alt-tab-macos source (read): https://raw.githubusercontent.com/lwouis/alt-tab-macos/master/src/macos/api-wrappers/SkyLight.framework.swift , .../src/macos/api-wrappers/CGSSymbolicHotKey.swift , .../src/preferences/settings-window/tabs/controls/NativeHotkeyResolver.swift , .../src/events/KeyboardEvents.swift , .../src/main.swift , .../src/App.swift
12. Ghacks on UCPD: https://www.ghacks.net/?p=205343 and Gigazine https://gigazine.net/gsc_news/en/20240408-windows-default-browser-userchoice-protection-driver
13. SetUserFTA background (TechTarget): https://www.techtarget.com/enterprise-software/tutorial/Manage-Windows-10-default-file-type-associations-with-SetUserFTA
14. Chromium shell_integration_linux.cc (read): https://raw.githubusercontent.com/chromium/chromium/main/chrome/browser/shell_integration_linux.cc
15. Chromium shell_integration_win.cc (read, OpenSystemSettingsHelper): https://raw.githubusercontent.com/chromium/chromium/main/chrome/browser/shell_integration_win.cc
16. Chromium shell_integration_mac.mm (read): https://raw.githubusercontent.com/chromium/chromium/main/chrome/browser/shell_integration_mac.mm
17. Firefox Set Default docs: https://firefox-source-docs.mozilla.org/widget/windows/shell/set-default.html
18. Qt QDesktopServices: https://doc.qt.io/qt-6/qdesktopservices.html
19. GLib mimeapps tests (set_as_default, reset_type_associations): https://fuchsia.googlesource.com/third_party/glib/+/refs/tags/2.33.2/gio/tests/mimeapps.c
20. xdg-settings man page: https://man.archlinux.org/man/xdg-settings.1.en
21. xdg-utils issue 206 (GNOME 3 text/html): https://gitlab.freedesktop.org/xdg/xdg-utils/-/issues/206
22. opener docs: https://docs.rs/opener/latest/opener/
23. opener source (read): https://raw.githubusercontent.com/Seeker14491/opener/master/opener/src/linux_and_more.rs
24. crates.io API searches (handlr 0.6.4, handlr-regex, webbrowser, opener; no maintained cross-platform setter found): https://crates.io/crates/handlr
25. duti and dutix: https://formulae.brew.sh/formula/duti and https://awesome.ecosyste.ms/projects/github.com%2Fjackchuka%2Fdutix
26. freedesktop mime-apps spec: https://specifications.freedesktop.org/mime-apps/latest-single
27. Arch forum, FileManager1 dispatch: https://bbs.archlinux.org/viewtopic.php?id=285355 ; freedesktop spec https://wiki.freedesktop.org/www/Specifications/file-manager-interface
28. oss-sec PCManFM-Qt report: https://seclists.org/oss-sec/2026/q2/616
29. Microsoft, Default Programs: https://learn.microsoft.com/en-us/windows/win32/shell/default-programs
30. UserChoiceLatest (kolbi.cz, not fetched, 403): https://kolbi.cz/blog/2025/04/20/userchoicelatest-microsofts-new-protection-for-file-type-associations/
31. Microsoft, ms-settings URIs: https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-settings-app
32. Microsoft, File Types (uninstall guidance): https://learn.microsoft.com/en-us/windows/win32/shell/fa-file-types
33. FileVoyager forum Win+E CLSID: https://www.filevoyager.com/forums/reply/4536
34. XYplorer forum DelegateExecute: https://www.xyplorer.com/xyfc/viewtopic.php?p=164893
35. Apple, NSWorkspace setDefaultApplication: https://developer.apple.com/documentation/appkit/nsworkspace/setdefaultapplication(at:toopenurlswithscheme:completion:).md
36. Michael Tsai on LSSetDefaultHandlerForURLScheme: https://mjtsai.com/blog/?p=29975
37. MacPowerUsers ForkLift thread: https://talk.macpowerusers.com/t/forklift-3-default-file-viewer-with-mac-os-catalina-10-15-7/33989
38. Default Folder X NSFileViewer: https://stclairsoft.com/blog/?p=1182
39. Default Folder X May 2026 release note: https://www.stclairsoft.com/blog/2026/05/
40. Raycast manual: https://manual.raycast.com/hotkey
41. CGSHotKeys.h: https://raw.githubusercontent.com/NUIKit/CGSInternal/master/CGSHotKeys.h
42. symbolichotkeys defaults pattern: https://www.ccn.ucla.edu/wiki/index.php/OSX_Defaults_Hacks_and_General_Customization
43. Flameshot: https://flameshot.org/docs/guide/key-bindings/
44. ShareX issue and Print Screen toggle: https://github.com/ShareX/ShareX/issues/6581 and https://pureinfotech.com/enable-print-key-take-screenshot-snipping-tool-windows-11
45. Octoclip Win+V remap guide: https://docs.octoclip.app/advanced/windows/win-v-shortcut/
46. PowerToys Keyboard Manager: https://learn.microsoft.com/en-us/windows/powertoys/keyboard-manager
47. PowerToys Run: https://learn.microsoft.com/en-us/windows/powertoys/run
48. PowerToys FancyZones: https://learn.microsoft.com/en-us/windows/powertoys/fancyzones
49. RegisterHotKey: https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-registerhotkey
50. DisabledHotkeys, Geoff Chappell: https://geoffchappell.com/notes/windows/shell/explorer/globalhotkeys.htm
51. Winaero DisabledHotkeys: https://winaero.com/disable-certain-winkey-shortcuts-in-windows-10
52. xdg-desktop-portal GlobalShortcuts: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html
