# Linux research: qol as the host's default apps

Scope: Linux only. Desktops: Cinnamon, GNOME, KDE Plasma, XFCE. Display servers: X11 and Wayland.

Evidence tags used in every table and list:

- V = verified from an official spec, official docs or upstream source (URL given).
- L = verified live on this machine (Linux Mint, Cinnamon 6.6.9, X11, glib 2.80, xdg-utils 1.1.3, xdg-desktop-portal 1.20, session bus is the reference dbus-daemon). Read-only commands only.
- R = a reproducible user or bug report (URL given), not upstream docs.
- I = inferred. Needs a VM test before design relies on it.

I could not fetch some upstream pages (freedesktop.org file-manager-interface returned 403, xdg-utils gitlab returned an access-denied page, several KDE and XFCE doc URLs returned 404). Where that left a gap the claim is tagged I.

## 0. Findings that change the design

1. There is no single "default" on Linux. Each job has two or more independent stores that different callers read. Example on this machine, default terminal: `org.cinnamon.desktop.default-applications.terminal exec` = `kitty` (L), `org.gnome.desktop.default-applications.terminal exec` = `x-terminal-emulator` (L), Debian alternative `x-terminal-emulator` = `gnome-terminal.wrapper` (L), and both `~/.config/xdg-terminals.list` and `~/.config/X-Cinnamon-xdg-terminals.list` say `org.gnome.Terminal.desktop` (L). Four stores, three answers. The facade needs a "set of stores" model per job, not one key.
2. Handler default and file-manager D-Bus name are separate slots. "Open this folder" resolves through `inode/directory` in mimeapps. "Reveal this file in its folder" resolves through the D-Bus name `org.freedesktop.FileManager1`. Taking one does not take the other.
3. A live D-Bus name owner cannot be displaced. Nemo, Nautilus and Dolphin all request `org.freedesktop.FileManager1` without `ALLOW_REPLACEMENT` (V, sources in section 3). So qol can only win the name when the incumbent is not running, by shadowing the activation file, or by being queued behind it.
4. On Wayland nobody can take a compositor-owned key by grabbing it. The only portable routes are: free the key in the compositor's own settings and bind it in qol through the GlobalShortcuts portal (GNOME 48+, KDE, Hyprland), or run an input-device reader (evdev). X11 `XGrabKey` works on X11 only and fails with `BadAccess` if someone already holds the key (I, standard X11 behaviour).
5. Sandboxed apps (Flatpak, Snap) do not read the host `mimeapps.list` themselves. They hand the request to a host portal or helper, and the host side applies its own mimeapps and its own remembered choices. A qol write to `~/.config/mimeapps.list` still governs them indirectly (I), but a Flatpak app cannot be forced through qol by that file alone.
6. Distro tooling lags: `xdg-open` 1.1.3 (Debian/Ubuntu/Mint) has no branch for `KDE_SESSION_VERSION=6` (L), and picks a different opener per desktop (L). Never depend on `xdg-open` as the only path. Call `gio`/GLib or the portal directly where possible, and keep `xdg-open` as the fallback.

## 1. Handler defaults: XDG mime-apps

### 1.1 File lookup and precedence

