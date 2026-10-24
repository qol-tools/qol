# macOS research: qol as the host's default apps (macOS 13 Ventura to 26 Tahoe)

Date of research: 2026-10-24. Scope: macOS only.

Legend: **V** = verified (Apple docs, source code, or a reproducible first-hand report). **V-community** = a named person reports it, not Apple. **I** = inferred (my reasoning, or a source that is thin, old, or secondary). Where a fetch of Apple's own page failed, I say so.

Bottom line first:

1. Handler defaults are settable with public API (`NSWorkspace setDefaultApplication...`), but since macOS 26.4 every content-type (UTI) change shows a user consent sheet, and `http` has prompted since High Sierra. Restoring is a change too, so restore also prompts. A silent "hand back on quit" is not possible for those.
2. Writing the LaunchServices plist directly is not viable on 26. It is unreliable on 13 to 25 and bypasses nothing durable.
3. A handler must be an `.app` bundle with Info.plist declarations. A bare CLI binary cannot be a handler.
4. Folders: Finder owns `public.folder`. The `NSFileViewer` global default exists and some apps honor it, but nothing shows AppKit's reveal calls honor it, and on 26.6.2 binding `public.folder` fails validation.
5. Slots: Spotlight, screenshot and Cmd+Tab keys are all "symbolic hotkeys" and can be switched off per user without admin or TCC. The plist write plus `activateSettings -u` applies live in most reports; the private SkyLight call `CGSSetSymbolicHotKeyEnabled` is the live, direct route. Cmd+Tab is symbolic hotkeys 1 and 2. Tiling keys and the dictation key are not symbolic hotkeys as far as I could find.

---

## 1. Handler defaults through Launch Services

### 1.1 The APIs

