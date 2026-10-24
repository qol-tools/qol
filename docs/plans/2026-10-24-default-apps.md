# qol default apps

Goal: qol can make one of its own apps the host's default for a role, for example qol explorer as the file manager, on Linux, Windows and macOS.
This is research and a proposed shape; nothing is built yet.

## What it is under the mission

Pointing a host default at a qol binary is state qol introduces to make itself work, not a host setting the user directed.
When qol leaves a Portable host, a default that still names `qol-explorer` dangles and breaks folder opening.
So every default qol sets for one of its own apps carries a lifetime:

- `PortableSession`: claim through `qol_host_fixes::takeover` (the ledger hotkey takeover already uses), restore the recorded previous handler on exit, and replay outstanding claims at the next start after a crash.
- `ResidentPolicy`: a new `ResidentPolicy` variant beside `nvidia` in `libs/host-fixes/src/policy`, journalled and restored when residency is disabled or qol is uninstalled.

Restore writes back the previous value only when the current value is still the one qol applied; if the user changed it since, the claim is dropped, never reverted.
Setting a default to a non-qol app (VLC for `.mp4`) through qol is the user's change and carries no lifetime.

## Roles

1. File manager: open a folder, and reveal a file in its folder.
2. Content type: a MIME type, UTI or file extension (images, text, archives).
3. URL scheme: `http`, `https`, `mailto` and custom schemes.

## Per OS

### Linux (full control)

- Content types and the file manager's open-a-folder path: `[Default Applications]` in `~/.config/mimeapps.list`, key `inode/directory` for folders.
  No protection, no admin.
  Desktop-specific files such as `cinnamon-mimeapps.list` win over the generic one when present, so the backend reads and claims whichever file currently decides the value.
- The target needs a desktop entry with `MimeType=inode/directory;` and `Exec=qol-explorer open %U`.
  The tray installer already writes desktop entries (`apps/tray/src/installer/platform/linux/desktop_entry.rs`), so the plugin declares one and the installer writes it.
- Reveal a file is not routed by `mimeapps.list`.
  Browsers and editors call `org.freedesktop.FileManager1.ShowItems` on the session bus, which goes to whichever process owns that name.
  On this Cinnamon host `nemo-desktop` (pid 1910) owns it permanently, because it draws the desktop, so qol cannot take the name without replacing the desktop process.
  Residual: reveal stays in Nemo on Cinnamon; on desktops where the file manager only claims the name while a window is open, a qol explorer daemon can own it.
- Folder double-clicks inside Nemo and on the Cinnamon desktop stay in Nemo.

### Windows (folders yes, file types need one click)

- File manager: per-user default verb on `HKCU\Software\Classes\Directory\shell` and `Drive\shell` pointing at `qol-explorer open "%1"`.
  This is what XYplorer and OneCommander do; it needs no admin and is not hash-protected.
  Use `Directory` and `Drive`, never `Folder`, because `Folder` also covers Control Panel and other virtual folders the explorer cannot show.
  Win+E is a hotkey qol already knows how to grab.
  Residual: `explorer.exe /select` and `SHOpenFolderAndSelectItems` (reveal from other apps), the taskbar Explorer pin, and navigation inside Explorer windows stay in Explorer.
- Content types and URL schemes: the effective default lives in `UserChoice`, which carries a hash Windows checks, and on Windows 11 Home and Pro it is moving to `UserChoiceLatest`, whose hash adds a machine id.
  The UserChoice Protection Driver (`UCPD.sys`) also blocks non-Microsoft processes from writing `http`, `https`, `.pdf` and a few others.
  Writing the hash by reverse engineering it (SetUserFTA) breaks with each Windows rotation and fights the OS, so qol does not do it.
  Supported route: register the app (ProgID, `Capabilities`, `RegisteredApplications` under `HKCU`, all reversible), then open `ms-settings:defaultapps?registeredAppUser=<name>` so the user confirms with one click.
  This bends non-negotiable 1 (the user never configures the host), so the settings row says plainly that Windows requires the click.
  Admin-only alternative for Resident hosts: the `DefaultAssociationsConfiguration` policy XML, applied at sign-in.

### macOS (partial)

- qol apps need a real `.app` bundle declaring `CFBundleDocumentTypes` and `CFBundleURLTypes` before Launch Services will offer them as handlers; plugin binaries ship bare today.
- Content types and URL schemes: `NSWorkspace.setDefaultApplication(at:toOpen:)` and `toOpenURLsWithScheme:` (macOS 12 and later).
  Changing the `http` handler shows a system confirmation; document types do not.
- File manager: Finder cannot be replaced; it owns the desktop, the Dock and folder double-clicks.
  The global default `NSFileViewer` (`defaults write -g NSFileViewer -string <bundle id>`) routes reveal and open-folder calls from Cocoa apps to another app, which is how ForkLift and Path Finder take over "Reveal in Finder".
  Restore is `defaults delete -g NSFileViewer`, or the previous value.
  Users report it applying unreliably until a relaunch on some versions.

## Proposed shape

- A `default_apps` capability in `libs/apps` with one trait and three backends under `platform/`:
  `current(role) -> Option<Handler>`, `claim(role, handler, lifetime) -> Claim`, `release(claim)`.
  Each backend reports per role whether it can set the default silently, needs the user's click (Windows file types), or cannot (Finder folders), so the UI never offers what the OS will refuse.
- Plugins declare roles in `plugin.toml`, for example `[[default_handler]] role = "file_manager"`; the tray owns applying and restoring, the plugin never writes host state.
- The plugin's settings page gets one row per role, "Use Explorer as the default file manager", showing what the OS will and will not route (the residuals above).
- `qol-tray doctor` lists active claims and flags a claim whose recorded handler no longer exists.

## Order

1. Linux file manager: `inode/directory` claim with the PortableSession lifetime, explorer desktop entry, settings row. qol explorer is Linux-only today, so this is the first useful slice.
2. Windows `Directory`/`Drive` verbs, when qol explorer gains Windows.
3. Content types: Linux silent, Windows via the Settings click, macOS once plugins ship as `.app` bundles.
4. macOS `NSFileViewer`.

## Sources

- UCPD: https://www.bleepingcomputer.com/news/microsoft/new-windows-driver-blocks-software-from-changing-default-web-browser/
- UserChoiceLatest: https://kolbi.cz/blog/2025/04/20/userchoicelatest-microsofts-new-protection-for-file-type-associations/
- Windows folder verbs used by third-party managers: https://www.xyplorer.com/xyfc/viewtopic.php?p=41441
- `NSFileViewer`: https://stclairsoft.com/blog/?p=1182 and https://talk.macpowerusers.com/t/forklift-3-default-file-viewer-with-mac-os-catalina-10-15-7/33989
- `FileManager1` routing by bus name, not MIME default: https://bbs.archlinux.org/viewtopic.php?id=285355
