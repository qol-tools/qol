# qol as host defaults

Goal: every job the host OS hands to a default app or a built-in tool, and that a qol plugin can do, can be handed to that plugin instead, on Linux, Windows and macOS.
Opening folders in qol explorer is one case; pressing Super for the qol launcher or PrtSc for qol shot is the same idea.
This is research and a proposed shape; nothing is built yet.

## Two kinds of default

1. Handler defaults: which app opens a thing. Folders, "show in folder", file types, URL schemes.
   The host keeps a registry of handlers and qol registers its app and points the entry at it.
2. Slot defaults: which tool answers a system trigger. The app launcher on Super or Start, the window switcher on Alt+Tab, the screenshot tool on PrtSc, clipboard history on Win+V, dictation on Win+H.
   The host binds the trigger to its own tool, so qol takes the trigger away from that tool and answers it.
   Linux already does this for dconf desktops in `apps/tray/src/hotkeys/takeover`; Windows and macOS takeover are stubs today.

## Roles qol can own

| Role | qol plugin | Linux (Cinnamon) | Windows | macOS |
|---|---|---|---|---|
| Open a folder | explorer | `inode/directory` in `mimeapps.list` | `Directory` and `Drive` shell verbs under `HKCU\Software\Classes` | not possible: Finder owns folder double-clicks, the Dock and the desktop |
| Show a file in its folder | explorer | not possible on Cinnamon: `nemo-desktop` permanently owns `org.freedesktop.FileManager1` | not possible: apps call `explorer.exe /select` or `SHOpenFolderAndSelectItems` directly | global default `NSFileViewer` (Cocoa apps only) |
| App launcher | launcher | take over the menu applet's Super binding (spices takeover exists) | swallow the lone Win key in the low-level keyboard hook | free Cmd+Space by disabling the Spotlight entry in `com.apple.symbolichotkeys` |
| Window switcher | alt-tab | take over `switch-windows` / `switch-applications` (exists) | swallow Alt+Tab in the low-level hook | Cmd+Tab is no symbolic hotkey; swallow it in an event tap (Accessibility permission) |
| Screenshot | shot | take over the screenshot keys in `org.cinnamon.desktop.keybindings.media-keys` | `PrintScreenKeyForSnippingEnabled=0` frees PrtSc; Win+Shift+S through `DisabledHotkeys` or the hook | disable symbolic hotkeys 28 to 31 and 184 (Cmd+Shift+3, 4, 5) |
| Clipboard history | clipboard | no host tool; plain hotkey | Win+V through `DisabledHotkeys` or the hook | no host tool before macOS 26; Spotlight's clipboard view on 26 has no own key |
| Dictation | voice | no host tool | Win+H through `DisabledHotkeys` or the hook | disable the dictation shortcut preference |
| Window snapping | window-actions | take over the tiling keys in `org.cinnamon.desktop.keybindings.wm` | Win+arrows through the hook | no host tool before macOS 15; disable its tiling keys on 15 and later |
| Lock screen | session | already the host's lock; route only | Win+L cannot be taken; qol's key calls the host lock | same |
| File types, URL schemes | none today | `mimeapps.list` | needs one click in Settings (below) | `NSWorkspace.setDefaultApplication` |

Plugins with no row (lights, bluetooth, sound, input, monitor, os-themes, memory, pointz, controllers, cli-sessions, keyremap, update-manager, removeapp) either change a host setting at the user's direction or do a job the host has no default for.
No qol app opens a file type today, so the file-type and URL-scheme rows are the mechanism a future viewer or editor plugin would use, plus `qol://` deep links into the tray.

## Platform limits

### Linux

`mimeapps.list` and dconf are plain per-user files with no protection and no admin.
Desktop-specific lists such as `cinnamon-mimeapps.list` win over the generic one, so the backend reads and claims whichever file decides the value.
A handler needs a desktop entry with the right `MimeType=`; the tray installer already writes desktop entries (`apps/tray/src/installer/platform/linux/desktop_entry.rs`).
Residual: Nemo and the Cinnamon desktop open folders in Nemo whatever the default says, and "show in folder" from browsers and editors goes to `nemo-desktop`.

### Windows