| API | Status | Notes |
|---|---|---|
| `LSSetDefaultRoleHandlerForContentType(uti, roles, bundleId)` | Deprecated since macOS 12, still works | Takes an `LSRolesMask`. The NSWorkspace replacement has no role parameter, so the Shell role cannot be targeted with the new API. **V-community** (dooti README; Apple DTS thread 741305) |
| `LSSetDefaultHandlerForURLScheme(scheme, bundleId)` | Deprecated since macOS 12, still works | Apple DTS (Mar 2025): "There is no recommended replacement" for some uses, keep using it in the interim. A Tahoe user reports `LSSetDefaultHandlerForURLScheme("tel", ...)` through `osascript -l JavaScript` works on 26. **V** (DTS thread 769441), **V-community** (tel thread 800777) |
| `-[NSWorkspace setDefaultApplicationAtURL:toOpenContentType:completionHandler:]` | macOS 12.0+ | Apple doc text: "If a change requires user consent, the system asks the user for consent asynchronously before invoking the completion handler." **V** |
| `-[NSWorkspace setDefaultApplicationAtURL:toOpenURLsWithScheme:completionHandler:]` | macOS 12.0+ | Same consent sentence. **V** |
| `-[NSWorkspace setDefaultApplicationAtURL:toOpenFileAtURL:completionHandler:]` | macOS 12.0+ | A per-file variant exists (it appears in Apple's NSWorkspace symbol list). I did not read its page. **I** |

Sources:
- https://developer.apple.com/documentation/appkit/nsworkspace/setdefaultapplication(at:toopenurlswithscheme:completion:).md
- https://developer.apple.com/documentation/appkit/nsworkspace/setdefaultapplication(at:toopen:completion:).md
- https://developer.apple.com/forums/thread/741305
- https://developer.apple.com/forums/thread/769441
- https://developer.apple.com/forums/thread/800777
- https://www.github.com/lkubb/dooti

I could not load Apple's own deprecation pages for the two LS functions (404 on the old doc URLs), so the "deprecated in 12.0" claim rests on dooti plus the DTS wording ("deprecated in favour of"). Treat the exact version as **V-community**.

### 1.2 Completion handler behavior

- The handler gets an optional `Error`. It runs after the consent sheet is answered. **V** (Apple doc).
- A declined consent ("Keep" button) comes back as a non-success result. Chromium handles it by returning early so it does not prompt a second time for `https`: "If the user declined the system permission dialog (e.g., 'Keep Safari' was chosen)". **V** (Chromium source). The exact error domain and code for a decline are not documented. **I**
- A prompt that nobody answers stalls the caller indefinitely: Scripting OS says an ignored prompt "stalls the script or service". **V-community**. So qol must treat every set call as async with a timeout and never block shutdown on it.
- Chromium's own comment: "Passing empty completion handlers ... is kinda broken", so implement the handler. **V**
- Sandboxed callers: both the LS and NSWorkspace setters fail with a generic error whose underlying error is -54 (`permErr`) under the App Sandbox; Mac App Store apps are blocked. **V** (Apple DTS, threads 800777 and 769441). qol ships outside the Mac App Store, so this does not apply unless the .app is ever sandboxed.

Sources: https://chromium.googlesource.com/chromium/src/+/main/chrome/browser/shell_integration_mac.mm and https://scriptingosx.com/2026/03/macos-26-4-brings-more-default-app-confirmation-prompts/

### 1.3 Which changes show a user confirmation

| Change | 13 Ventura to 25 | 26.0 to 26.3 | 26.4 and later | Source |
|---|---|---|---|---|
| `http` scheme (default browser) | Prompt ("Use" / "Keep"), since High Sierra | Prompt | Prompt | Scripting OS; utiluti README. **V-community** |
| `https` scheme | Chromium sets http then https and guards against "being prompted a second time" after a decline, so expect up to two sheets | same | same | Chromium comments. **V** that it can prompt; the exact count is **I** |
| `mailto` and other schemes | No prompt reported | No prompt reported | Not reported to prompt; Scripting OS says 26.4 added prompts for "all file type/UTI default app changes", not schemes | **I** (absence of a report, no first-hand test) |
| Content types / UTIs through the API | No prompt | No prompt | Prompt for every change | Scripting OS: "Through 26.3 ... could be changed through the proper APIs without user interaction". **V-community** |
| Content type changed in the Finder Get Info window | No extra prompt | same | No extra prompt | Scripting OS. **V-community** |

Extra details on 26.4:
- The sheet has "Use" and "Keep". It is drawn by CoreServicesUIAgent (bundle id `com.apple.coreservices.uiagent`), which "sometimes orders the dialog behind the Chrome window"; Chromium works around it by activating that agent with `NSApplicationActivateAllWindows`. **V** (Chromium source)
- Chromium skips setting the HTML content type on 26.4+ because "setting the HTML content type triggers a separate Finder confirmation dialog" (a second sheet). **V** (Chromium source, `MacOSVersion() < 26'04'00`).
- The set of triggering changes moved during the 26.4 beta and "may change again". **V-community**
- Apple gives no MDM profile or declaration to manage default apps on macOS. The MDM default-app commands exist on iOS and visionOS only. **V-community** (Scripting OS)
- Tahoe removed the "Default for calls" picker for `tel:` from FaceTime and the Phone app; Apple DTS called it "bugworthy" (FB20321931). **V** (thread 800777)

Design consequence: restoring a previous handler after qol exits is itself a default change. On 26.4+ that means a consent sheet for each content type and for `http`. For Portable mode ("hand it back when qol stops") this means restore cannot be silent on those slots. qol should keep its macOS handler takeovers to cases that tolerate a prompt at takeover and at restore, and should not take `http`/`https` by default.

### 1.4 The plist: `com.apple.launchservices.secure`

- Path: `~/Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist`. Shell users address it as `defaults write com.apple.LaunchServices/com.apple.launchservices.secure LSHandlers -array-add '{...}'`. **V-community** (MPU Talk thread)
- Entry shape seen in the wild: `{LSHandlerContentType="public.folder"; LSHandlerRoleAll="com.apple.finder";}`, and for schemes an `LSHandlerURLScheme` key. **V-community** (MPU Talk thread; Stack Overflow question 56929742)
- Reading: an explicit entry in `LSHandlers` means someone set the handler. No entry means the system default. The NSWorkspace read calls return only the resolved effective handler and cannot tell "unset" from "set to the same value as the default". **I**
- Writing it directly is not viable for qol:
  - A 2019 report says edits to the plist were not reflected in `lsregister -dump` until a restart, and killing `lsregister` or running `lsregister -seed` did not help. **V-community** (https://windows-hexerror.linestarve.com/q/so56929742-adding-and-deleting-lshandlerurlscheme-from-comapplelaunchservicessecureplist-is-not-immediately-ref)
  - Scripting OS (2026) says direct plist edits to bypass prompts "are generally reliable only early in enrollment and must be followed by a logout or restart", and the supported shape is the User Template path (plist exists before the account does). **V-community**
  - On 26.6.2 a user reports "manual resets via lsregister or defaults write do not persist because the OS invalidates the handler upon validation", with the log line "changing default handler: bindable public.folder failed validation" from `com.apple.launchservices:uiagent`. **V-community** (ForkLift 4.7.4 post comments, Aug 2026). ForkLift's developer replied "Apple appears to have changed this behavior".
  - Writing it directly also races `cfprefsd` and `launchservicesd` caches. **I**
- Use of the plist for qol: read-only, to learn whether an explicit entry exists and what it says, as the "before" snapshot. Writes go through NSWorkspace.

Other LaunchServices facts for 13 to 26:
- `lsregister -kill` (registry reset) was removed from Tahoe. `lsregister -u` almost always fails with -10814 on Sequoia and Tahoe. Recent macOS registers apps from almost any accessible folder. **V-community** (Eclectic Light, 2026-08-04 and 2025-03-27). So a freshly dropped qol .app is picked up without a manual `lsregister -f`, but a stale or duplicate copy elsewhere on disk can also be registered. **I**

### 1.5 What the target app must be

Declarations in the handler app's Info.plist (Apple Info.plist key reference, **V**):

| Key | Purpose |
|---|---|
| `CFBundleDocumentTypes` | Array of dicts. Each has `LSItemContentTypes` (UTIs, spelled out), `CFBundleTypeRole` (required: `Editor`, `Viewer`, `Shell`, `None`), and `LSHandlerRank`. |
| `LSHandlerRank` | `Owner`, `Default`, `Alternate`, `None`. Precedence is Owner, then Default, then Alternate. Missing means `Default`. |
| `CFBundleURLTypes` | Array of dicts with `CFBundleURLSchemes` and `CFBundleTypeRole` (required) for schemes such as `mailto`, `http`, custom. |
| `UTImportedTypeDeclarations` | Declares a UTI the system does not know, so `LSItemContentTypes` can name it. Not covered on the page I fetched (it points to the UTI docs). Standard practice. **I** |

Source: https://developer.apple.com/library/archive/documentation/General/Reference/InfoPlistKeyReference/Articles/CoreFoundationKeys.html

How Launch Services breaks ties between equal ranks is not documented in anything I found. **I** (unknown).

Real examples of a terminal-like app claiming to be the default:
- Ghostty 1.3 ships "set as default terminal" using `NSWorkspace.setDefaultApplication(at:toOpen:)` with the `unixExecutable` and `shellScript` content types, minimum macOS 13. **V-community** (https://github.com/ghostty-org/ghostty/discussions/8111)
- SonicTerm's plan claims `public.shell-script` and `com.apple.terminal.shell-script` with the Shell role, adds `public.unix-executable`, and refuses to run unless the bundle is a real `.app` with its identifier, not translocated, not on a mounted DMG. **V-community** (https://github.com/D0n9X1n/SonicTerm/issues/1654, a plan, not shipped code)
- Chromium uses the outer app bundle (not the helper) for browser/UTI changes and the main bundle for scheme shortcuts. **V**
- The third-party tool `openwith` greys out system-critical types (apps, Unix executables, folders, volumes, `public.data`, the `file` scheme) and says changing URL schemes "may trigger a macOS confirmation prompt". That is a product choice, not an OS block, because Ghostty does take `unixExecutable`. **V-community** (https://github.com/hza/openwith)

Can a bare CLI binary be a handler? No:
- Every setter takes an application URL or bundle id and resolves a registered bundle. **V** (API signatures)
- Handler capability comes from Info.plist declarations of a bundle. **V**
- The handler also has to receive the open event: an `.app` receives `application:openURLs:` / Apple Events. A plain Mach-O has no such channel. **I**

Practical shape for qol: a minimal `.app` (can be `LSBackgroundOnly` or `LSUIElement`) that declares the roles and forwards the open event over the daemon socket to the right plugin. The tray's own bundle is the natural host if it is already a signed `.app`. Code-signing requirements for being chosen as a default handler: none found; unsigned apps get registered, but Gatekeeper still applies on first launch. **I**

Reading the current default (all macOS 13 to 26):
- `-[NSWorkspace URLForApplicationToOpenURL:]` for a scheme, `URLForApplicationToOpenContentType:` for a UTI, `URLsForApplicationsToOpenURL:` / `URLsForApplicationsToOpenContentType:` for candidates. Chromium reads defaults this way. **V**
- Deprecated `LSCopyDefaultRoleHandlerForContentType` / `LSCopyAllRoleHandlersForContentType` remain the only way to filter by role (Shell). Apple DTS says keep using LS calls where NSWorkspace lacks the feature. **V** (thread 741305)

### 1.6 Restore and detecting a user change

Recommended record (my design, **I**), per slot:
`{key, previous_effective_handler, previous_explicit_plist_entry_or_none, our_handler, written_at}`.

- At restore: read the current default. If it differs from `our_handler`, the user changed it since, so do not restore. If it equals `our_handler`, call the setter with the previous handler.
- If the previous state was "no explicit entry" the NSWorkspace setters cannot remove an entry; they can only set the app that was the effective default (for example `com.apple.finder` or `com.apple.Safari`). That leaves an explicit entry where there was none. Removing the entry again needs a plist edit, with the reliability problems in 1.4. **I** Accept the explicit entry as a known restore residue.
- Each restore call can prompt (1.3), and the user can answer "Keep". Mark that slot as "restore declined, user owns it".

### 1.7 Scope of the takeover

- Everything that goes through `open` / `NSWorkspace openURL:` / double-click in Finder / Apple Events obeys the default. **V-community** (general LS contract)
- Apps that hardcode a bundle id (for example "reveal in Finder" via AppleScript `tell application "Finder"`) bypass it. **I**
- Browsers that embed their own URL handling are unaffected by the `http` default for in-app links.

---

## 2. Folders and "reveal in folder"

### 2.1 What owns it

- `public.folder` is bound to Finder (`com.apple.finder`). The MPU Talk reversal command writes `{LSHandlerContentType="public.folder";LSHandlerRoleAll="com.apple.finder";}` to restore Finder, which shows this binding is plist-backed. **V-community**
- The old global key `NSFileViewer`: `defaults write -g NSFileViewer -string <bundle id>`; revert with `defaults delete -g NSFileViewer`. **V-community** (St. Clair Software blog)
- Who honors `NSFileViewer`: Default Folder X ("Default Folder X respects the system's NSFileViewer setting"), and ForkLift's "default file viewer" feature is built on it. St. Clair warns "other applications may also start using that app ... so results may vary". **V-community** (https://stclairsoft.com/blog/?p=1182)
- Path Finder's "default file browser" checkbox "makes other apps use Path Finder for Reveal in Finder commands"; the same vendor note says it "may not work in all applications". **V-community** (https://www.stclairsoft.com/blog/?p=2000, and Path Finder 4.8.2 release note via iclarified)

### 2.2 Which APIs honor it

- `NSWorkspace activateFileViewerSelectingURLs:` and `selectFile:inFileViewerRootedAtPath:` are documented as activating Finder and selecting the files (Monkeybread's wrapper description: "Activates the Finder, and opens one or more windows selecting the specified files"). An Apple forum reply says it works through the same path as the "Show In Finder" service and needs no file entitlement. **V-community** (https://developer.apple.com/forums/thread/667569 and the MBS page). I found no source showing these calls read `NSFileViewer`. **I** that they do not. The method name "in file viewer" is a legacy hint only.
- Dock stack "Show in Finder" did not honor `NSFileViewer` or a `public.folder` binding on Catalina, even after restart; one other user saw it work after some reboots only. **V-community** (https://talk.macpowerusers.com/t/forklift-3-default-file-viewer-with-mac-os-catalina-10-15-7/33989)
- Launchers: a Setapp review (Mar 2026) says Path Finder's setting stopped working after 26.4 for "launching a folder from Alfred", which now opens Finder. **V-community**, thin.

### 2.3 Version spread

| Version | Behavior | Source |
|---|---|---|
| 13 to 25 | `NSFileViewer` plus `public.folder` binding in the LS plist: works for apps that read the key; unreliable for system paths | **V-community** |
| 26.0 to 26.3 | No specific report found | none |
| 26.4 | Reports that third-party folder launching falls back to Finder; all UTI changes now prompt (`public.folder` is a UTI) | **V-community** |
| 26.6.2 | `bindable public.folder failed validation`; third-party apps including Raycast fall back to Finder; resets do not persist | **V-community** (ForkLift comments, 2026-08-20 and 2026-08-24) |

### 2.4 Consequence for the design

On macOS the Explorer takeover cannot be a system-wide default. Only paths qol controls can obey it: qol's own "reveal" and "open folder" actions call the Explorer plugin directly; `NSFileViewer` is set as a best-effort extra for cooperating apps and recorded in the claims ledger (`defaults read -g NSFileViewer`: absent means Finder). On 26.6+ do not attempt a `public.folder` binding at all. Restore of `NSFileViewer` is a plain `defaults delete`/write with no prompt. **I**

---

## 3. Slot takeovers

### 3.1 Symbolic hotkeys: the mechanism for Spotlight, screenshots, Cmd+Tab

Storage: `~/Library/Preferences/com.apple.symbolichotkeys.plist`, key `AppleSymbolicHotKeys`, one dict per numeric id, each with `enabled` (bool) and `value = {type = standard; parameters = (character, keyCode, modifierMask);}`. A missing id means enabled with the stock combo. **V-community** (Octoshot PR 54; fastlight PR 3)

Modifier masks here are NSEvent-style flags (Command 1048576, Shift 131072, Option 524288, Control 262144), and 65535 in the first two slots means "no key". **V-community**

| Id | Meaning | Stock params (char, keycode, mask) | Source |
|---|---|---|---|
| 64 | Show Spotlight search (Cmd+Space) | 32, 49, 1048576 | andyjakubowski mapping table; fastlight |
| 65 | Spotlight window / Finder search (Opt+Cmd+Space) | not listed | CGSInternal header names it `SpotlightWindow`; fastlight calls it "Finder search window" |
| 28 / 29 | Screenshot full screen to file / to clipboard (Cmd+Shift+3 / Ctrl+Cmd+Shift+3) | 51, 20, 1179648 / 1441792 | mapping table |
| 30 / 31 | Screenshot selection to file / clipboard (Cmd+Shift+4 / +Ctrl) | 52, 21, 1179648 / 1441792 | mapping table |
| 184 | Screenshot and recording options (Cmd+Shift+5) | 53, 23, 1179648 | mapping table; Octoshot tracks 28, 29, 30, 31, 184 |
| 181 / 182 | Touch Bar screenshots | 54, 22 | mapping table |
| 1 / 2 | Cmd+Tab / Cmd+Shift+Tab | not in plist tables I found | AltTab source (`CGSSymbolicHotKey.swift`) |
| 27 | Move focus to next window (Cmd+`) | 96, 50, 1048576 | mapping table; AltTab source |
| 60 / 61 | Select previous / next input source | 32, 49, 262144 for 60 | mapping table; CGSInternal |
| 79 to 82 | Move left/right a space | n/a | mapping table |
| 118 to 126 | Switch to Desktop 1 to 9 | Ctrl+digit | KiwiDesk PR 2317 (read on macOS 27.0.1) |

Sources:
- https://github.com/andyjakubowski/dotfiles/blob/main/AppleSymbolicHotKeys%20Mappings
- https://raw.githubusercontent.com/NUIKit/CGSInternal/master/CGSHotKeys.h
- https://github.com/neardws/Octoshot/pull/54
- https://github.com/elboletaire/fastlight/pull/3
- https://raw.githubusercontent.com/lwouis/alt-tab-macos/master/src/macos/api-wrappers/CGSSymbolicHotKey.swift
- https://github.com/KiwiCanopy/KiwiDesk/pull/2317

Ids are a private numbering, so each backend should discover live values by calling `CGSGetSymbolicHotKeyValue` over the first few hundred ids (AltTab's own source comment suggests exactly that) instead of trusting a static table. **V** (AltTab comment)

### 3.2 How to write it and make it apply

Two routes:

1. **Plist plus `activateSettings`** (all public except the helper).
   - Write the entry with CFPreferences or `defaults write com.apple.symbolichotkeys AppleSymbolicHotKeys -dict-add 64 '<dict>...</dict>'` with `enabled` false and the full `value` dict (a bare `enabled false` can leave a malformed entry, per the extended search note; use the stock `value`). **V-community** (Zameer Manji; Octoshot)
   - Apply: `/System/Library/PrivateFrameworks/SystemAdministration.framework/Resources/activateSettings -u`. Zameer Manji: without it, `defaults` edits to this domain "don't take effect right away", and the tool "reads all of the values in com.apple.symbolichotkeys and binds all of those shortcuts" (from disassembly). Octoshot (screenshots) and a dotfiles PR use it so changes apply "without requiring a restart or logout". **V-community**
   - Fastlight keeps a logout fallback: if the live state still differs after `activateSettings -u`, it tells the user to log out and back in. So "applies without logout" is the common case, not a guarantee. **V-community**
   - Octoshot avoids `killall cfprefsd`.
   - I found no per-release test of this on 13, 14, 15 or 26. Octoshot's PR lists no macOS versions; fastlight's real write path was not run on the author's machine. Treat all four releases as **I** and verify in a guest per version.
   - Sources: https://zameermanji.com/blog/2021/6/8/applying-com-apple-symbolichotkeys-changes-instantaneously/ and the PRs above.
2. **`CGSSetSymbolicHotKeyEnabled(id, bool)`** from SkyLight (private, resolved with `dlsym`), the call System Settings uses. AltTab uses it for ids 1 and 2 and its comment says "the effect of enabling/disabling persists after the app is quit". Fastlight reads live state with `CGSIsSymbolicHotKeyEnabled` and `CGSGetSymbolicHotKeyValue`. **V** (AltTab source), **V-community** (fastlight)
   - Persistence after quit means a crash or a kill leaves the system key disabled. qol's claims ledger must treat these as durable and re-enable on the next start or on uninstall. **I** (follows from the comment)

Protection: none found. No SIP, no TCC prompt, no admin right, no signature check on writes to the user's own preference domain. MDM can enforce `com.apple.symbolichotkeys` as a configuration profile (a Jamf thread describes disabling screenshot shortcuts that way), which would revert qol's change; qol can detect that by reading back after write. **V-community** (https://community.jamf.com/t5/jamf-pro/symbolic-keys/td-p/14581)

Read and "set by someone": a missing id or `enabled = 1` is stock. An entry with `enabled = 0` or a changed `parameters` means the user (or a tool) changed it. A value of `enabled = 0` with `(65535, 65535, 0)` is an unset key. **V-community** (extended search note, from a keyboard-shortcut guide). The Settings pane can disagree with the stored state after upgrades or restores, so read the plist, not the pane. **V-community**

Restore: back up the exact prior entry or the fact it was missing (Octoshot records "the existing entry or missing"; fastlight's restore is a byte-for-byte round trip and keeps the first backup on repeated disables). At restore, compare the current entry with what qol wrote; if different, the user changed it, so leave it. **V-community**

Scope: these ids drive the system's own handlers (Spotlight, `screencaptureui`, the Dock's switcher). They do not affect a Carbon hotkey registered by another app, but if an enabled system hotkey owns the combo, the system wins and the other app's registration never fires. Tauri's global-shortcut README: "system shortcuts take precedence" over `RegisterEventHotKey`. **V-community**

### 3.3 Spotlight, Cmd+Space

- Disable id 64 (and 65 if qol also wants Opt+Cmd+Space). The documented user route is System Settings, Keyboard, Keyboard Shortcuts, Spotlight, uncheck "Show Spotlight search". Disabling it does not stop indexing. **V-community** (WebNots / HowToIsolve, via search)
- Deep link to the pane: `x-apple.systempreferences:com.apple.Keyboard-Settings.extension` (macOS 13+). No stable link to the shortcuts sheet. **V-community** (fastlight PR 3)
- Tahoe 26: Spotlight gained an Applications view (Cmd+1) and a Clipboard view (Cmd+4, within Spotlight). The magnifier menu-bar icon moved to Settings, Menu Bar. **V-community** (MacMost; TechRadar via search)
- If qol takes Cmd+Space, the macOS clipboard view loses its entry key (Cmd+4 only works inside Spotlight). That is acceptable because qol has its own clipboard history. **I**

### 3.4 Screenshot keys

- Disable ids 28, 29, 30, 31 and 184 together. Octoshot's flow: back up, set `enabled=false` (creating stock entries if missing), write, `CFPreferencesAppSynchronize`, then `activateSettings -u`. **V-community** (PR 54)
- Fallback when it will not apply: deep link to Keyboard Shortcuts. **V-community**
- The Screenshot app (Cmd+Shift+5) stays reachable from Spotlight and the Launchpad-style app list; only the keys move. **I**

### 3.5 Cmd+Tab

- Disable ids 1 and 2 with `CGSSetSymbolicHotKeyEnabled` (AltTab's `setNativeCommandTabEnabled`). **V** (source: https://raw.githubusercontent.com/lwouis/alt-tab-macos/master/src/macos/api-wrappers/SkyLight.framework.swift)
- Cmd+` (id 27, "Move focus to next window") is separate: AltTab leaves it alone because "it is not consumed by any system process", but issue 2053 shows a conflict with Stage Manager on macOS 13 and later (reports on 13.0, 13.4, 14.7, 15.5) that was only fixed by disabling "Move focus to next window" in Keyboard Shortcuts. **V-community** (https://github.com/lwouis/alt-tab-macos/issues/2053, closed March 2026)
- Older advice that "there is no way to turn off the system Command-Tab" (Keyboard Maestro forum) predates this route; those tools use an event tap whose ordering decides the winner, sometimes losing to the system tap. **V-community** (https://forum.keyboardmaestro.com/t/the-application-switcher-hotkey-sometimes-triggers-the-os-x-app-switcher/5912)
- A QEMU commit notes that macOS handles Cmd+Tab "at an earlier part of the event handling chain" than normal key events, which is why an HID-level tap is needed to see it. **V-community** (search result, FRET-LibAFL mirror)

### 3.6 Event taps and permissions (Cmd+Tab and any key qol must see or eat)

| Tap kind | Permission | Source |
|---|---|---|
| `CGEventTapCreate` with `kCGEventTapOptionDefault` (can drop or edit events) | Accessibility | Apple forum thread 122492 (community poster); keyremap's tap uses this. **V-community** |
| `kCGEventTapOptionListenOnly` | Input Monitoring (`CGPreflightListenEventAccess`, `CGRequestListenEventAccess`) | same thread. **V-community** |
| Having Accessibility is reported to also satisfy Input Monitoring; a counter-report says the tap still worked after removing Input Monitoring from an app with Accessibility | inconsistent | same thread. **V-community**, behavior across versions **I** |
| Carbon `RegisterEventHotKey` | No permission for hotkeys that carry a modifier (AltTab's comment: the alternatives "require Accessibility Permission") | AltTab `KeyboardEvents.swift`. **V** |

AltTab, a mature macOS switcher, uses: Carbon `RegisterEventHotKey` for its shortcuts, a listen-only session tap for modifier/key ordering, and a default HID tap only while Esc must be absorbed (it keeps the active HID tap off during normal typing "to avoid breaking third-party input methods"). It also absorbs Cmd+Esc because of the macOS 26 Game Overlay. It re-enables taps on `tapDisabledByTimeout` and `tapDisabledByUserInput`. **V** (https://raw.githubusercontent.com/lwouis/alt-tab-macos/master/src/events/KeyboardEvents.swift)

Its permission checks: Accessibility through the private `TCCAccessCheckAuditToken` (because `AXIsProcessTrusted` "answers from an in-process cache"), with `AXIsProcessTrustedWithOptions` as fallback, and Screen Recording separately. It does not call `CGPreflightListenEventAccess`. **V** (https://raw.githubusercontent.com/lwouis/alt-tab-macos/master/src/macos/SystemPermissions.swift)

Hazards:
- Active tap plus revoked Accessibility: forum reports of system-wide input hanging or clicks blocked until reboot when Accessibility is revoked while a default tap runs, even with a pass-through callback. **V-community** (https://developer.apple.com/forums/thread/844416, https://developer.apple.com/forums/thread/735204)
- Secure Event Input: Apple TN2150 says event taps stop receiving keyboard events "whenever any process has enabled secure event input, whether that process is in the foreground or background". It is process-global and reference-counted; a leaked holder (1Password, Cursor reports on 15.7 and 26.5) disables every tap system-wide. `IsSecureEventInputEnabled()` detects it; `ioreg -l -w 0 | grep SecureInput` shows `kCGSSessionSecureInputPID`, which can wrongly name `loginwindow`. **V** (TN2150), **V-community** (1Password and Cursor threads)
- Carbon hotkeys reportedly still fire under secure input ("WindowServer dispatches" them), per one project's README. Apple's note does not mention Carbon hotkeys. **V-community** (https://github.com/jackowfish/shiftly), unverified by Apple.
- Design consequence: put Cmd+Tab-class slots on a Carbon hotkey after freeing the system id (route 3.5), and keep an event tap only for what Carbon cannot express (Fn, bare modifiers, Cmd+Esc). Show a doctor warning when secure input is held. **I**

Repo cross-check (read-only):
- `plugins/keyremap/src/platform/macos/tap.rs` creates default-option taps at `AnnotatedSession` and `HID` and `std::process::exit(1)` if creation fails. `libs/platform/src/permissions/platform/macos.rs` maps `Permission::InputCapture` to `AXIsProcessTrusted` only; there is no Input Monitoring (`CGPreflightListenEventAccess`) state, so a listen-only tap has no permission check, and a Carbon-hotkey-only path would need none. The plugin already has a `secure_input` module. Observed in the repo, not external.

### 3.7 Dictation shortcut

- The setting lives in System Settings, Keyboard, Dictation, Shortcut. Options reported: Fn/Globe twice, Ctrl twice, the F5 microphone key on 2021+ MacBooks, or a custom shortcut; guides disagree on the default. **V-community** (several vendor blogs, low quality)
- Apple's shortcuts page is reported to list Fn-D for dictation. I could not confirm the storage key. No source ties dictation to a symbolic hotkey id (I looked for 164 and found nothing). The idea that it is stored in `com.apple.assistant.support` is also unconfirmed. **I** (unknown)
- Globe/Fn key behavior is `com.apple.HIToolbox AppleFnUsageType`, with named choices Do Nothing, Change Input Source, Show Emoji & Symbols, Start Dictation. A restart is reported as needed. Numeric values not verified. **V-community** (nix-darwin option page, https://mynixos.com/nix-darwin/option/system.defaults.hitoolbox.AppleFnUsageType)
- Carbon cannot register the Fn/Globe key; tools such as Karabiner-Elements and BetterTouchTool reach it at HID level. **V-community** (Keyboard Maestro forum, search result)
- Design consequence: to take the dictation slot for qol-voice, the reliable route is "set Fn usage to Do Nothing" (needs verification of the numeric value and whether it applies live) plus a tap on the chosen key. Read-back of the previous value is `defaults read com.apple.HIToolbox AppleFnUsageType` (missing means the OS default). **I**

### 3.8 Window tiling keys on macOS 15 and later

- Sequoia added system tiling with keys Fn+Ctrl+arrows (halves), Fn+Ctrl+F (fill), Fn+Ctrl+C (center), with Shift variants for arrange. The toggles are under System Settings, Desktop & Dock, Windows. **V-community** (Computerworld, Macworld, via search)
- They behave like Window-menu key equivalents (ignored in apps without the standard Window menu, such as VectorStyler), not like symbolic hotkeys. I found no symbolic hotkey ids for them. **V-community** for the app behavior (vectorstyler.com forum), **I** for the mechanism.
- Edge-drag: `defaults write com.apple.WindowManager EnableTilingByEdgeDrag -bool false` (a blog source, not Apple). The Option-drag switch exists as a separate setting; its key name is unverified. **V-community** (Mainspring blog)
- Disabling the keyboard shortcuts: edge-drag toggles "don't affect keyboard shortcuts". The working workaround in Apple Community is to create two new entries under System Settings, Keyboard, Keyboard Shortcuts, All Applications, which then disable tiling. The likely mechanism is `NSUserKeyEquivalents` in `NSGlobalDomain` overriding menu titles. **V-community** (https://discussions.apple.com/thread/255802129), mechanism **I**.
- Known conflict: Fn+Ctrl+Left in Excel triggers tiling instead of navigation. **V-community** (same thread)
- A third-party snapper that uses other chords (Rectangle defaults to Ctrl+Opt+arrows) does not collide with the system keys; Rectangle detects macOS tiling on first install and offers to disable the system one. **V-community** (Rectangle pages, via search)
- Design consequence: Window Actions on macOS 15+ should either keep its own chords (no takeover needed) or take Fn+Ctrl+arrows with a tap and write the `NSUserKeyEquivalents` override; the second is unverified and should be tested in a guest first. **I**

### 3.9 Clipboard history in Spotlight on macOS 26

- Reached with Cmd+Space then Cmd+4, or a button in Spotlight's top right. Toggle: System Settings, Spotlight, "Clipboard" (a row at the bottom; one source calls it "Clipboard Search"). When off, the Clipboard button disappears from Spotlight. **V-community** (MacMost)
- Retention is reported two ways: MacMost says items older than 8 hours are removed, plain text only, text over about 16,384 characters not saved, Apple Passwords copies excluded; another summary lists 30 minutes, 8 hours or 7 days as choices. The sources conflict. **V-community**, **I** on which is right. (https://macmost.com/how-to-use-the-spotlight-clipboard-history-in-macos-tahoe.html)
- No scripting, API or `defaults` key found; the only documented off switch is the Settings toggle. **V-community** (MacMost)
- It has no key of its own, so there is nothing to take over; qol's clipboard plugin simply coexists, or the user turns the Spotlight toggle off. Before macOS 26 there is no host clipboard history at all. **V-community**

### 3.10 Menu-bar extras (Bluetooth, sound)

- A third-party status item is `NSStatusItem`. Its visibility "may change if the user removes the item manually", and it is restored by `autosaveName`. **V** (Apple `NSStatusItem.isVisible` doc, via search)
- Tahoe 26 added System Settings, Menu Bar, "Allow in the Menu Bar": a per-app list of third-party apps allowed to show items. A Parallels forum post says apps can end up disabled there without the user knowing; the fix is to enable them and relaunch. **V-community** (https://forum.parallels.com/threads/asking-how-to-keep-toolbox-icon-on-menu-bar.369594/). The API that triggers the entry is unverified. **I**
- Apple's own items (Wi-Fi, Bluetooth, Sound) are toggled in "Menu Bar Controls". Pre-Tahoe, Bluetooth was `ByHost/com.apple.controlcenter.plist` key `Bluetooth` (18 show, 24 hide), reported on Big Sur with the caveat that editing the plist from the command line does not force the change. Sound is historically in the `systemuiserver` domain and writes did not stick. **V-community** (Jamf and MacScripter threads, old), so unverified on 13 to 26. **I**
- Design consequence: qol's menu-bar applet can be added freely on macOS 13 to 15; on 26 expect it to be off until the user allows it, and detect "not allowed" in doctor. Hiding Apple's own Bluetooth or Sound item has no verified automation; leave it to the user.

---

## 4. Version spread (macOS 13 to 26)

| Version | Notable for this design |
|---|---|
| 13 Ventura | `NSWorkspace` setters available (macOS 12+). `http` prompts. Keyboard Settings deep link `com.apple.Keyboard-Settings.extension`. AltTab issue 2053: native Cmd+` handling conflicts with Stage Manager. |
| 14 Sonoma | Same API contract. No specific Launch Services change found. AltTab 2053 still reported on 14.7. |
| 15 Sequoia | System tiling keys (Fn+Ctrl) and edge-drag. `lsregister -u` fails with -10814. |
| 26 Tahoe | Spotlight Applications and Clipboard views. Menu Bar "Allow in the Menu Bar". Game Overlay takes Cmd+Esc. `lsregister -kill` removed. 26.4: all UTI default changes prompt, plus a second Finder sheet when HTML is set. 26.6.2: `public.folder` bind fails validation. `tel:` default picker removed. |

macOS 27 appears in sources only as a plist format check (fastlight: the symbolic hotkey format "was confirmed read-only on macOS 27"). Out of scope.

---

## 5. Claims I could not verify (do not build on these)

- Exact error domain and code when the user answers "Keep".
- Whether `mailto` and other non-`http` schemes prompt on any release; I only found "http always prompts", plus "26.4 adds UTIs".
- `activateSettings -u` on 13, 14, 15 and 26 specifically; no per-release test found.
- Storage key and symbolic hotkey id for the dictation shortcut; numeric values of `AppleFnUsageType`.
- Symbolic hotkey ids or keys for the Sequoia tiling shortcuts.
- That `NSWorkspace activateFileViewerSelectingURLs` ignores `NSFileViewer` (I found no source either way).
- Tie-break among equally ranked handlers in Launch Services.
- Code-signing or notarization requirement for being accepted as a default handler.
- Clipboard history retention values.

## 6. Source list (all fetched or surfaced in this session)

- Apple NSWorkspace setters: https://developer.apple.com/documentation/appkit/nsworkspace/setdefaultapplication(at:toopenurlswithscheme:completion:).md and https://developer.apple.com/documentation/appkit/nsworkspace/setdefaultapplication(at:toopen:completion:).md
- Apple Info.plist keys: https://developer.apple.com/library/archive/documentation/General/Reference/InfoPlistKeyReference/Articles/CoreFoundationKeys.html
- Apple TN2150 secure event input: https://developer.apple.com/library/mac/technotes/tn2150/_index.html
- Apple DTS threads: https://developer.apple.com/forums/thread/741305, /769441, /800777, /719043, /122492, /844416, /735204, /735223
- Chromium: https://chromium.googlesource.com/chromium/src/+/main/chrome/browser/shell_integration_mac.mm
- Scripting OS (26.4 prompts): https://scriptingosx.com/2026/03/macos-26-4-brings-more-default-app-confirmation-prompts/
- utiluti: https://github.com/scriptingosx/utiluti ; dooti: https://www.github.com/lkubb/dooti ; openwith: https://github.com/hza/openwith
- Ghostty discussion: https://github.com/ghostty-org/ghostty/discussions/8111 ; SonicTerm plan: https://github.com/D0n9X1n/SonicTerm/issues/1654
- LS plist Stack Overflow: https://windows-hexerror.linestarve.com/q/so56929742-adding-and-deleting-lshandlerurlscheme-from-comapplelaunchservicessecureplist-is-not-immediately-ref
- Eclectic Light: https://eclecticlight.co/2025/03/27/controlling-launchservices-in-macos-sequoia/ and https://eclecticlight.co/2026/08/04/fix-launchservices-problems-with-command-tools/
- Folders: https://stclairsoft.com/blog/?p=1182 , https://www.stclairsoft.com/blog/?p=2000 , https://talk.macpowerusers.com/t/forklift-3-default-file-viewer-with-mac-os-catalina-10-15-7/33989 , https://blog.binarynights.com/2026/08/18/forklift-4-7-4-is-available-fixes-for-file-transfers-and-tags/
- Symbolic hotkeys: https://zameermanji.com/blog/2021/6/8/applying-com-apple-symbolichotkeys-changes-instantaneously/ , https://github.com/neardws/Octoshot/pull/54 , https://github.com/elboletaire/fastlight/pull/3 , https://github.com/andyjakubowski/dotfiles/blob/main/AppleSymbolicHotKeys%20Mappings , https://raw.githubusercontent.com/NUIKit/CGSInternal/master/CGSHotKeys.h , https://github.com/KiwiCanopy/KiwiDesk/pull/2317 , https://community.jamf.com/t5/jamf-pro/symbolic-keys/td-p/14581
- AltTab source: https://raw.githubusercontent.com/lwouis/alt-tab-macos/master/src/events/KeyboardEvents.swift , .../src/macos/api-wrappers/CGSSymbolicHotKey.swift , .../src/macos/api-wrappers/SkyLight.framework.swift , .../src/macos/SystemPermissions.swift , issue https://github.com/lwouis/alt-tab-macos/issues/2053
- Secure input: https://www.1password.community/1password-at-work-58/secure-input-blocking-other-apps-event-taps-25015 , https://github.com/jackowfish/shiftly , https://forum.cursor.com/t/cursor-leaks-secure-event-input-breaks-all-system-wide-input-taps-until-quit-or-screen-lock/167585
- Tiling: https://discussions.apple.com/thread/255802129 , https://trymainspring.com/blog/window-tiling-mac , https://www.computerworld.com/article/3550352/how-to-use-window-tiling-with-macos-sequoia.html
- Tahoe features: https://macmost.com/how-to-use-the-spotlight-clipboard-history-in-macos-tahoe.html , https://forum.parallels.com/threads/asking-how-to-keep-toolbox-icon-on-menu-bar.369594/
- Dictation Fn usage: https://mynixos.com/nix-darwin/option/system.defaults.hitoolbox.AppleFnUsageType
