# Windows research: qol as the host's default apps

Scope: Windows 10 (22H2, 19045) and Windows 11 (22H2, 23H2, 24H2, 25H2 = build 26200, 26H2 = build 26300), Home / Pro / Enterprise. Researched 2026-10-24.

Tags: [V] verified (official docs, source code, or a reproducible report), [R] reverse-engineering write-up or vendor source docs, [I] inferred (my reasoning or unsourced community claim). Two primary sources were unreachable (kolbi.cz returned 403, web.archive.org is blocked), so UserChoiceLatest and the 2025 UCPD list rest on secondary reports and are tagged accordingly.

## 0. Bottom line for the design

1. On Windows 10 1703+ and Windows 11 there is no supported way for an app to silently become the default for a file type or URL scheme. Microsoft says apps cannot change defaults, only the user can [V]. The supported path is: register the app (ProgID + Capabilities + RegisteredApplications under HKCU), then open Settings on the app's own page with `ms-settings:defaultapps?registeredAppUser=<name>`, where the user clicks "Set default".
2. Writing `UserChoice` / `UserChoiceLatest` directly is a hash-forgery. It is blocked by UCPD.sys for http, https, .pdf (and per some reports more) on every current build, it breaks every few years when the hash changes, and Microsoft's own principles post treats it as an "unrequested modification" to mitigate. Do not build qol on it. Do read it.
3. Folder handling is not a "default app" at all on Windows. It is a shell-verb override (Directory / Drive / Folder), with known bypasses (Win+E, SHOpenFolderAndSelectItems, `explorer.exe /select`, Control Panel, Desktop, file dialogs).
4. Slot takeovers split in three: free (Print Screen via a policy-free toggle), hook-only (Win+V, Win+H, Win+Shift+S, Alt+Tab, bare Win key, Win+Arrow), and impossible (Ctrl+Alt+Del, Win+L is also not interceptable [I]). `RegisterHotKey` is the right first tool but the OS reserves Win+key combinations, so a `WH_KEYBOARD_LL` fallback is required for most slots.
5. Restore: for handler defaults the only safe restore is "do nothing to the user's choice", because qol never wrote it. For slots, qol-owned claims (a registry toggle, a hook) are reversible and each has a detectable "user changed it since" signal (section 9).

## 1. UserChoice and UserChoiceLatest

### 1.1 Where defaults live

| Kind | Key (HKCU) | Notes |
|---|---|---|
| File type | `Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\<.ext>\UserChoice` | values `ProgId`, `Hash` [V: Mozilla docs] |
| URL scheme | `Software\Microsoft\Windows\Shell\Associations\UrlAssociations\<scheme>\UserChoice` | values `ProgId`, `Hash` [V] |
| Newer parallel key | same parents, subkey `UserChoiceLatest` | `ProgId`, `Hash`; Windows 11 24H2/25H2 Settings writes here [R, see 1.3] |
| Candidates | `HKCU\Software\RegisteredApplications`, `HKLM\Software\RegisteredApplications` | value name -> Capabilities path [V: Win32 docs] |

Sources: https://firefox-source-docs.mozilla.org/widget/windows/shell/set-default.html , https://learn.microsoft.com/en-us/windows/win32/shell/default-programs

### 1.2 Legacy UserChoice hash (Windows 8 to 10, still read on Windows 10 22H2)

Inputs [V: Firefox source `WindowsUserChoice.cpp`, which Mozilla ships]: lowercase concatenation of extension or protocol, user SID, ProgId, the last-write FILETIME of the UserChoice key with seconds and milliseconds zeroed (two DWORDs as `%08lx`), and a fixed string "User Experience ..." (the source says it "might vary across Windows SKUs"). MD5 over UTF-16, then a custom two-accumulator scramble ("patent hash", US 6,570,988 B1), then Base64. Format noted by Mozilla as current "as of Windows 10 20H2", in use since at least 1803, changed around 1703.
Source: https://raw.githubusercontent.com/mozilla/gecko-dev/master/browser/components/shell/WindowsUserChoice.cpp

Consequences:
- The hash binds the ProgId to the key's last-write time. Any write by Settings changes the time, so a snapshot of (ProgId, Hash, last-write time) detects "the user changed it" without computing anything [I, follows directly from the algorithm].
- Windows ignores a ProgId whose Hash does not match and falls back to the default handler [V: BleepingComputer, Kolbicz summary].
- Mozilla notes file-handler UserChoice keys have Set Value denied to the owning user, and Firefox deletes and recreates the key to work around it [V].

### 1.3 UserChoiceLatest (Windows 11 24H2 and later)