The folder verbs and `DisabledHotkeys` are per-user registry values with no protection; `DisabledHotkeys` takes effect after Explorer restarts, so the low-level keyboard hook (`libs/platform/src/native/platform/windows/hooks.rs`, already used by keyremap) is the instant path and `DisabledHotkeys` the fallback when the hook is not running.
File-type and URL-scheme defaults live in `UserChoice`, which carries a hash Windows checks; Windows 11 Home and Pro are moving to `UserChoiceLatest`, whose hash adds a machine id, and `UCPD.sys` blocks non-Microsoft writers for `http`, `https`, `.pdf` and others.
Forging the hash (SetUserFTA) breaks on each Windows rotation, so qol registers the app (ProgID, `Capabilities`, `RegisteredApplications` under `HKCU`) and opens `ms-settings:defaultapps?registeredAppUser=<name>` for the user's one click.
That bends non-negotiable 1, so the settings row says Windows requires the click.
Admin-only alternative for Resident hosts: the `DefaultAssociationsConfiguration` policy XML, applied at sign-in.

### macOS

Disabling a symbolic hotkey is a `defaults` write plus `activateSettings -u`; the doctor's hotkey-shadow check already reads `com.apple.symbolichotkeys`.
Cmd+Tab and anything not in that list need an event tap, which needs the Accessibility permission the user grants once; keyremap's tap (`plugins/keyremap/src/platform/macos/tap.rs`) is the existing owner.
Handler defaults need a real `.app` bundle declaring `CFBundleDocumentTypes` and `CFBundleURLTypes`; plugin binaries ship bare today.
Changing the `http` handler shows a system confirmation; document types do not.
`NSFileViewer` changes apply unreliably until the calling app relaunches.

## Lifetime

Pointing a host default or slot at qol is state qol introduces, not a host setting the user directed: when qol leaves a Portable host, a default that names a missing binary breaks folder opening and a disabled PrtSc stays dead.
Every claim therefore carries a lifetime:

- `PortableSession`: claim through `qol_host_fixes::takeover`, the ledger hotkey takeover already uses; restore on exit, and replay outstanding claims at the next start after a crash.
- `ResidentPolicy`: a new variant beside `nvidia` in `libs/host-fixes/src/policy`, journalled and restored when residency is disabled or qol is uninstalled.

Restore writes the previous value back only while the current value is still the one qol applied; if the user changed it since, the claim is dropped, never reverted.
Setting a default to a non-qol app through qol (VLC for `.mp4`) is the user's change and carries no lifetime.

## Proposed shape

- One `host_defaults` capability in qol-tray with a role enum (the rows above) and one backend per OS under `platform/`, each built from the existing owners: the mimeapps and registry handler writers, the dconf/spices takeover, the Windows hook, the macOS symbolic hotkeys and tap.
  `current(role)`, `claim(role, plugin, lifetime)`, `release(claim)`.
  Each backend reports per role whether it can claim silently, needs the user's click or a permission, or cannot, so the UI never offers what the OS will refuse.
- Plugins declare roles in `plugin.toml`, for example `[[default_for]] role = "window_switcher"`; the tray applies and restores, a plugin never writes host state.
- One settings page, "Defaults", lists every role a loaded plugin declares, with a toggle per role and the residual the OS leaves (Nemo still opens desktop folders, Windows needs a click).
- `qol-tray doctor` lists active claims and flags one whose recorded handler or plugin is gone.

## Order

1. Generalize the Linux takeover into roles: window switcher, launcher, screenshot and snapping already have dconf paths, so they move first, plus the explorer folder handler.
2. Windows slots through the low-level hook, then folder verbs once explorer runs on Windows.
3. macOS symbolic hotkeys, then the event tap for Cmd+Tab.
4. Handler defaults for file types and URL schemes, when a plugin can open one.

## Sources

- UCPD: https://www.bleepingcomputer.com/news/microsoft/new-windows-driver-blocks-software-from-changing-default-web-browser/
- UserChoiceLatest: https://kolbi.cz/blog/2025/04/20/userchoicelatest-microsofts-new-protection-for-file-type-associations/
- Windows folder verbs used by third-party managers: https://www.xyplorer.com/xyfc/viewtopic.php?p=41441
- `NSFileViewer`: https://stclairsoft.com/blog/?p=1182 and https://talk.macpowerusers.com/t/forklift-3-default-file-viewer-with-mac-os-catalina-10-15-7/33989
- `FileManager1` routing by bus name, not MIME default: https://bbs.archlinux.org/viewtopic.php?id=285355