Source: [mime-apps spec, file location](http://specifications.freedesktop.org/mime-apps/latest/file.html) (V).

| # | Path | Note |
|---|---|---|
| 1 | `$XDG_CONFIG_HOME/$desktop-mimeapps.list` | user, desktop-specific, "advanced users" |
| 2 | `$XDG_CONFIG_HOME/mimeapps.list` | user, recommended place for settings GUIs |
| 3 | `$XDG_CONFIG_DIRS/$desktop-mimeapps.list` | admin/ISV, e.g. `/etc/xdg` |
| 4 | `$XDG_CONFIG_DIRS/mimeapps.list` | admin/ISV |
| 5 | `$XDG_DATA_HOME/applications/$desktop-mimeapps.list` | deprecated |
| 6 | `$XDG_DATA_HOME/applications/mimeapps.list` | deprecated |
| 7 | `$XDG_DATA_DIRS/applications/$desktop-mimeapps.list` | distro defaults |
| 8 | `$XDG_DATA_DIRS/applications/mimeapps.list` | distro defaults, e.g. `/usr/share/applications` |

- `$desktop` is each lowercase component of the colon-separated `XDG_CURRENT_DESKTOP`, tried in turn (V). On this machine the value is `X-Cinnamon`, so the desktop file is `x-cinnamon-mimeapps.list` (I from the rule; GLib builds the name as `%s/%s-mimeapps.list` from the lowercased desktop list, V: [gdesktopappinfo.c](https://raw.githubusercontent.com/GNOME/glib/main/gio/gdesktopappinfo.c)). The matching portals file here is `x-cinnamon-portals.conf` (L), which confirms the lowercase form.
- The spec text says desktop-specific files can only set defaults, not add or remove associations (V, same page).
- Local state (L): user file has `[Default Applications]` with `x-scheme-handler/http=firefox.desktop` and a second `[Added Associations]` section; the only system file is `/usr/share/applications/mimeapps.list` (it sets `inode/directory=nemo.desktop;caja-folder-handler.desktop;thunar.desktop`, `x-scheme-handler/mailto=thunderbird.desktop`). There is no `/etc/xdg/mimeapps.list` and no desktop-specific file.

### 1.2 Semantics

Sources: [default application](http://specifications.freedesktop.org/mime-apps/latest/default.html), [associations](http://specifications.freedesktop.org/mime-apps/latest/associations.html) (V).

- `[Default Applications]`: `type=a.desktop;b.desktop;` (semicolon list of desktop file IDs). Lookup walks the files in order. Within a list it takes the first entry that is installed and is actually associated with the type. If none, it moves to the next file. If no file yields one, it falls back to the most preferred application from associations.
- The walk runs from most specific MIME type to least specific. A handler for a specific type beats a default set for a parent type.
- `[Added Associations]` and `[Removed Associations]` act as if the `.desktop` file's `MimeType=` had that line. They apply only to desktop files in the same directory or a later (lower precedence) directory. Listing the same app and type in both is invalid.
- Consequence for qol: a stale ID (the app was uninstalled) is skipped silently and the next entry wins. A stale ID pointing at a removed Flatpak browser was reported to make `xdg-open` silently do nothing (R: [Flathub report summarised in search](https://discourse.flathub.org/t/desktop-used-apps-is-not-good-design/11114), weak source).

### 1.3 Read: "unset", "system default" and "set by someone"

| Question | How | Tag |
|---|---|---|
| Effective default | `gio mime <type>` (prints default, registered, recommended) or `xdg-mime query default <type>` or GLib `g_app_info_get_default_for_type` | L |
| Is it user-set? | Parse `$XDG_CONFIG_HOME/mimeapps.list` and the desktop-specific user file. Key present in the user file = set by the user or a tool. Absent = comes from system file or association order. | V (by spec order) |
| Is it system default? | Key absent in both user files, present in `/etc/xdg` or `/usr/share/applications` files. Example (L): `xdg-mime query default inode/directory` returns `nemo.desktop` although the user file has no such key. | L |
| Truly unset | No file lists it. Answer comes from `mimeinfo.cache` order (GLib reads `mimeinfo.cache` and `defaults.list` only in non-config dirs, V per glib source). | V |

Use the facade to read the user file text itself (an INI parse), not only the effective value, so "user set X" and "system says X" are distinguishable and the exact previous text is capturable.

### 1.4 Write

| Tool | What it writes | Tag |
|---|---|---|
| `xdg-mime default a.desktop type` | `$XDG_CONFIG_HOME/mimeapps.list` `[Default Applications]` (function `make_default_generic`). On KDE it also runs `make_default_kde` and `kbuildsycoca` to rebuild the sycoca cache. | L (xdg-utils 1.1.3 script) |
| `gio mime type a.desktop` / `g_app_info_set_as_default_for_type` | the user mimeapps file (exact path not fetched; current GLib writes the config-dir file) | I |
| `xdg-settings set default-web-browser a.desktop` | uses `xdg-mime default` for `text/html` and the `x-scheme-handler/{http,https,about,unknown}` types on GNOME3/Cinnamon; `kwriteconfig5 --file kdeglobals --group General --key BrowserApplication` on KDE; an `helpers.rc` edit on XFCE. Rolls back to the original value if the post-check fails. | L |
| `xdg-settings set default-url-scheme-handler scheme a.desktop` | `x-scheme-handler/<scheme>` via `xdg-mime`; KDE writes `emaildefaults` for mailto | L |

- No admin needed for per-user writes. A machine-wide default needs root and a file in `/etc/xdg/mimeapps.list` (V by spec; I that distros accept it everywhere).
- No OS prompt appears. Nothing signs or hashes the file. Any process running as the user can rewrite it.
- Concurrency: tools use `mv file.new file` (L, xdg-mime). qol should do the same atomic rename and preserve unknown groups, comments and key order so a revert diff is clean.
- Cache: GLib re-reads on change. KDE needs `kbuildsycoca6` or the equivalent (L script shows `kbuildsycoca${KDE_SESSION_VERSION}`; I for Plasma 6 naming).

### 1.5 Protection

None at the OS level. Policy and managed overrides can sit in `/etc/xdg/*mimeapps.list`; the user file still wins over it (V by precedence). Distros with immutable `/usr` just ship the lowest layer. GNOME's own settings UI and `xdg-settings` rewrite the same user file, so a third party can revert qol's write at any time (I). Detect that with the restore protocol in 1.8.

### 1.6 What the target app must be

- A `.desktop` file in `applications/` of `XDG_DATA_HOME` or `XDG_DATA_DIRS` with `Exec` using field codes (`%u`, `%U`, `%f`, `%F`), `MimeType=` listing the types (or an `[Added Associations]` entry), not `NoDisplay`/`Hidden` if it should appear in choosers, and `Categories` as needed. Spec: [Desktop Entry spec](http://specifications.freedesktop.org/desktop-entry/latest/) (V that it defines these keys; the key-by-key page was not readable through the fetch tool, so exact wording is I).
- For scheme handlers: `MimeType=x-scheme-handler/<scheme>;` (L: system file has `x-scheme-handler/http=firefox.desktop`).
- For D-Bus activation (`DBusActivatable=true`): the app needs a session-bus service file named after the desktop file ID, and must implement `org.freedesktop.Application` with `Activate(a{sv})`, `Open(as, a{sv})`, `ActivateAction(s, av, a{sv})` at an object path made from the well-known name (dots to slashes, dashes to underscores) (V: [D-Bus activation](http://specifications.freedesktop.org/desktop-entry/latest/dbus.html)).
- `update-desktop-database` rebuilds `mimeinfo.cache` for system installs. A user-dir desktop file in `~/.local/share/applications` is picked up by GLib directly (I), but an install should still run it for older toolkits.

### 1.7 Scope of takeover: who obeys the default and who bypasses it

| Caller | Path | Obeys mimeapps? | Tag |
|---|---|---|---|
| `xdg-open` on Cinnamon and GNOME3 branch | `gio open` | yes (GLib) | L |
| `xdg-open` on XFCE | `exo-open` first, then `gio open` | exo uses its own preferred-apps helpers for the four categories, then MIME for files | L for the order, I for exo's internals |
| `xdg-open` on KDE | `kde-open5` only when `KDE_SESSION_VERSION=5`, `kde-open` for 4; **no branch for 6 in 1.1.3** | KIO/KService, which reads mimeapps plus its sycoca cache | L for the script, I for KIO |
| `xdg-open` in unknown DE | `xdg-mime query default` for the file's type or `x-scheme-handler/<scheme>`; then `$BROWSER` or a hard-coded browser list | yes, with env override | L |
| `xdg-open` inside Flatpak | `gdbus` call to `org.freedesktop.portal.Desktop` `OpenURI` (detected by `/.flatpak-info`) | portal decides | L |
| GTK/GLib apps `g_app_info_launch_default_for_uri` | GLib | yes | V by glib source (reads desktop-specific lists) |
| Qt apps `QDesktopServices` | KIO on Plasma, otherwise `xdg-open` | mostly | I |
| Browsers and Electron apps | usually `xdg-open` or portal | yes | I |
| Apps that hard-code a path (`nemo`, `nautilus`, `firefox`) | direct exec | **no** | I |
| Terminal-emulator launches for `Terminal=true` entries | see section 4 | no | V |

`xdg-open` 1.1.3 runs `detectDE` from `XDG_CURRENT_DESKTOP`: `Cinnamon|X-Cinnamon` becomes `cinnamon`, `GNOME*` becomes `gnome`, `KDE`, `XFCE`, `MATE`, `LXQt`, etc. (L). The `KDE` match is exact, so `KDE` works but a compound value does not (L).

### 1.8 Restore and "did the user change it since"

Design that works with these files (I, built on V file semantics):

1. Before write, read the user mimeapps file bytes and record: path, the `[Default Applications] <type>` line (or "absent"), a content hash of the whole file, mtime. Store it in the claim ledger next to the existing hotkey claims (`libs/host-fixes/src/takeover`).
2. After write, record the exact line qol wrote.
3. On restore, re-read. If the line for that type still equals what qol wrote, put back the saved line (or delete the key if it was absent). If it differs, the user (or a tool) changed it: leave it, drop the claim, log it.
4. Never restore the whole file from a snapshot; other keys changed meanwhile. Edit only the keys qol touched.
5. If the saved previous app is no longer installed, do not write a dangling ID; remove the key (the next layer then answers).

### 1.9 Version spread

- Spec has been stable since 1.0 (desktop-specific files were added in later revisions; older distros may ignore `$desktop-mimeapps.list`, I).
- GLib older than 2.3x read `defaults.list`; modern GLib reads `mimeapps.list` only for config dirs (V per glib source note).
- Plasma 6 proposes shipping `kde-mimeapps.list` (R: [plasma-desktop MR 2030](https://invent.kde.org/plasma/plasma-desktop/-/merge_requests/2030)); its presence changes which file wins on KDE (I). Check `$XDG_CONFIG_DIRS` and `$XDG_DATA_DIRS` for `kde-mimeapps.list` at runtime.
- `xdg-utils` 1.1.3 is old; 1.2.x added KDE 6 and more (I; not fetched).

## 2. gio, xdg-mime, xdg-settings, schemes, inode/directory

| Tool | Reads | Writes | Notes | Tag |
|---|---|---|---|---|
| `gio mime` | GLib lists, with desktop-specific files | user mimeapps | prints "Default application for ..." plus registered and recommended | L |
| `xdg-mime query default` | script scans `check_mimeapps_list` over config and data dirs with the `$desktop-` prefix | n/a | independent re-implementation, can differ from GLib in edge cases | L (script) |
| `xdg-mime default` | n/a | `$XDG_CONFIG_HOME/mimeapps.list` | atomic `.new` + `mv` | L |
| `xdg-settings get/set` | wraps the above per DE | wraps the above plus KDE/XFCE files | verifies the target `.desktop` maps to a binary; rolls back on failure | L |

Facade rule: read through GLib (or a native INI parser following 1.1) and write through the same native code. Shelling out to `xdg-mime` works but pulls in DE-specific side effects and version differences. Use the tools only as an optional compatibility write (for KDE's `kbuildsycoca`).

`x-scheme-handler/*`:

- Browsers: `http`, `https` (plus `about`, `unknown` set by `xdg-settings`, L). `text/html`, `application/xhtml+xml` for local files.
- Mail: `x-scheme-handler/mailto`. Local system default here is `thunderbird.desktop` (L).
- Custom schemes: add a desktop file with `MimeType=x-scheme-handler/<scheme>;` and `Exec` with `%u`. Conflicts are resolved by the same list order.
- `xdg-settings` sets all four browser types together (L); a direct write that sets only `http` leaves `https` pointing elsewhere. Write all that the host tool writes, or the check `xdg-settings check default-web-browser` reports "no".

`inode/directory`:

- Default holder here: `nemo.desktop` from the system file (L). `gio mime inode/directory` also lists `org.gnome.baobab.desktop` as registered (L).
- `xdg-open <dir>` on Cinnamon, GNOME, MATE, XFCE reaches the `inode/directory` handler through `gio open` (L script) or exo.
- "Reveal in folder" is not `inode/directory`. See section 3.
- The file manager must declare `MimeType=inode/directory;` and accept a directory path or URI (`%U`).

## 3. org.freedesktop.FileManager1

Spec page: [File Manager DBus Interface](https://wiki.freedesktop.org/www/Specifications/file-manager-interface) (the fetch tool got 403; method list confirmed through the portal docs and live introspection below).

Interface (L, introspected live on this machine):

- `ShowFolders(as uris, s startup_id)`: show these folders.
- `ShowItems(as uris, s startup_id)`: open the parent folders and select the items.
- `ShowItemProperties(as uris, s startup_id)`: properties dialog.
- Property `OpenLocations` (as), emits change (Nemo extension; L).

Callers:

- The Flatpak/Snap OpenURI portal `OpenDirectory` method calls `FileManager1.ShowItems` and falls back to `OpenURI` on the parent directory if the service is missing (V: [OpenURI docs](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.OpenURI.html)).
- Chromium calls `ShowItems` (V: [Chromium platform_util_linux.cc](https://chromium.googlesource.com/chromium/src/+/cff8ad5/chrome/browser/platform_util_linux.cc)).
- Firefox and other apps use it for "show in folder" (R: the Arch thread in [KDE discuss](https://discuss.kde.org/t/default-applications-no-longer-being-respected/331/2)).

### 3.1 Who owns the name

| Desktop | File manager | Who holds the name and how | Tag |
|---|---|---|---|
| Cinnamon | Nemo | **`nemo-desktop` (the desktop-icons process) owns it for the whole session** (pid 1910 here); `nemo` is the activation target: `/usr/share/dbus-1/services/nemo.FileManager1.service` has `Exec=/usr/bin/nemo --no-default-window`. Nemo requests the name with `G_BUS_NAME_OWNER_FLAGS_NONE`. | L, V ([nemo-freedesktop-dbus.c](https://raw.githubusercontent.com/linuxmint/nemo/master/src/nemo-freedesktop-dbus.c)) |
| GNOME | Nautilus | activatable by D-Bus; registers with `g_bus_own_name_on_connection(..., G_BUS_NAME_OWNER_FLAGS_NONE)`; on name lost it only logs | V ([nautilus-freedesktop-dbus.c](https://raw.githubusercontent.com/GNOME/nautilus/main/src/nautilus-freedesktop-dbus.c)); service file presence I |
| KDE | Dolphin | `dolphin --daemon` (user unit `plasma-dolphin.service` or activation). Registers with `registerService(..., QueueService)`. Non-Flatpak builds install the file as `org.kde.dolphin.FileManager1.service`, not `org.freedesktop.FileManager1.service` | V ([dbusinterface.cpp](https://raw.githubusercontent.com/KDE/dolphin/master/src/dbusinterface.cpp), [CMakeLists.txt](https://raw.githubusercontent.com/KDE/dolphin/master/CMakeLists.txt)) |
| XFCE | Thunar | implements the interface (Thunar issue on `ShowItemProperties`) | R ([thunar #920](https://gitlab.xfce.org/xfce/thunar/-/work_items/920)); service file name I |

### 3.2 Can a second process claim it?

- D-Bus rules (V: [D-Bus spec, RequestName](https://dbus.freedesktop.org/doc/dbus-specification.html)): a second requester replaces the owner only if it passes `REPLACE_EXISTING` **and** the owner passed `ALLOW_REPLACEMENT`. Otherwise it is queued (return 2) or refused if `DO_NOT_QUEUE` (return 3).
- None of Nemo, Nautilus or Dolphin passes `ALLOW_REPLACEMENT` (V, sources above). So while the incumbent runs, qol cannot take the name. If qol requests it with queueing, it becomes owner when the incumbent exits (V by the queue rule).
- Activation shadow: qol can install `~/.local/share/dbus-1/services/org.freedesktop.FileManager1.service` with `Exec=<qol binary> --file-manager-service`. The D-Bus spec says that in different directories the higher-priority one wins, and leaves the order of user dir versus system dir to the implementation (V). Whether the user dir beats `/usr/share/dbus-1/services` on the **reference dbus-daemon** (this machine) and on **dbus-broker** (Fedora, Arch default) needs a VM test (I). Two service files declaring the same Name in the same directory is undefined behaviour (V).
- Multiple installed file managers already fight: Firefox "show in folder" launched Nautilus instead of Dolphin on Arch (R: [Arch thread](https://bbs.archlinux.org/viewtopic.php?id=304712)). Expect users with two file managers to show the same randomness; qol's takeover must work by removing ambiguity, not by adding another contender.
- Cinnamon special case (L): `nemo-desktop` is already running, so the activation shadow never fires. To take `FileManager1` on Cinnamon qol must either stop `nemo-desktop` (it draws the desktop icons, so that has a visible cost) or own the name before login finishes (autostart ordering, I). A pure "queue behind it" strategy would only win after Nemo exits.
- Once qol owns the name, it must implement all three methods and fall back to calling the real file manager if the target is not something qol shows (for example qol's Explorer refusing network URIs).

### 3.3 Restore

- If qol installed a user service file: delete it. If a user file existed already, keep its text in the ledger and put it back byte for byte.
- If qol registered by queueing at runtime, the name releases when qol exits, with no cleanup.
- Detect user change: the user service file hash differs from the one qol wrote, or the file is absent (user removed it): leave it.

### 3.4 Version spread

- Name and methods are stable since about 2011 (Nautilus 3.3.3 added the first implementation, V: [GNOME changes](https://download.gnome.org/sources/nautilus/3.3/nautilus-3.3.3.changes)).
- Dolphin support arrived in 2015 (V: [KDE bug 343016](https://bugs.kde.org/show_bug.cgi?id=343016)); the file-name change exists because multiple services with the same Name caused trouble (R).
- Flatpak builds of Dolphin keep the original file name (V, CMakeLists).

## 4. Portals, Flatpak, Snap

### 4.1 OpenURI and AppChooser

Sources: [OpenURI](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.OpenURI.html), [impl AppChooser](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.AppChooser.html) (V).

- Methods: `OpenURI` (no `file://`), `OpenFile` (fd, v2+), `OpenDirectory` (v3+, uses FileManager1 `ShowItems`, falls back to `OpenURI` of the parent), `SchemeSupported` (v5+).
- Options: `ask` (prompt for an app; if false the portal "may use a default or pick the last choice"), `writable`, `activation_token` (v4+).
- The docs do not say how the default is picked. I read this as: the host-side portal process looks up the handler with the host user's GLib mimeapps and applies remembered choices, and shows the AppChooser only when `ask` or no unique answer (I).
- AppChooser backend interface: `ChooseApplication(handle, app_id, parent_window, choices[], options)` where options carry `last_choice`, `uri`, `content_type`, `filename`, `modal`, `activation_token`. It returns the chosen app ID (V). So a backend implementer decides the dialog and may bypass mimeapps.

### 4.2 Do Flatpak and Snap apps ignore mimeapps?

- Flatpak: `xdg-open` inside the sandbox is replaced by a wrapper that calls the OpenURI portal (L: `open_flatpak` branch in xdg-open; R: [flatpak-xdg-utils notes](https://community.linuxmint.com/software/view/flatpak-xdg-utils)). `xdg-mime` and `xdg-settings` inside the sandbox cannot change host settings (R: Flathub thread above). The portal runs on the host and uses the host user's files (I). So host mimeapps still matters, but remembered per-app choices in the permission store can override it (R: [Flathub: desktop-used-apps](https://discourse.flathub.org/t/desktop-used-apps-is-not-good-design/11114), title only).
- Snap: apps use the core snap's `xdg-open` through the `desktop` plug; it opens with the host default for the type and cannot launch a chosen app. Opening files may need user confirmation (R: [snapcraft forum](https://forum.snapcraft.io/t/allowing-xdg-open-to-open-files/3789), [expose xdg-open](https://forum.snapcraft.io/t/expose-xdg-open-without-an-interface/1096)). The `desktop-launch` interface is a different feature, needs snapd 2.52+, and is not auto-connected (V: [docs](https://snapcraft.io/docs/desktop-launch-interface)).
- Sandboxed apps that talk to `org.freedesktop.FileManager1` directly need that D-Bus name permission; most reach it only through the portal (I).

### 4.3 Making qol the portal's choice

- Backend selection: `portals.conf` with `[preferred] default=...` and per-interface keys like `org.freedesktop.impl.portal.AppChooser=...`, found by lowercase `XDG_CURRENT_DESKTOP` names, searching `~/.config/xdg-desktop-portal`, `/etc/xdg/...`, `/etc/xdg-desktop-portal`, `~/.local/share/...`, `/usr/share/xdg-desktop-portal` (V: local `man portals.conf`). Local file: `/usr/share/xdg-desktop-portal/x-cinnamon-portals.conf` has `default=xapp;gtk;` (L).
- A backend is declared by a `.portal` file under `.../xdg-desktop-portal/portals/` with `DBusName=`, `Interfaces=`, `UseIn=` (L: `xapp.portal`: `UseIn=X-Cinnamon;MATE;XFCE;`).
- So qol could ship a portal backend for `AppChooser` or `FileChooser` and register it by a user `portals.conf`. That is a heavy surface (D-Bus service plus config) and affects every portal call, so treat it as a later stage (I).

## 5. Default terminal

| Store | Who reads it | Read | Write | Tag |
|---|---|---|---|---|
| `org.cinnamon.desktop.default-applications.terminal` (`exec`, `exec-arg`) | Cinnamon, Nemo "open in terminal", `cinnamon-settings-daemon` terminal key | `gsettings get` | `gsettings set` (local: `kitty`, `-e`) | L |
| `org.gnome.desktop.default-applications.terminal` | GNOME-family tools (older gnome-desktop users) | `gsettings get` | `gsettings set` (local: `x-terminal-emulator`, `-e`) | L |
| Debian alternative `x-terminal-emulator` | `xdg-open` (`term_run="x-terminal-emulator -e"`, or `$TERMINAL`), scripts | `update-alternatives --display x-terminal-emulator` (local: auto mode, `gnome-terminal.wrapper`) | `update-alternatives --set`, **root** | L |
| `xdg-terminals.list` (proposal) | `xdg-terminal-exec`, and GLib when that binary exists | file | file | see below |
| KDE `kdeglobals [General] TerminalService / TerminalApplication` | KIO, Dolphin | `kreadconfig6` | System Settings > Default Applications or `kwriteconfig6` | R ([discuss.kde.org](https://discuss.kde.org/t/dolphin-setup-default-open-terminal-here-outside-of-plasma/49305), [KDE docs](https://docs.kde.org/stable_kf6/en/plasma-workspace/kcontrol/componentchooser/)); key names I |
| XFCE preferred applications (`TerminalEmulator`, in `helpers.rc`) | `exo-open`, Thunar, panel | file | `xfce4-settings` (moved from exo at 4.15.1) | R ([exo-open man page](https://man.archlinux.org/man/exo-open.1.en)); path I |

### 5.1 xdg-terminal-exec

Proposed, not adopted (R: [GNOME Discourse](https://discourse.gnome.org/t/gnome-terminal-or-nautilus-disrespects-xdg-terminal-exec/26193); man page at [Arch](https://man.archlinux.org/man/extra/xdg-terminal-exec/xdg-terminal-exec.1.en)).

- Config: `${desktop}-xdg-terminals.list` first (desktop from `XDG_CURRENT_DESKTOP`), then `xdg-terminals.list`. Search order `$XDG_CONFIG_HOME`, `$XDG_CONFIG_DIRS` (`/etc/xdg`), then `$XDG_DATA_DIRS/xdg-terminal-exec` (R).
- Format: one desktop entry ID per line, optional `:action`, `#` comments (R).
- Entry rules: category `TerminalEmulator` and an `X-TerminalArgExec=` key; the reference tool has a compat mode that assumes `-e` (R).
- First valid listed entry wins; otherwise the first valid entry found (R).
- Local (L): `xdg-terminal-exec` is not installed (no `/usr/share/xdg-terminal-exec`), but both `~/.config/xdg-terminals.list` and `~/.config/X-Cinnamon-xdg-terminals.list` exist with `org.gnome.Terminal.desktop`. Someone wrote the desktop-specific file in the original case (`X-Cinnamon-`), not lowercase. Whether the reference tool lowercases is not confirmed (I), so write and read both spellings.
- A GNOME terminal extension reportedly rewrites these list files to make itself default (R, same Discourse thread). Another rewriter to guard against in restore.

### 5.2 GLib and `Terminal=true`

Current GLib `prepend_terminal_to_vector` tries, in order: `xdg-terminal-exec`, `kgx`, `gnome-terminal`, `mate-terminal`, `xfce4-terminal`, `tilix`, `konsole`, `nxterm`, `color-xterm`, `rxvt`, `dtterm`, `xterm`. It reads no gsettings key and no environment variable (V: [gdesktopappinfo.c](https://raw.githubusercontent.com/GNOME/glib/main/gio/gdesktopappinfo.c)). Debian's notes say it uses `xdg-terminal-exec` if that package is installed and otherwise prefers a GTK terminal even when `x-terminal-emulator` differs (R: [libglib2.0 README.Debian](https://doc.duckcorp.org/cgi-bin/dwww/usr/share/doc/libglib2.0-0t64/README.Debian)). The glib version that added it is not confirmed (R says commit log dates it to Oct 2022; this machine runs 2.80.0, so whether 2.80 has it is I). GLib issue [#1984](https://gitlab.gnome.org/GNOME/glib/-/issues/1984) asks for an override and is open as far as I found.

Consequence: the only knob that works for GLib launches is a `xdg-terminal-exec` binary on `PATH` (a qol-shipped wrapper) plus the list file. All other stores affect other callers.

### 5.3 Facade approach

Treat "default terminal" as "write every store the host reads, keep a per-store backup, restore per store". A reasonable first set on Linux: the DE's own store (Cinnamon gsettings, GNOME gsettings, KDE `kdeglobals`, XFCE helpers), plus `xdg-terminals.list` (desktop-specific name, both spellings). Do not touch `update-alternatives` (needs root, system-wide, would revert for all users).

## 6. Slot takeovers per desktop

The repo already does the Cinnamon part: `apps/tray/src/hotkeys/takeover/` lists `BINDING_ROOTS` for `/org/cinnamon/desktop/keybindings/`, `.../wm/`, `.../media-keys/`, `/org/cinnamon/muffin/keybindings/`, the GNOME wm, media-keys and shell roots, ibus, plus Cinnamon spice settings (read in `spices.rs`) (L, repo). The facade should reuse that ledger and add the missing desktops.

### 6.1 Cinnamon (X11; L unless marked)

| Slot | Where it lives | Current value here | How to free it | Notes |
|---|---|---|---|---|
| Launcher key (Super) | applet setting `overlay-key` in the Menu applet's spice JSON (`"type": "keybinding"`, default `Super_L::Super_R`) at `~/.config/cinnamon/spices/menu@cinnamon.org/<instance>.json`; `org.cinnamon.muffin overlay-key` is a separate global key | muffin `overlay-key` = `''`; no menu spice JSON exists here | edit the instance JSON, or remove the applet from `enabled-applets` | the JSON is the user's, so back up the value and hash |
| Alt+Tab | `org.cinnamon.desktop.keybindings.wm switch-windows`, `switch-windows-backward`, `switch-group` (`<Alt>Above_Tab`), `switch-panels` (`<Control><Alt>Tab`) | `switch-windows` = `@as []` (already freed) | set the key to `@as []`, then bind qol | style keys `org.cinnamon alttab-switcher-*` (V: [org.cinnamon.gschema.xml](https://raw.githubusercontent.com/linuxmint/cinnamon/master/data/org.cinnamon.gschema.xml)); a JS replacement is a Cinnamon extension listed in `enabled-extensions` (R: [CoverflowAltTab](https://github.com/p91paul/CoverflowAltTab)) |
| PrtSc family | `org.cinnamon.desktop.keybindings.media-keys`: `screenshot=['Print']`, `area-screenshot=['<Shift>Print']`, `window-screenshot=['<Alt>Print']`, plus `-clip` variants with `<Control>` | as shown | clear the keys, bind qol | handled by `csd-media-keys` (pid 1634, name `org.cinnamon.SettingsDaemon.KeybindingHandler`); the screenshot service is `org.gnome.Shell.Screenshot` owned by `cinnamon` itself, and `org.gnome.Screenshot` is an activatable name |
| Terminal and file keys | media-keys `terminal=['<Primary><Alt>t']`, `home=['<Super>e','XF86Explorer']`, `www=['XF86WWW']`, `search`, `calculator` | as shown | the key still runs Cinnamon's own command, which reads the gsettings default terminal and `inode/directory`; changing those stores re-points it | no need to take the key |
| Snapping | `org.cinnamon.muffin edge-tiling` (true here) plus WM keys `push-tile-*`, `push-snap-*`, `move-to-side-*`, `move-to-corner-*`, `maximize*`, `toggle-maximized=['<Alt>F10']` (most are `@as []` here) | as shown | set `edge-tiling=false` and clear tile keys, then run qol's own snapping | edge tiling is a pointer-drag feature; qol needs its own pointer hook (I) |
| Custom keys | `org.cinnamon.desktop.keybindings custom-list` (here `custom1,custom3,custom4,custom5`) with per-entry `name`/`command`/`binding` | here a user script is on `custom1` | add an entry | the older `org.cinnamon.keybindings.custom-keybinding` schema is deprecated (V, schema) |
| Panel applets | `org.cinnamon enabled-applets`: strings `panel:zone:position:uuid:instance` (here e.g. `panel1:right:3:xapp-status@cinnamon.org:4`); settings in `~/.config/cinnamon/spices/<uuid>/<instance>.json` | as shown | remove the string to hide the applet, or replace its uuid | Cinnamon watches the key live (I); applet JSON schemas live in `/usr/share/cinnamon/applets/<uuid>/settings-schema.json` (L for Menu) |

Protection: dconf has no signing. Cinnamon/muffin warn of nothing; a conflicting binding is accepted and the first grab wins (I). Hard reverts come from Cinnamon Settings > Keyboard "Reset to defaults" (I).

Wayland: schemas `org.cinnamon.muffin.wayland` and `.wayland.keybindings` exist (L), so a Wayland session is in development in 6.x (I about maturity). Treat Cinnamon-on-Wayland as unsupported until a VM test.

### 6.2 GNOME

Verified schemas (V: [gnome-shell schema](https://raw.githubusercontent.com/GNOME/gnome-shell/main/data/org.gnome.shell.gschema.xml.in), [mutter schema](https://raw.githubusercontent.com/GNOME/mutter/main/data/org.gnome.mutter.gschema.xml.in)):

| Slot | Schema and key | Default |
|---|---|---|
| Super key (Overview) | `org.gnome.mutter overlay-key` | `'Super'` |
| App grid | `org.gnome.shell.keybindings toggle-application-view` | `<Super>a` |
| Overview toggle | `org.gnome.shell.keybindings toggle-overview` | none |
| Screenshot UI | `org.gnome.shell.keybindings show-screenshot-ui`, `screenshot`, `screenshot-window`, `show-screen-recording-ui` | `Print`, `<Shift>Print`, `<Alt>Print`, `<Ctrl><Shift><Alt>R` |
| Clipboard-like tray | `toggle-message-tray` | `<Super>v`, `<Super>m` |
| Dock favourites | `switch-to-application-1..9` | `<Super>1..9` |
| Snapping | `org.gnome.mutter.keybindings toggle-tiled-left/right` | `<Super>Left`, `<Super>Right` |
| Windows switcher | `org.gnome.desktop.wm.keybindings switch-applications`, `switch-windows`, `switch-group` | distro-dependent; on this Mint box `switch-applications=['<Super>Tab']`, `switch-windows=@as []` (L) |
| Custom keys | `org.gnome.settings-daemon.plugins.media-keys custom-keybindings` (list of relocatable-schema paths with `name`, `command`, `binding`) | none |
| Terminal / www / home / email | same media-keys schema keys `terminal`, `www`, `home`, `email` (L, key names) | per distro |

- GNOME Shell on Wayland: `gsd-media-keys` can fail to register an accelerator that conflicts (R: [GNOME Discourse regression thread](https://discourse.gnome.org/t/noticed-regressions-in-some-keyboard-shortcuts/28233)).
- GlobalShortcuts portal: GNOME 48 supports it (V: [GNOME 48 developer notes](https://release.gnome.org/48/developers/); the backend landed in xdg-desktop-portal-gnome 48.rc, R: [Artix package changelog](https://packages.artixlinux.org/packages/world/x86_64/xdg-desktop-portal-gnome)). Before 48 no portal route exists.
- gnome-shell extensions (JavaScript, per shell version) are the only way to replace Overview, the top bar and the app switcher UI (I). They need user opt-in and break on shell upgrades.
- Panel applets: GNOME has no applet list. Status icons are drawn by extensions (I).

### 6.3 KDE Plasma

| Slot | Mechanism | Tag |
|---|---|---|
| Any global shortcut | `kglobalacceld` D-Bus service `org.kde.KGlobalAccel`; actions belong to a "component" (app); methods include `doRegister`, `setShortcut`, `setForeignShortcut`, `unregister`; flags `SetPresent=2`, `NoAutoloading=4`, `IsDefault=8`; state persisted in `~/.config/kglobalshortcutsrc` | V for names/flags ([kglobalacceld.h](https://lxr.kde.org/source/plasma/kglobalacceld/src/kglobalacceld.h)); persistence file I |
| Conflicts | The KDE README says: assigning a key twice tells the older owner its shortcut became empty. The `setShortcutKeys` source I fetched contains no conflict check, and "steal" is a separate call. Net: the exact steal path needs a Plasma VM test | R: [kglobalaccel README](https://lxr.kde.org/source/graphics/krita/libs/widgetutils/xmlgui/README); V: [kglobalacceld.cpp](https://raw.githubusercontent.com/KDE/kglobalacceld/master/src/kglobalacceld.cpp) |
| Editing the file | Sysadmin advice: do not edit `kglobalshortcutsrc` directly; use `doRegister`/`setShortcut` over D-Bus because file edits need a daemon restart | R: [KDE forum](https://forum.kde.org/viewtopic.php%3Ff=43&t=109524.html) |
| Super key launcher | `~/.config/kwinrc [ModifierOnlyShortcuts] Meta=org.kde.plasmashell,/PlasmaShell,org.kde.PlasmaShell,activateLauncherMenu`, applied with `qdbus org.kde.KWin /KWin reconfigure` (Plasma 5-era reports); Plasma 6 reports say clearing it by file no longer works and the shortcut is managed in System Settings > Shortcuts > plasmashell | R: [Manjaro thread](https://forum.manjaro.org/t/super-l-meta-windows-key-permanently-bound-to-app-menu-2/169132); treat as I for Plasma 6 |
| Alt+Tab | KWin "Walk Through Windows" global shortcuts plus tabbox config in `kwinrc` | I |
| PrtSc | Spectacle registers its own global shortcuts as a component | I |
| Snapping | KWin "Window Quick Tile ..." shortcuts, edge tiling in `kwinrc` | I |
| Panel applets | `plasma-org.kde.plasma.desktop-appletsrc` and plasmashell D-Bus scripting `evaluateScript` | I |
| File manager default | `kdeglobals`, mimeapps, Dolphin FileManager1 | see sections 1 and 3 |

Wayland: KWin is the compositor, so key interception has no X grab; KDE's portal backend advertises GlobalShortcuts (R: [GNOME issue #85](https://gitlab.gnome.org/GNOME/xdg-desktop-portal-gnome/-/issues/85)), and `kglobalaccel` D-Bus works on both X11 and Wayland (I).

### 6.4 XFCE

| Slot | Mechanism | Tag |
|---|---|---|
| App shortcuts | xfconf channel `xfce4-keyboard-shortcuts`, properties `/commands/default/<keys>` and `/commands/custom/<keys>`, value = command | R: [XFCE forum](https://forum.xfce.org/viewtopic.php?id=15826) |
| WM shortcuts | same channel, `/xfwm4/default/<keys>` and `/xfwm4/custom/<keys>`, value = window-manager action (`cycle_windows_key`, `tile_left_key`, ...) | R |
| Write | `xfconf-query -c xfce4-keyboard-shortcuts -np '/commands/custom/<Primary><Alt>e' -t string -s cmd` | R |
| Remove | `xfconf-query ... --reset -p <property>` | R |
| Edit while running | use xfconf-query, not the XML, because xfconfd writes memory to the XML | R |
| Duplicate keys | Reports disagree on whether a second binding of the same key overrides or is added; test | R, I |
| Super key | not bound by default in XFCE (I); users bind `xfce4-popup-whiskermenu` | I |
| Preferred apps | `TerminalEmulator`, `WebBrowser`, `MailReader`, `FileManager` categories, read by `exo-open`; GUI moved to xfce4-settings in 4.15.1 | R ([docs](https://docs.xfce.org/xfce/exo/preferred-applications), [man](https://man.archlinux.org/man/exo-open.1.en)) |
| Panel plugins | `xfce4-panel` channel, plugin IDs per panel | I |

xfwm4 is X11 only; XFCE's Wayland work is experimental (I). `xdg-open` on XFCE runs `exo-open` first (L script), so exo's preferred-apps store can answer before mimeapps does for the four categories.

### 6.5 X11 key grabs versus Wayland

| Mechanism | X11 | Wayland | Tag |
|---|---|---|---|
| `XGrabKey` (e.g. the repo's `global-hotkey` crate) | works; fails with `BadAccess` if another client grabbed the same key+modifiers | does nothing for native Wayland windows; XWayland clients get no global keys | I (X11), V (Wayland limit) |
| Wayland exclusive grab | n/a | "no Wayland client can grab the keyboard exclusively; an XWayland client is no different" | R: [Red Hat bug 1285770](https://bugzilla.redhat.com/show_bug.cgi?id=1285770) |
| XWayland keyboard grab for an allow-listed client | n/a | mutter allows it if `xwayland-allow-grabs` is set, the client class is allow-listed in `xwayland-grab-access-rules`, or it sets `_XWAYLAND_MAY_GRAB_KEYBOARD`; a user shortcut `restore-shortcuts` breaks the grab | R: [mutter xwayland grab commit/search](https://git.gnome.org/browse/mutter/commit/?id=072afa5) |
| GlobalShortcuts portal v2 | works where a portal backend exists | `CreateSession`, `BindShortcuts` (shows a dialog; one attempt per session; `preferred_trigger` is only a suggestion), `ListShortcuts`, `ConfigureShortcuts` (v2), `Activated`/`Deactivated` signals with `activation_token` | V: [GlobalShortcuts docs](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html) |
| Backends | KDE advertises it (R), GNOME 48+ (V), Hyprland (R: [niri PR 1165](https://github.com/YaLTeR/niri/pull/1165)); wlroots generic and Cinnamon/XFCE: none found | | mixed |
| evdev reader | needs `input` group or udev rule; sees keys regardless of focus; cannot swallow keys unless it also grabs the device and re-injects (uinput) | same | I; the repo already has a `linux_evdev` path |

Design consequence: for a slot owned by the compositor (Super, Alt+Tab, PrtSc), the takeover has two steps on every desktop. Step 1 frees the key in the compositor's own settings (gsettings, kglobalaccel, xfconf, spice JSON). Step 2 binds it in qol: `XGrabKey` or evdev on X11; the GlobalShortcuts portal on Wayland where it exists. The portal asks the user, so Wayland takeover is never silent.

## 7. Restore and change detection for slot keys

Same protocol as 1.8, applied per store (I, consistent with the existing claim ledger in `libs/host-fixes/src/takeover`):

1. Record the key's full path, the exact previous GVariant text (or "default", via `gsettings get` vs `dconf read` returning empty), and whether the dconf key was user-set (dconf read returns data) or schema default (returns nothing).
2. After writing record qol's value.
3. At restore, read again. Equal to qol's value: restore (write old value, or `dconf reset` if it was default). Different: the user changed it, so drop the claim and leave the value.
4. For spice JSON and XML files, keep the old value of that key plus the file hash; restore edits only that key.
5. For KGlobalAccel and xfconf, record the previous shortcut-to-action mapping the same way.

## 8. Open items that need a VM test before the design locks

1. Does `~/.local/share/dbus-1/services/org.freedesktop.FileManager1.service` beat `/usr/share/dbus-1/services/...` on dbus-daemon and on dbus-broker? What does the daemon do with two services of the same Name in different dirs?
2. Can qol take `FileManager1` on Cinnamon while `nemo-desktop` runs, without hiding desktop icons?
3. Plasma 6: exact conflict behaviour of `setShortcut`, and whether `[ModifierOnlyShortcuts]` still works.
4. Which GLib version has `xdg-terminal-exec` first, and does it take effect on Mint (GLib 2.80)?
5. Does the Flatpak/portal side honour a new user mimeapps entry immediately, or via remembered choices in the permission store?
6. Case of the desktop-specific `xdg-terminals.list` name: lowercase or original.
7. XFCE: duplicate-key binding behaviour and what the Super key does on a fresh install.

## 9. Source index

- mime-apps spec: http://specifications.freedesktop.org/mime-apps/latest/ (file.html, default.html, associations.html)
- Desktop Entry spec (D-Bus activation): http://specifications.freedesktop.org/desktop-entry/latest/dbus.html
- D-Bus specification: https://dbus.freedesktop.org/doc/dbus-specification.html
- OpenURI: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.OpenURI.html
- GlobalShortcuts: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html
- impl AppChooser: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.AppChooser.html
- portals.conf: local `man portals.conf` (xdg-desktop-portal 1.20), mirrors upstream docs
- GLib `gdesktopappinfo.c`: https://raw.githubusercontent.com/GNOME/glib/main/gio/gdesktopappinfo.c
- Nemo / Nautilus / Dolphin FileManager1: nemo-freedesktop-dbus.c, nautilus-freedesktop-dbus.c, dolphin dbusinterface.cpp and CMakeLists.txt (URLs inline above)
- Cinnamon schema: https://raw.githubusercontent.com/linuxmint/cinnamon/master/data/org.cinnamon.gschema.xml
- gnome-shell and mutter schemas (URLs inline in 6.2)
- GNOME 48 developer notes: https://release.gnome.org/48/developers/
- kglobalacceld: https://lxr.kde.org/source/plasma/kglobalacceld/src/kglobalacceld.h and the cpp (URL inline)
- xdg-terminal-exec man: https://man.archlinux.org/man/extra/xdg-terminal-exec/xdg-terminal-exec.1.en
- Local evidence: `/usr/bin/xdg-open`, `/usr/bin/xdg-mime`, `/usr/bin/xdg-settings` (xdg-utils 1.1.3), `gsettings`, `dconf`, `busctl --user`, `/usr/share/dbus-1/services`, `/usr/share/xdg-desktop-portal`, user and system `mimeapps.list`