What is established:
- Windows 11 25H2 Settings records the https default in `UrlAssociations\https\UserChoiceLatest\ProgId`; the legacy `UserChoice` key stops being updated and may hold a stale browser or exist with no value. A public project fixed real breakage by reading `AssocQueryStringW(ASSOCSTR_PROGID, "https")` first, then `UserChoiceLatest\ProgId`, then legacy `UserChoice`; `ASSOCSTR_EXECUTABLE` failed with `0x80070483` on 25H2 [R: hermes-agent PR #134428, with a live Windows runner test]. https://github.com/NousResearch/hermes-agent/pull/134428
- Kolbicz's April 2025 post (title "UserChoiceLatest: Microsoft's new protection for file type associations", not fetched, seen only through a search summary) reports a migration that computes a new hash for the ProgId into `UserChoiceLatest\Hash`; once migrated Windows ignores the old `UserChoice` keys; the new hash includes user ID, a timestamp, association details and the machine ID; he reimplemented it in SetUserFTA. [R, secondary] https://kolbi.cz/blog/2025/04/20/userchoicelatest-microsofts-new-protection-for-file-type-associations/
- A PS-SFTA fork README (single third-party source, [I] until reproduced) says: feature name "AppDefaultHashRotation", GA on 24H2/25H2; the per-user flag is `HashVersion` = 1 under `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\SystemProtectedUserData\<SID>\AnyoneRead\AppDefaults`, and once set Explorer reads only `UserChoiceLatest`; hash input adds machine ID and one of three Microsoft salt strings read from `windows.storage.dll` and `shell32.dll`; a "v2" salt set ships unused (possible future rotation). Its test table: 25H2 (26200) and 26H2 (26300) honor `UserChoiceLatest`; on 26H2 with UCPD on, `.pdf`, `http`, `https` writes are refused; Win 10 22H2 uses legacy `UserChoice`. https://github.com/ComoEstaisAmigos/PS-SFTA

Design rule: qol READS defaults through the shell API (`AssocQueryStringW`, or `IApplicationAssociationRegistration::QueryCurrentDefault`) and never parses the hash keys, except for the snapshot-compare used to detect user change. Order of truth for a direct read: `UserChoiceLatest` if present, else `UserChoice` [R].

### 1.4 Unset vs set by someone

- "Unset / system default": no `UserChoice*` subkey. Shell resolves through `HKCU\Software\Classes` then `HKLM\Software\Classes` default value (merged HKCR view), then OEM / DISM defaults [V: Microsoft DISM docs, Win32 docs].
- "Set by someone": a `UserChoice*` subkey exists. A key with a broken hash is treated as unset by the shell but still present on disk [V: BleepingComputer].
- Tip 3 in the Microsoft DISM doc: if exactly one app can handle a type, it shows as default with no registry choice recorded [V]. So "unset" can still display an app.

## 2. UCPD.sys (UserChoice Protection Driver)

### 2.1 What it is and when it appeared

- Filter driver `C:\Windows\System32\drivers\UCPD.sys`, registry-callback based (`CmRegisterCallbackEx`). Shipped with the February 2024 cumulative updates: Windows 10 KB5034763, Windows 11 KB5034765. Cannot be unloaded, service can be disabled. [V: BleepingComputer https://www.bleepingcomputer.com/news/microsoft/new-windows-driver-blocks-software-from-changing-default-web-browser/ ; R: Binary Ninja https://binary.ninja/2025/03/25/default-browser-upcd.html]
- Microsoft has not published a reference. Everything below is reverse engineering or vendor behavior notes.

### 2.2 Which keys

| Period | Protected | Source |
|---|---|---|
| Feb 2024 launch | `http\UserChoice`, `https\UserChoice`, `.pdf\UserChoice` | [V] BleepingComputer |
| Mozilla docs (2025/26) | "http(s) and .pdf; htm(l) is excluded"; originally only `UserChoice` was locked so renaming an ancestor bypassed it; "all ancestors are now locked" | [V] Mozilla source docs |
| Kolbicz 2025 (secondary summary) | UserChoice and UserChoiceLatest for http, https, .pdf, .htm, .html, .doc(x), .xls(x), .ppt(x); list varies by region and edition | [R, unverified, kolbi.cz 403] |
| PS-SFTA README tests (25H2/26H2) | blocked: http, https, .pdf; NOT blocked: ftp, .htm, .html, .svg, .xml, .webp, ordinary extensions | [I] one third-party repo |

The sources conflict on .htm/.html, and Kolbicz explicitly warns the list can change "on the fly". Treat the protected set as: always http, https, .pdf; maybe more; region and SKU dependent. qol must not depend on any of it, which is the argument for the Settings flow.

### 2.3 Which processes it blocks

Registry callback allows an operation on a protected key only if the caller passes both checks [R: Binary Ninja 2025, UCPD 3.1]:
1. The executable is signed by Microsoft.
2. The executable is not on the utility denylist.

Denylist, from the 2026 Binary Ninja post (14 entries, `*\name` patterns, same list hardcoded in the driver): dllhost.exe, reg.exe, rundll32.exe, powershell.exe, regedit.exe, wscript.exe, cscript.exe, cmd.exe, InfDefaultInstall.exe, pwsh.exe, wmiprvse.exe, regini.exe, bssafe.exe, mshta.exe. [R] https://binary.ninja/2026/08/04/ucpd-dynamic-rules.html

Net effect: a non-Microsoft-signed app (qol, SetUserFTA, Firefox's agent) is denied; Settings, Explorer and other Microsoft-signed non-denylisted binaries are allowed. Blocked writes return access denied; regedit shows "Cannot edit Hash: Error writing the value's new contents" [V].

Anti-injection (UCPD 3.1, feature bit `0x100`, [R]): process-notify and object callbacks strip `PROCESS_VM_OPERATION|WRITE` / `THREAD_SET_CONTEXT` from handles that certain Chinese security-suite processes (Kingsoft, Qihu 360, Zhuhai Juntian) request to `explorer.exe` / `msedge.exe`, and `DESKTOP_JOURNALPLAYBACK` from Opera processes. This targets known default-hijackers and is irrelevant to qol unless qol injects into explorer.exe, which it must not.

### 2.4 How it changed, 2024 to 2026

| When | Change | Source |
|---|---|---|
| Feb 2024 | Driver ships, http/https/.pdf, only the `UserChoice` leaf path locked | [V] |
| 2024 | Early gap: scriptable Microsoft utilities (reg.exe etc.) still wrote; then the denylist was added | [V] Mozilla docs ("early releases missed some scriptable Microsoft utilities") |
| 2024 to 2025 | Path-based locking widened to all ancestors (rename bypass closed) | [V] Mozilla docs |
| Mar 2025 (UCPD 4.0, feature bit `0x400` BACKTRACE) | For writes by `SystemSettings.exe`, UCPD walks the call stack (`RtlWalkFrameChain`); a frame whose module is signed by one of ~16 blocked vendors rejects the write. Image-load tracking added | [R] Binary Ninja 2025 |
| Apr 2025 onward | `UserChoiceLatest` hash rotation on Windows 11 24H2/25H2 | [R] Kolbicz, [I] PS-SFTA |
| Jul 2025 | Kolbicz "part 2" post (not readable) | unverified |
| 2026-08 | `HKLM\SYSTEM\CurrentControlSet\Services\UCPD\DR`: a Base64 PE containing a Microsoft-signed, encrypted rule blob (handler types AntiInjection, UIA, DenyListV1, AllowListV1, StackTrace) so Microsoft can ship new rules without a new driver or reboot. The shipped blob equals the built-in list today. Author speculates it may be pushed regionally (inferred) | [R] Binary Ninja 2026 |
| Nov 2024 to mid 2025 | EEA DMA changes: Settings "Set default" for a browser now also sets more types (ftp, read, .htm, .html, .svg, .xml, .pdf in EEA); Edge prompts reduced; Bing and Start experiences honor the default browser in EEA | [V, news] see section 5 |

Feature flags live at `HKLM\SYSTEM\ControlSet001\Services\UCPD\FeatureV2` (Binary Ninja spells it "Srevices", a typo), written by `UCPDMgr.exe SetUCPDStatus`. Bits: 0x8 WSB, 0x10 TASKBAR, 0x80 ANTIUIA, 0x100 anti-injection, 0x200 NEWDENYLIST, 0x400 BACKTRACE, 0x800 RENAME_ATTACK. [R]

### 2.5 Disabling it (do NOT make qol do this)

Set `HKLM\SYSTEM\CurrentControlSet\Services\UCPD\Start` = 4 and disable the scheduled task `\Microsoft\Windows\AppxDeploymentClient\UCPD velocity`, which otherwise re-enables the service; both need admin and a reboot. Updates may re-enable it. [V: BleepingComputer; R: ghacks, elevenforum guides]. Disabling a security protection from a QoL tool contradicts the "hand it back exactly" goal and Microsoft's principles post. Recommend against; document only so the doctor can explain failures of third-party tools.

### 2.6 Version / edition spread

| Build | UCPD present | Notes |
|---|---|---|
| Win 10 22H2 with Feb 2024+ CU | yes | BleepingComputer: locked keys on Win 10; legacy `UserChoice` |
| Win 11 22H2/23H2 with Feb 2024+ CU | yes | BleepingComputer saw the driver on 11 but the keys locked only on 10 in their test; later builds lock on 11 [V/R] |
| Win 11 24H2, 25H2 (26200) | yes | `UserChoiceLatest` [R] |
| Win 11 26H2 (26300) | yes | blocks .pdf/http/https; works when UCPD off [I] |
| Home / Pro / Enterprise | same driver | no edition gate found; Kolbicz says list varies by region and edition [R, unverified] |

## 3. IApplicationAssociationRegistration / IApplicationActivationManager

### IApplicationAssociationRegistration (Default Programs API)

- Whole API set "deprecated in Windows 8"; docs for Default Programs and "Managing Default Applications" carry "does not apply for Windows 10" banners. As of Windows 8 "applications themselves can no longer query and change defaults. Only the user can do that" and apps "should not attempt to query for the current default or change that default through any mechanism". [V] https://learn.microsoft.com/en-us/windows/win32/shell/vista-managing-defaults
- `SetAppAsDefault`: "Not intended for use in Windows 8"; sets the default only if the app's publisher matches the current default's, otherwise `E_ACCESSDENIED`. `SetAppAsDefaultAll` and `ClearUserAssociations` same fate. [V] https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-iapplicationassociationregistration-setappasdefault
- Reading still works in practice: `AssocQueryStringW(ASSOCSTR_PROGID, "https")` returned the right handler on 25H2 [R: hermes PR]; `IApplicationAssociationRegistration::QueryCurrentDefault` is the COM equivalent [I]. Docs discourage reading, so treat as best-effort and keep the registry snapshot as a cross-check.
- `IApplicationAssociationRegistrationUI::LaunchAdvancedAssociationUI(appRegisteredName)`: still the documented way to open the per-app defaults UI ("Set associations for a program") [V: Win32 docs]. On Windows 10/11 it lands in the Control Panel path or redirects to Settings [I]; the ms-settings deep link in section 4 is the modern equivalent.
- Registration side (still required for the app to be listed): Capabilities + RegisteredApplications, section 6.

### IApplicationActivationManager

Launches packaged (UWP/MSIX) apps by AUMID (`ActivateApplication`, `ActivateForFile`, `ActivateForProtocol`). It is an activation API, not a default-setting API [I, from API purpose; no doc fetched]. Relevance to qol: when qol is an MSIX/packaged app, `uap:Protocol` and `uap:FileTypeAssociation` in the manifest are the registration path and `registeredAUMID` is the deep-link parameter (section 4). qol is unpackaged today, so use `registeredAppUser`. When qol needs to open the previous default handler (Portable hand-back for a packaged handler such as Photos), `ActivateForFile` is the correct call; for unpackaged handlers use `ShellExecuteEx` with the `OpenWith` machinery, never the previous app's EXE path.

## 4. ms-settings:defaultapps deep links

Source (updated 2026-05-29): https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-default-apps-settings [V]

| Form | Meaning |
|---|---|
| `ms-settings:defaultapps` | opens Settings > Apps > Default apps |
| `ms-settings:defaultapps?registeredAppUser=<escaped name>` | app page for an app whose Capabilities are listed under `HKCU\Software\RegisteredApplications` (value NAME, URI-escaped) |
| `ms-settings:defaultapps?registeredAppMachine=<escaped name>` | same, for `HKLM\Software\RegisteredApplications` |
| `ms-settings:defaultapps?registeredAUMID=<AUMID>` | packaged app with `uap:FileTypeAssociation` / `uap:Protocol`; may need `MaxVersionTested` bumped (10.0.22000.1817 for 21H2, 10.0.22621.1555 for 22H2) after an OS upgrade to be reindexed |

Availability: Windows 11 21H2 and 22H2 with the 2023-04 cumulative update, and 23H2 or later [V]. First appeared in Insider build 23419 [V: news]. Not available on Windows 10 (use the plain `ms-settings:defaultapps` and the user picks by hand [I]).

Launch from Rust: `ShellExecuteW("open", "ms-settings:defaultapps?registeredAppUser=...")` or `Launcher.LaunchUriAsync`; no elevation. The name must match the RegisteredApplications value exactly. Windows 11 shows a "Set default" button on the app page that assigns all types and schemes the app registered; Windows 10 shows per-type choosers [V: Mozilla docs, browser guides].

No callback tells qol the user finished. Poll `AssocQueryStringW` / `UserChoiceLatest` when qol regains focus or on a short timer [I].

## 5. Microsoft's 2023+ guidance and "one-click set default"

- Principles post, 2023-03-17 [V] https://blogs.windows.com/windowsexperience/2023/03/17/a-principled-approach-to-app-pinning-and-app-defaults-in-windows/ : users stay in control of defaults and pins through consistent, clear Windows dialogs; developers get a supported common way to request a default or a pin; Microsoft's own apps use the same methods; Microsoft will keep mitigating "unrequested modifications" (this is the stated rationale for UCPD). Defaults: a Settings deep link only, the user makes the change. Pins: a public taskbar-pin API (`TaskbarManager.RequestPinCurrentAppAsync` / secondary tile pin) that always shows a trusted confirmation. The post describes no one-click silent set-default.
- I found NO shipped API in 2025 or 2026 that lets a third party set a default with a single trusted prompt. [I: absence of evidence after two searches]. What users call one-click is the Windows 11 "Set default" button inside Settings after your deep link lands on the app page (browsers: sets http, https, .htm, .html and, since the EEA changes, more types) [V].
- EEA / DMA changes (Insider 26120.4151, retail from about July 2025) [V, news: Thurrott, Pure Infotech, TechRepublic]: the browser "Set default" button covers ftp, read, .htm, .html, .svg, .xml and .pdf options; the chosen browser is pinned to taskbar unless opted out; Edge prompts reduced; Bing and Start experiences use the default browser. Outside the EEA behavior is the narrower set. https://www.thurrott.com/windows/windows-10/321697/microsoft-makes-more-dma-based-changes-to-windows-10-11-in-the-eu
- Mozilla's flow, current practice [V]: try silent UserChoice via `nsIDefaultAgent`; if it fails fall back to the Settings path (Windows 11 "Set default" button), and fire a `deeplinkedToWindowsSettingsUI` message trigger. Mozilla says UserChoice "is no longer viable in modern releases of Windows 11". https://firefox-source-docs.mozilla.org/widget/windows/shell/set-default.html
- Chromium, same: Make default leads to Settings (a Gerrit change for one-click was proposed, status unverified) [R, news].

Conclusion for qol: the user-visible cost is exactly one Settings page and one click per handler group. Make that the design, show a clear in-app step ("Click Set default for qol Explorer"), then verify by reading.

## 6. DISM and GPO default associations

### Mechanisms

| Mechanism | Scope | Edition | Admin | Behavior |
|---|---|---|---|---|
| `Dism /Online /Import-DefaultAppAssociations:file.xml` | machine-wide "default" layer for NEW user profiles (first sign-in) and re-applied on feature upgrade per `ApplyOnUpgrade` / `overwriteIfProgIdIs` | Home, Pro, Enterprise (DISM runs on all; Home not explicitly documented) [I for Home] | yes | Only the last imported file counts, imports are not cumulative. Silently ignored for users who already chose. Missing entries trigger "An app default was reset" notifications [V] |
| GPO "Set a default associations configuration file" (`Software\Policies\Microsoft\Windows\System\DefaultAssociationsConfiguration`, ADMX `WindowsExplorer.admx`, path Windows Components > File Explorer) | machine policy, applied at EACH sign-in | Pro, Enterprise, Education (CSP table lists Pro, Enterprise, Education, IoT; NOT Home) | yes | overwrites the user's association at every sign-in; on 22H2+ `Version` attribute and `Suggested="true"` on an `Association` apply it once per version bump instead of every sign-in [V] |
| MDM `./Device/Vendor/MSFT/Policy/Config/ApplicationDefaults/DefaultAssociationsConfiguration` | device | Pro, Enterprise, Education | Intune | MDM wins over GP even without MDMWinsOverGP [V] |

Sources: https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/export-or-import-default-application-associations , https://learn.microsoft.com/en-us/windows/client-management/mdm/policy-csp-applicationdefaults

Notes:
- XML: `<Association Identifier=".htm|http" ProgId="..." ApplicationName="..."/>`. ProgIds for packaged apps are generated hashes, so XML for packaged handlers must be exported from a reference machine [V].
- These mechanisms DO get past UCPD because the shell itself applies them, which is why the Microsoft forums recommend them for managed fleets [R].
- qol should never write the policy keys: they need admin, are overwritten at every sign-in for non-Suggested entries (so qol could fight the admin), and on Home do nothing. Doctor should DETECT a DefaultAssociationsConfiguration policy and warn that it can revert qol as default at next sign-in.
- Windows 11 22H2+ "Suggested" semantic is the closest official "set once, then let the user change it" primitive, but only reachable via admin policy.

## 7. ProgID + Capabilities + RegisteredApplications under HKCU

Documented registration (apply under HKCU for per-user, no admin; the doc's example uses HKLM but states `%ApplicationCapabilityPath%` may be under either hive) [V] https://learn.microsoft.com/en-us/windows/win32/shell/default-programs

Required layout for a handler app, e.g. a folder/file manager "qol Explorer":

```
HKCU\Software\Classes\qol.Explorer.Folder          (Default) = "qol Explorer folder"
    DefaultIcon, shell\open\command = "C:\...\qol-tray.exe" explorer open "%1"
HKCU\Software\qol-tools\Explorer\Capabilities
    ApplicationName = qol Explorer     (must equal the RegisteredApplications value name)
    ApplicationDescription = ...       (REQUIRED; without it the app is not listed)
    FileAssociations\.ext = qol.Explorer.File
    UrlAssociations\<scheme> = qol.Explorer.Url
HKCU\Software\RegisteredApplications
    qol Explorer = Software\qol-tools\Explorer\Capabilities
```

Rules:
- Each ProgID must be application-specific, not the generic `.ext` class [V].
- After writing, call `SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_DWORD|SHCNF_FLUSH, NULL, NULL)` then wait about a second; for browsers Windows then raises a "new default browser available" notification only if http and https were both newly registered and the user was never notified [V].
- Optional: `Capabilities\Hidden=1` hides the entry from the classic UI [V]. Not used.
- `OpenWithProgids` under `HKCU\Software\Classes\.ext` adds qol to the Open With list [V].
- Per-user is mandatory in practice: Windows 8+ ignores per-computer defaults and machine-level attempts after install "will be unsuccessful" [V].
- The `registeredAppUser` deep link name must match the RegisteredApplications value name exactly.
- Register at enable, unregister at disable/uninstall (Resident) or at tray exit (Portable); unregistering also removes qol from the chooser, and Windows drops an orphaned UserChoice whose ProgID vanished (the key becomes dangling; the shell falls back to the next default) [I]. This is the cleanest "hand back" and it needs no hash write.
- Must the target exist? The EXE referenced in `shell\open\command` must exist and be a valid file (unsigned works for per-user; signature only affects the publisher URL shown in classic UI) [V].

## 8. Folder handling

### 8.1 Mechanism

Windows has no "default file manager" setting. Opus staff: "Windows doesn't really have the concept of default File Manager" [R: https://resource.dopus.com/t/replace-explorer-in-all-calls/14843 ]. Replacement works by overriding the default verb on shell classes:

| Class | Covers |
|---|---|
| `HKCU\Software\Classes\Directory\shell` (Default value = verb name, plus `<verb>\command`) | real directories |
| `HKCU\Software\Classes\Drive\shell` | drive roots (C:\, D:\) |
| `HKCU\Software\Classes\Folder\shell` | all shell folders including virtual ones (Explorer ships `open` here with `DelegateExecute`) |

Community practice (many guides, FileExplorerInterceptor README, XYplorer forum) [R/I]: add a verb under `Directory\shell` and `Drive\shell`, set it as the class default value, and have it launch the replacement. A forum report says Win+E can be redirected by renaming the `DelegateExecute` value and pointing the key at your EXE [I: anecdotal]. HKCU `Software\Classes` overrides HKLM, so no admin [I: standard merged HKCR rules]. Microsoft documents none of this as a supported default-file-manager contract [absence confirmed by Opus staff].

Files (files-community) ships a "Set Files as the default file manager" toggle since 2.0.33 that writes this class of keys and reverts on uninstall [R, news; exact keys not found, read the repo before copying]. Directory Opus Pro calls it "Explorer Replacement". Its manual states exactly what is NOT replaced [R: https://github.com/Chaoses-Ib/DirectoryOpus/blob/main/Manual/basic_concepts/explorer_replacement.md ]:
- Control Panel always opens in Explorer. Desktop is implemented by explorer.exe. Standard File Open/Save dialogs unchanged. "Programs that call Explorer directly may not be intercepted, though often they are." Needs registry changes (so unavailable in portable exports); Pro only.

### 8.2 Call paths

| Call path | Obeys verb override? | Source |
|---|---|---|
| `ShellExecuteEx(folder)` with the NULL (default) verb | yes | Opus staff recommend NULL not "open" [R] |
| `ShellExecuteEx(folder, "open")` explicit | depends on whether the class default is "open"; may bypass | [R] |
| Double-click folder in a third-party app's own list | yes if via ShellExecute | [I] |
| `SHOpenFolderAndSelectItems` | honors the registry default verb but then performs an Explorer-only selection protocol; selection works only if the replacement handles it; XYplorer: "hard-coded to work with Explorer only"; Opus: "works with Opus" but harder to intercept; Q-Dir opens but does not select | [R] XYplorer https://www.xyplorer.com/xyfc/viewtopic.php?p=214999 |
| `explorer.exe /select,<path>` and `explorer.exe <path>` spawned by apps | NO, runs real Explorer | [R]; FileExplorerInterceptor says the WinEvent-hook approach "still replaces it" because it closes the new window after creation |
| Win+E | on Windows 10/11 handled by a separate mechanism from verb override; Opus has a dedicated "Launching Opus from the Win+E hotkey" setting; reports that Win+E opens Explorer when the registry association is broken | [R] Opus docs and forum |
| Taskbar pinned File Explorer icon, Start menu "File Explorer", "Open in Explorer" context entry | pinned icon launches `explorer.exe`; always Explorer | [R] Opus manual lists these as ways to reach Explorer |
| "Show in folder" from browsers (Chromium uses `SHOpenFolderAndSelectItems`, falls back to ShellExecute if the export is missing and loses selection) | partial; Opus forum: opens Opus then a second Explorer window about 15 s later in some programs | [R] Chromium source, Opus forum |
| This PC, Recycle Bin, Control Panel | Explorer (virtual folders) | [R] Opus users report Recycle Bin not taken over in "all filesystem folders" mode |

Fallback technique with a downside: a `SetWinEventHook(EVENT_OBJECT_CREATE)` watcher that reads `Shell.Application` windows and closes new Explorer windows, then launches the replacement with the path and optional selected items (FileExplorerInterceptor). Brief window flash; a folder already open is focused rather than created so nothing fires; hooking CreateWindow inside explorer.exe crashed it [R]. Do not inject into explorer.exe.

### 8.3 Design implication for the Explorer plugin

- Primary: register Directory + Drive verbs under HKCU with a qol-owned verb name (for example `qol.open`) and set the class default value to it. Record the PREVIOUS default value (absent vs set) in the claim ledger, and restore exactly that.
- For "reveal in folder": the only reliable hook is to handle `SHOpenFolderAndSelectItems` consumers' behavior indirectly; qol cannot make arbitrary callers go through it. State this limit in the UI.
- Win+E: separate slot, handled as a hotkey (section 10), not a handler default.
- Detect user change: compare the class default value now vs the value qol wrote; if different, drop the claim (a file manager installer or the user changed it).
- Windows 11 caveat: the Explorer "Folder\shell\open" uses `DelegateExecute`; overriding only `Directory` and `Drive` leaves Folder-class virtual items [I].

## 9. Default terminal

- Registry: `HKCU\Console\%%Startup` values `DelegationConsole` and `DelegationTerminal` (REG_SZ GUID strings). Windows Terminal: console `{2EACA947-7F5F-4CFA-BA87-8F7FBEEFBE69}`, terminal `{E12CFF52-A866-4C77-9A90-F570A7AA2C6B}`. Console Host (legacy): `{B23D10C0-E52E-411E-9D5B-C09FDF709C7D}` for both. All-zero GUID = "let Windows decide" since Terminal PR #13160. [V: Terminal PR https://github.com/microsoft/terminal/pull/13160 and Microsoft docs]. Setting lives in Settings > Privacy & security > For developers (Win 11), Terminal startup settings, and the conhost properties sheet [V: Redmond/Neowin news]. Docs: feature is Windows 11 or Windows 10 Insider Dev only [V: https://learn.microsoft.com/en-us/windows/terminal/customize-settings/startup ].
- Read: read both values; "unset/system default" = values absent or all-zero GUID. Write: per-user, no admin, no prompt [V from registry location].
- Handoff: conhost, when a console app starts WITHOUT an existing console window, looks up the delegation pair, activates the terminal's COM class, and passes the console session via `ITerminalHandoff`; the terminal's class must implement the marker `IDefaultTerminalMarker` to signal it accepts handoffs, otherwise conhost falls back to itself [V from PR text; interface naming `ITerminalHandoff3` unverified, I could not confirm it]. A third-party terminal therefore must register an out-of-proc COM local server (CLSID under HKCU\Software\Classes\CLSID) implementing the handoff interface. WezTerm/Alacritty status not found [unverified].
- Scope: only console-subsystem apps launched from outside a console (Explorer double-click, Run dialog, Start); programs started from an existing terminal never hand off [I, from how conhost works].
- Restore: write back the exact previous pair (including all-zero or missing). Detect user change: pair differs from what qol wrote.

## 10. Slot takeovers

### 10.1 Mechanisms

| Mechanism | Pros | Cons / rules |
|---|---|---|
| `RegisterHotKey` | no hook, cheap, robust, ignored by UIPI for registering, delivered as WM_HOTKEY | "Keyboard shortcuts that involve the WINDOWS key are reserved for use by the operating system"; fails if already registered; some OS defaults (PrintScreen) can be overridden when one of your windows is in the foreground [V: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerhotkey ]; F12 reserved for debugger |
| `WH_KEYBOARD_LL` | sees and can swallow almost everything | needs a message-loop thread; callback must finish within `HKCU\Control Panel\Desktop\LowLevelHooksTimeout` (capped at 1000 ms since Win 10 1709) or on Windows 7+ the hook is SILENTLY removed with no notification; elevated foreground windows are unreachable unless the hook owner is elevated (UIPI); qol's `foreground_reaches_hooks` already models this; injected input has `LLKHF_INJECTED`; return nonzero to swallow [V: https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc ] |
| `DisabledHotkeys` | frees Explorer-registered Win+letter combos so `RegisterHotKey` can win | per-user string in `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced`; each CHARACTER disables that key in ALL Win/Win+Ctrl/Win+Shift combinations; read only when Explorer starts as the shell, so needs sign-out; documented in 2009 for 22 keys [R: Geoff Chappell https://geoffchappell.com/notes/windows/shell/explorer/globalhotkeys.htm ]. Windows 11 users report Win+F still reaches Feedback Hub and "Prt Sc" as a string disabled P, r, t, S, c everywhere, including third-party hotkeys [R: voidtools, ElevenForum]. Only legacy Explorer-registered hotkeys; does NOT cover Win+V, Win+H, Win+Shift+S which are registered by other components [I] |
| `NoWinKeys` policy | disables all Explorer Win hotkeys | `HKLM|HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\Explorer\NoWinKeys` DWORD; GPO "Turn off Windows+X hotkeys" [R: Chappell]; at startup only; too blunt; the Win key alone still opens Start. Do not use |
| Raw Input / `SendInput` for suppression | no | Raw Input cannot swallow keys; Microsoft itself suggests raw input over hooks where possible but it does not block [V: LowLevelKeyboardProc docs] |

AutoHotkey is the reference implementation: it uses `RegisterHotKey` when it can and switches to the keyboard hook for wildcard, up-event, tilde, and combination hotkeys, and for combinations the OS refuses; Win/Alt hotkeys send a mask key (default Ctrl, better `vkE8`) before the Win or Alt key-up so the Start menu or menu bar does not fire [V: https://www.autohotkey.com/docs/v2/Hotkeys.htm , https://www.autohotkey.com/docs/v2/lib/A_MenuMaskKey.htm ]. Flow Launcher falls back to a low-level hook when `RegisterHotKey` refuses a Win+key combo [R: PR 4434].

### 10.2 Per slot

| Slot | How it works today | How qol takes it | Restore / detect | Confidence |
|---|---|---|---|---|
| Start / bare Win key | Explorer / Start experience opens Start on Win key-UP if no other key was pressed in between | Hook: on bare LWin/RWin tap, send mask key (vkE8) so Start does not open, then fire qol launcher; keep Win as modifier for chords. `~LWin::Send "{Blind}{vkE8}"` pattern | Stop hook = Start works again, nothing persistent. Detect: hook silently removed (timeout), so run a watchdog by sending a self-test key or checking `GetLastInputInfo` deltas and re-arming | [V] AutoHotkey docs; behavior [I] on current builds |
| Alt+Tab | Switcher implemented by the OS shell (explorer / win32k), not a registered hotkey; cannot be claimed with `RegisterHotKey` | Hook: swallow Alt+Tab and Alt+Shift+Tab, keep Alt state, show qol switcher, commit on Alt-up. AHK notes "special hotkeys that are substitutes for alt-tab" exist | Persistent: none. Registry `AltTabSettings` (classic vs. Win 10 style) irrelevant to a hook. Win+Tab (Task View) is a separate shell hotkey; same hook route | [I] AHK docs mention alt-tab substitutes; no Microsoft source; the CodeProject claim that `RegisterHotKey` can catch Alt+Tab is unreliable [I] |
| PrtSc | Windows 11 22H2/23H2 (22621.1928 retail; beta 22621.1546, KB5025310) opens Snipping Tool by default. Toggle: Settings > Accessibility > Keyboard "Use the Print screen button to open screen snipping", registry `HKCU\Control Panel\Keyboard\PrintScreenKeyForSnippingEnabled` DWORD (1 on, 0 off) | `RegisterHotKey(VK_SNAPSHOT)` can OVERRIDE the OS default while one of the app's windows is in the foreground (Microsoft docs); for a global claim set `PrintScreenKeyForSnippingEnabled=0` (documented user setting, no admin, may need sign-out or Explorer restart) and register VK_SNAPSHOT or use the hook | Restore: write back the previous value (absent vs 0 vs 1; record absence). Detect user change: value differs from what qol wrote. Windows 10: PrtSc just copies to clipboard, no setting [V: news] | [V] Microsoft docs RegisterHotKey remark; [R] ElevenForum/Winaero; a user reported Snipping Tool still appears after the toggle + policy [R: unresolved] |
| Win+Shift+S | Snipping Tool capture via `ms-screenclip:`; handled by a resident component that launches `ScreenClippingHost.exe`; ShareX users report it cannot be overridden by registering, and uninstalling Snipping Tool does not free it | Hook: swallow Win+Shift+S (and Win+PrtSc, Win+Shift+R) and fire qol-shot; mask key so Start does not open. `RegisterHotKey` is expected to fail [R: ShareX report; I] | Hook-only; stopping the hook gives the key back | [R/I] no primary source for the owner process |
| Win+V (clipboard history) | Shell feature; toggle in Settings > System > Clipboard; policy "Allow Clipboard History" = `HKLM\SOFTWARE\Policies\Microsoft\Windows\System\AllowClipboardHistory` | Hook swallow Win+V; also consider keeping Windows clipboard history on (it feeds the data qol would show) | Hook-only. A policy that disables history makes Win+V do nothing but does not free the combo for `RegisterHotKey` [I] | [R] PowerToys/AHK remap guides confirm remapping works; owner unverified |
| Win+H (dictation / voice typing) | Windows 11 22H2+ opens voice typing (Win+H); on 24H2+ may route to voice access / other, build dependent [I] | Hook swallow Win+H; qol-voice launcher | Hook-only | [I] |
| Snapping (Win+Arrows, Snap layouts, Snap assist) | Settings > System > Multitasking "Snap windows"; `EnableSnapAssistFlyout` DWORD under `HKCU\...\Explorer\Advanced` controls only the maximize-button flyout; toggling "Snap windows" off disables everything snap related | To take over Win+Arrow: hook swallow Win+Left/Right/Up/Down. To avoid double-snapping also disable "Snap windows" (HKCU `Control Panel\Desktop\WindowArrangementActive` = 0 [I]) | Restore the previous value; detect change by compare | [V] Settings path; [I] registry name; Win+Arrow has no policy [I] |
| Lock (Win+L) | Handled by Winlogon / secure attention path | NOT interceptable. `DisableLockWorkstation` policy removes the feature, which is destructive; do not touch | n/a | [I] AHK docs mention Ctrl+Alt+Del; Win+L unsourced |
| Win+E, Win+R, Win+D, Win+B, Win+M, Win+T, Win+F1, Win+Break, Win+1..0 | Explorer registers these via `RegisterHotKey` (22 combos on classic Explorer) | `DisabledHotkeys` (needs shell restart) or hook | Hook-only is simpler | [R] Chappell 2009; list may have grown since [I] |
| Copilot key | Sends Shift+Win+F23 on keyboards that have it [I] | Hook can see it | | [I] unsourced |

### 10.3 Which combos the shell reserves

- Documented: any Win+key is "reserved for use by the operating system" [V]. In practice `RegisterHotKey` fails for the combinations the shell, Settings, Xbox Game Bar, Snipping Tool, input panel, and clipboard components already registered, and for ones Explorer registers; users report Win+J, Win+V, Win+, , Win+F as blocked on Windows 10 [R: wxPython forum, PureBasic forum].
- The robust approach is attempt-and-observe: try `RegisterHotKey`; on failure (return 0, `ERROR_HOTKEY_ALREADY_REGISTERED` 1409) mark the combo as hook-only and arm `WH_KEYBOARD_LL` [V for the error semantics].
- Never reliable by any user-mode route: Ctrl+Alt+Del. Hooks do not see it [R].
- Hooks never reach elevated foreground windows if qol is not elevated (UIPI) and never reach the secure desktop (UAC, lock screen) [V/I].
- If a combination reaches a game with exclusive input (raw input only) the hook may not see it [I].

## 11. Scope of takeover summary

| Takeover | Obeyed by | Bypassed by |
|---|---|---|
| Browser / protocol default via Settings | ShellExecute of URLs, Run dialog, most apps, Start | Edge-internal links, Windows widgets/Start web results outside EEA (they open Edge) [V: BleepingComputer: OS links still opened in Edge; EEA changes narrowed this] |
| Directory/Drive verb override | ShellExecute default verb | `explorer.exe` direct, SHOpenFolderAndSelectItems (partial), taskbar pin, Win+E, Control Panel, Desktop, file dialogs, virtual folders |
| Default terminal | console apps started from outside a console | anything started inside an existing terminal, Windows Terminal profiles |
| Hotkey hook | physical keyboard in non-elevated foreground | elevated apps, secure desktop, raw-input games, injected keys marked `LLKHF_INJECTED` if qol filters them |
| PrintScreen toggle | the PrtSc key | Win+PrtSc, Win+Shift+S, Snipping Tool app launch |

## 12. Read / write / protect / restore matrix (Windows)

| Default | READ | WRITE | Admin | User prompt | Protection | Target must be | Restore + user-change detect |
|---|---|---|---|---|---|---|---|
| File type / URL scheme (incl. browser, mail) | `AssocQueryStringW`; `UserChoiceLatest` then `UserChoice` ProgId | Settings deep link + user click. Never write the Hash | no | Settings page, click "Set default" | hash, UCPD (http, https, pdf ..., and more), `HashVersion` flag | ProgID + Capabilities + RegisteredApplications under HKCU, SHChangeNotify | Restore = unregister qol's ProgID/Capabilities (shell falls back) or ask the user to re-pick the previous app via its own deep link. Detect: snapshot (ProgId, last-write time); if ProgId != qol's, the user changed it |
| Folder opening | HKCU `Directory/Drive` shell default value (absent vs set) | write HKCU verb + default value | no | none | no hash, no UCPD (not on the protected list) | EXE with a command taking `%1` | Restore exact previous default value; detect by comparing to qol's written value |
| Default terminal | `HKCU\Console\%%Startup` pair | write pair | no | none | none known | COM local server implementing handoff; Windows 11 or Insider | Restore previous GUID pair (including absent); compare |
| PrtSc | `HKCU\Control Panel\Keyboard\PrintScreenKeyForSnippingEnabled` | write DWORD 0 | no | none | none | n/a | Restore previous (absent vs 0/1); compare |
| Other hotkeys | n/a (try `RegisterHotKey`) | register / hook | no | none | UIPI limits | n/a | Unregister / unhook; hook liveness watchdog |
| DefaultAssociationsConfiguration | `Software\Policies\Microsoft\Windows\System` value | admin GPO/MDM | yes | none | Pro/Ent/Edu only | XML | qol must only DETECT and warn |

## 13. Version and edition spread

| Topic | Win 10 22H2 | Win 11 22H2/23H2 | Win 11 24H2/25H2/26H2 |
|---|---|---|---|
| Settings "Set default" button | no, per-type picker | yes | yes, EEA covers more types |
| `registeredApp*` deep link | no | 21H2/22H2 with 2023-04 CU, 23H2 | yes |
| UserChoice hash | legacy | legacy | `UserChoiceLatest` (per sources) |
| UCPD | yes (KB5034763) | yes | yes, stack-walk check (4.0), DR dynamic rules |
| Default terminal | Insider Dev only | yes | yes |
| PrtSc opens Snipping Tool | no | 22621.1928+ (default; beta earlier) | yes |
| DefaultAssociationsConfiguration | Pro/Ent/Edu | + `Version`/`Suggested` attrs (22H2+) | same |
| Home | no GPO default associations; DISM import for new profiles; all registry/Settings/hook mechanisms work | | |

## 14. Source list

- UCPD / UserChoice: https://www.bleepingcomputer.com/news/microsoft/new-windows-driver-blocks-software-from-changing-default-web-browser/ ; https://binary.ninja/2025/03/25/default-browser-upcd.html ; https://binary.ninja/2026/08/04/ucpd-dynamic-rules.html ; https://firefox-source-docs.mozilla.org/widget/windows/shell/set-default.html ; https://raw.githubusercontent.com/mozilla/gecko-dev/master/browser/components/shell/WindowsUserChoice.cpp ; https://kolbi.cz/blog/2025/04/20/userchoicelatest-microsofts-new-protection-for-file-type-associations/ (unread) ; https://kolbi.cz/blog/2025/07/15/ucpd-sys-userchoice-protection-driver-part-2/ (unread) ; https://github.com/NousResearch/hermes-agent/pull/134428 ; https://github.com/ComoEstaisAmigos/PS-SFTA ; https://www.ghacks.net/2024/04/08/new-sneaky-windows-driver-ucdp-stops-non-microsoft-software-from-setting-defaults/ (unread)
- Defaults APIs and guidance: https://learn.microsoft.com/en-us/windows/win32/shell/default-programs ; https://learn.microsoft.com/en-us/windows/win32/shell/vista-managing-defaults ; https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-iapplicationassociationregistration-setappasdefault ; https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-default-apps-settings ; https://blogs.windows.com/windowsexperience/2023/03/17/a-principled-approach-to-app-pinning-and-app-defaults-in-windows/ ; https://www.thurrott.com/windows/windows-10/321697/microsoft-makes-more-dma-based-changes-to-windows-10-11-in-the-eu
- Policy / DISM: https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/export-or-import-default-application-associations ; https://learn.microsoft.com/en-us/windows/client-management/mdm/policy-csp-applicationdefaults
- Terminal: https://github.com/microsoft/terminal/pull/13160 ; https://learn.microsoft.com/en-us/windows/terminal/customize-settings/startup
- Folders: https://github.com/Chaoses-Ib/DirectoryOpus/blob/main/Manual/basic_concepts/explorer_replacement.md ; https://resource.dopus.com/t/replace-explorer-in-all-calls/14843 ; https://resource.dopus.com/t/explorer-replacement-not-fully-working-win-e-v13-0-49-beta/47151 ; https://github.com/abdonkov/FileExplorerInterceptor ; https://www.xyplorer.com/xyfc/viewtopic.php?p=214999
- Hotkeys: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerhotkey ; https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc ; https://geoffchappell.com/notes/windows/shell/explorer/globalhotkeys.htm ; https://www.autohotkey.com/docs/v2/Hotkeys.htm ; https://www.autohotkey.com/docs/v2/lib/A_MenuMaskKey.htm ; https://github.com/Flow-Launcher/Flow.Launcher/pull/4434 ; https://www.elevenforum.com/t/enable-or-disable-use-print-screen-key-to-open-screen-snipping-in-windows-11.520/latest ; https://winaero.com/how-to-disable-print-screen-from-opening-snipping-tool-in-windows-11-and-10

## 15. Open items the design must settle by test (not by more reading)

1. Reproduce on a Windows 11 25H2 VM: which of `.htm`, `.html`, `.pdf`, `ftp`, `http`, `https` writes are refused by UCPD, and whether `HashVersion` is 1 by default. Needed only for doctor messages.
2. Confirm by `RegisterHotKey` probe on 24H2 and 25H2 which of Win+V, Win+H, Win+Shift+S, Win+Arrows, Alt+Tab, Win+Tab, PrtSc return 0 and with which `GetLastError`. Store the result in the doctor output, not in the code.
3. Confirm exact keys Files (files-community) writes for "default file manager" and whether Win+E honors them on Windows 11.
4. Confirm the third-party default terminal COM contract (interface name and IIDs) from the Windows Terminal repo (`ITerminalHandoff3`, `IDefaultTerminalMarker`).
5. Prove `AssocQueryStringW` and `UserChoiceLatest` agree after a Settings change on 24H2, 25H2 and 26H2.
6. Check that unregistering qol's ProgID leaves the shell on a sane fallback and does not trigger the "An app default was reset" notification.
