# File Separation of Concerns Inventory

Scope: all tracked files under `src/` and `ui/`.

Legend:
- `Boundary`: the primary architecture seam the file belongs to.
- `SoC status`: `clean`, `review`, `mixed`.
- `Action`: `keep`, `split`, `coalesce`, `move` (proposal seed only; no changes applied).

| File | Lines | Concern | Boundary | SoC status | Action | Notes |
|---|---:|---|---|---|---|---|
| `src/bin/doctor.rs` | 4 | Doctor CLI entrypoint | CLI boundary | clean | keep |  |
| `src/bin/install.rs` | 3 | Installer CLI entrypoint | CLI boundary | clean | keep |  |
| `src/daemon/events.rs` | 133 | Event bus types and publish/subscribe behavior | Daemon internal boundary | clean | keep |  |
| `src/daemon/init.rs` | 20 | Daemon construction helpers | Daemon internal boundary | clean | keep |  |
| `src/daemon/mod.rs` | 185 | Daemon facade and lifecycle orchestration | Daemon boundary | clean | keep |  |
| `src/dev/adapters/mod.rs` | 1 | Dev adapter exports | Dev adapter boundary | clean | keep |  |
| `src/dev/adapters/traits.rs` | 67 | Dev adapter traits/contracts | Abstraction seam | clean | keep |  |
| `src/dev/build.rs` | 298 | Dev build module facade | Dev build boundary | review | keep | Medium-large; verify boundary remains focused |
| `src/dev/build/cargo_build.rs` | 369 | Cargo build process invocation and parsing | Process/tooling boundary | review | keep | Medium-large; verify boundary remains focused |
| `src/dev/build/fingerprint.rs` | 112 | Source fingerprint calculation | Build domain boundary | clean | keep |  |
| `src/dev/build/fingerprint_store.rs` | 61 | Fingerprint persistence | Storage boundary | clean | keep |  |
| `src/dev/build/service.rs` | 272 | Build service orchestration | Dev service boundary | clean | keep |  |
| `src/dev/build/types.rs` | 48 | Build domain types | Domain model boundary | clean | keep |  |
| `src/dev/config.rs` | 73 | Dev configuration loading and defaults | Config boundary | clean | keep |  |
| `src/dev/core/events.rs` | 15 | Dev core event model | Pure core boundary | clean | keep |  |
| `src/dev/core/mod.rs` | 12 | Dev core exports | Pure core boundary | clean | keep |  |
| `src/dev/core/progress_estimator.rs` | 147 | Progress estimation logic | Pure core boundary | clean | keep |  |
| `src/dev/core/progress_parser.rs` | 118 | Console progress parsing logic | Pure core boundary | clean | keep |  |
| `src/dev/core/reducer.rs` | 137 | State reducer for dev runtime | Pure core boundary | clean | keep |  |
| `src/dev/core/state.rs` | 16 | Dev core state model | Pure core boundary | clean | keep |  |
| `src/dev/core/types.rs` | 48 | Shared dev core types | Pure core boundary | clean | keep |  |
| `src/dev/discovery.rs` | 350 | Discovery of local/dev plugins | Discovery boundary | review | keep | Medium-large; verify boundary remains focused |
| `src/dev/linking.rs` | 232 | Dev link management (dev-links.json) | Filesystem/config boundary | clean | keep |  |
| `src/dev/mod.rs` | 24 | Dev module composition | Dev feature boundary | clean | keep |  |
| `src/dev/state.rs` | 98 | Shared dev runtime state | State boundary | clean | keep |  |
| `src/doctor/mod.rs` | 493 | System health checks and fixes | Doctor feature boundary | review | split | High complexity by size; validate single responsibility |
| `src/doctor/platform/linux.rs` | 33 | Linux-specific doctor checks | Platform adapter | clean | keep |  |
| `src/doctor/platform/macos.rs` | 32 | macOS-specific doctor checks | Platform adapter | clean | keep |  |
| `src/doctor/platform/mod.rs` | 23 | Doctor platform dispatch facade | Platform facade | clean | keep |  |
| `src/doctor/platform/windows.rs` | 57 | Windows-specific doctor checks | Platform adapter | clean | keep |  |
| `src/features/mod.rs` | 36 | Feature registry contracts and composition | Feature boundary | clean | keep |  |
| `src/features/plugin_store/github/cache.rs` | 142 | Plugin-store cache persistence and cache model | Cache boundary | clean | keep |  |
| `src/features/plugin_store/github/catalog.rs` | 295 | Plugin catalog parsing, filtering, manifest shaping, and metadata tests | Catalog boundary | clean | keep |  |
| `src/features/plugin_store/github/manifests.rs` | 32 | Plugin manifest fetch across default branches | External API boundary | clean | keep |  |
| `src/features/plugin_store/github/mod.rs` | 102 | GitHub client facade and cache policy | External API boundary | clean | keep | Manifest and release concerns extracted |
| `src/features/plugin_store/github/releases.rs` | 84 | Latest-release fetch and platform-asset verification | Release boundary | clean | keep |  |
| `src/features/plugin_store/github/token.rs` | 141 | GitHub token storage, validation, and request helpers | Auth boundary | clean | keep |  |
| `src/features/plugin_store/installer/mod.rs` | 56 | Plugin installer facade and public API | Installation boundary | clean | keep | Operation locking and transaction flow extracted |
| `src/features/plugin_store/installer/operation_lock.rs` | 75 | Plugin install/update operation lock acquisition and stale-lock recovery | Locking boundary | clean | keep |  |
| `src/features/plugin_store/installer/operations.rs` | 132 | Plugin install/update/uninstall transaction flow | Installation boundary | clean | keep | Locking concerns extracted |
| `src/features/plugin_store/installer/command.rs` | 76 | Git and cargo command execution helpers | Process/tooling boundary | clean | keep |  |
| `src/features/plugin_store/installer/dependency.rs` | 113 | Dependency install orchestration and plan shaping | Installation domain boundary | clean | keep | Manifest, release, and source-build concerns extracted |
| `src/features/plugin_store/installer/dependency/manifest.rs` | 47 | Plugin manifest load and execution-contract preflight | Contract boundary | clean | keep |  |
| `src/features/plugin_store/installer/dependency/release.rs` | 74 | GitHub release asset fetch and binary download | External API boundary | clean | keep |  |
| `src/features/plugin_store/installer/dependency/source_build/artifact.rs` | 92 | Built-binary output resolution, staging, install, and permissions | Build artifact boundary | clean | keep |  |
| `src/features/plugin_store/installer/dependency/source_build/manifest_sanitizer.rs` | 113 | Cargo manifest sanitization for release fallback builds | Build manifest boundary | clean | keep |  |
| `src/features/plugin_store/installer/dependency/source_build/mod.rs` | 50 | Source-build fallback coordination | Build/install boundary | clean | keep | Artifact and manifest concerns extracted |
| `src/features/plugin_store/installer/lock.rs` | 47 | Installer lockfile primitives and stale-lock detection | Concurrency boundary | clean | keep |  |
| `src/features/plugin_store/installer/source.rs` | 149 | Repository clone/default-branch/reset preparation | Source sync boundary | clean | keep |  |
| `src/features/plugin_store/installer/staging.rs` | 191 | Staging, swap, rollback, and temp-dir cleanup | Transaction boundary | clean | keep |  |
| `src/features/plugin_store/mod.rs` | 58 | Plugin store feature facade | Feature boundary | clean | keep |  |
| `src/features/plugin_store/plugin_ui.rs` | 252 | Serving plugin-provided UI assets safely | Web/file serving boundary | clean | keep |  |
| `src/features/plugin_store/release_assets.rs` | 127 | Release asset matching/resolution logic | Release domain boundary | clean | keep |  |
| `src/features/plugin_store/server.rs` | 212 | HTTP API route wiring and middleware composition | HTTP composition boundary | clean | keep | Route composition is now thin after handler extraction |
| `src/features/plugin_store/server/assets.rs` | 51 | Embedded UI asset serving | HTTP static boundary | clean | keep |  |
| `src/features/plugin_store/server/dev_handlers.rs` | 21 | Dev reload/recompile handlers | HTTP handler boundary | clean | keep |  |
| `src/features/plugin_store/server/dev_plugin_cpu/mod.rs` | 347 | Per-plugin CPU sampling service | Telemetry service boundary | review | keep | Medium-large; verify boundary remains focused |
| `src/features/plugin_store/server/dev_plugin_cpu/platform/linux.rs` | 27 | Linux CPU sampling adapter | Platform adapter | clean | keep |  |
| `src/features/plugin_store/server/dev_plugin_cpu/platform/macos.rs` | 41 | macOS CPU sampling adapter | Platform adapter | clean | keep |  |
| `src/features/plugin_store/server/dev_plugin_cpu/platform/mod.rs` | 39 | CPU sampling platform facade | Platform facade | clean | keep |  |
| `src/features/plugin_store/server/dev_runtime.rs` | 397 | Dev runtime mock/build orchestration | Dev service boundary | review | keep | Medium-large; verify boundary remains focused |
| `src/features/plugin_store/server/dev_runtime_state.rs` | 176 | In-memory dev runtime state store | State storage boundary | clean | keep |  |
| `src/features/plugin_store/server/dev_services.rs` | 238 | Dev action queue helper layer | Service boundary | clean | keep |  |
| `src/features/plugin_store/server/helpers.rs` | 104 | Plugin-store HTTP helper utilities | HTTP helper boundary | clean | keep |  |
| `src/features/plugin_store/server/plugin_handlers.rs` | 139 | Plugin action/list/install HTTP handlers | HTTP handler boundary | clean | keep |  |
| `src/features/plugin_store/server/plugin_services.rs` | 326 | Plugin-store domain services behind handlers | Domain service boundary | review | keep | Medium-large; verify boundary remains focused |
| `src/features/plugin_store/server/restart/mod.rs` | 49 | Self-restart abstraction and policy | Lifecycle boundary | clean | keep |  |
| `src/features/plugin_store/server/restart/platform/mod.rs` | 25 | Restart platform dispatch | Platform facade | clean | keep |  |
| `src/features/plugin_store/server/restart/platform/unix.rs` | 63 | Unix restart implementation | Platform adapter | clean | keep |  |
| `src/features/plugin_store/server/restart/platform/windows.rs` | 12 | Windows restart implementation | Platform adapter | clean | keep |  |
| `src/features/plugin_store/server/settings/mod.rs` | 15 | Settings handler composition and exports | HTTP composition boundary | clean | keep |  |
| `src/features/plugin_store/server/settings/media_cover_handlers.rs` | 138 | Cover retrieval/validation handler flow | HTTP handler boundary | review | keep | Medium-large; verify boundary remains focused |
| `src/features/plugin_store/server/settings/media_icon_handlers.rs` | 71 | Bundle icon lookup and PNG encoding handlers | HTTP handler boundary | clean | keep |  |
| `src/features/plugin_store/server/settings/media_apps_handlers.rs` | 11 | Installed-app list handler entrypoint | HTTP handler boundary | clean | keep | Platform logic delegated to platform module |
| `src/features/plugin_store/server/settings/media_apps_handlers/platform/mod.rs` | 12 | Installed-app platform dispatch | Platform facade | clean | keep |  |
| `src/features/plugin_store/server/settings/media_apps_handlers/platform/macos.rs` | 80 | macOS installed-app discovery implementation | Platform adapter | review | keep | Medium-large; verify boundary remains focused |
| `src/features/plugin_store/server/settings/plugin_config_handlers.rs` | 151 | Plugin config GET/PUT handlers | HTTP handler boundary | review | keep | Medium-large; verify boundary remains focused |
| `src/features/plugin_store/server/settings/github_token_handlers.rs` | 65 | GitHub token status/set/delete handlers | HTTP handler boundary | clean | keep |  |
| `src/features/plugin_store/server/settings/hotkey_handlers.rs` | 98 | Hotkeys GET/PUT handlers | HTTP handler boundary | clean | keep |  |
| `src/features/plugin_store/server/types.rs` | 163 | Plugin-store server DTOs and shared state structs | API contract boundary | clean | keep |  |
| `src/features/task_runner/config.rs` | 202 | Task-runner config model, state, and persistence | Config boundary | clean | keep |  |
| `src/features/task_runner/execution.rs` | 111 | Task-runner command execution orchestration | Service boundary | clean | keep |  |
| `src/features/task_runner/handlers.rs` | 136 | Task-runner HTTP handlers and API DTOs | HTTP handler boundary | clean | keep |  |
| `src/features/task_runner/interpolation.rs` | 316 | Template interpolation and shell escaping | Pure utility boundary | clean | keep | Includes property-based and example-based tests |
| `src/features/task_runner/mod.rs` | 13 | Task-runner feature facade | Feature boundary | clean | keep |  |
| `src/features/task_runner/platform/mod.rs` | 20 | Task runner platform dispatch and shell adapter policy | Platform facade | clean | keep | Unix and Windows shell adapters are inlined; unsupported targets fail at compile time |
| `src/hotkeys/listener.rs` | 127 | Hotkey listener loop, reload handling, and action dispatch | Runtime boundary | clean | keep | Listener runtime extracted from hotkey manager module |
| `src/hotkeys/mod.rs` | 131 | Hotkey manager facade and hotkey registration | Feature boundary | review | keep | Parser, storage, listener, and tests extracted; registration flow remains the dense seam |
| `src/hotkeys/parser.rs` | 30 | Hotkey string parsing and modifier/key resolution | Parsing boundary | clean | keep |  |
| `src/hotkeys/store.rs` | 28 | Hotkey config file IO | Storage boundary | clean | keep |  |
| `src/hotkeys/tests.rs` | 189 | Hotkey parser and action-id validation tests | Test boundary | clean | keep |  |
| `src/hotkeys/types.rs` | 104 | Hotkey model/types and key maps | Domain model boundary | clean | keep |  |
| `src/installer/files.rs` | 60 | Installer file operations | Filesystem boundary | clean | keep |  |
| `src/installer/mod.rs` | 134 | Application installer facade | Feature boundary | clean | keep |  |
| `src/installer/platform/linux.rs` | 87 | Linux installer behavior | Platform adapter | clean | keep |  |
| `src/installer/platform/macos.rs` | 76 | macOS installer behavior | Platform adapter | clean | keep |  |
| `src/installer/platform/mod.rs` | 69 | Installer platform dispatch facade | Platform facade | clean | keep |  |
| `src/installer/platform/unix_common.rs` | 148 | Shared Unix installer behavior | Platform shared boundary | clean | keep |  |
| `src/installer/platform/windows.rs` | 98 | Windows installer behavior | Platform adapter | clean | keep |  |
| `src/installer/source.rs` | 111 | Installer source resolution | Source boundary | clean | keep |  |
| `src/lib.rs` | 19 | Crate module exports and public surface | Public API surface | clean | keep |  |
| `src/main.rs` | 145 | Application startup and composition root | Composition root | clean | keep |  |
| `src/menu/builder.rs` | 170 | Tray menu model construction | UI/menu boundary | clean | keep |  |
| `src/menu/mod.rs` | 2 | Menu module exports | UI/menu boundary | clean | keep |  |
| `src/menu/router.rs` | 51 | Menu event routing and dispatch | UI/menu boundary | clean | keep |  |
| `src/os/display/linux.rs` | 154 | Linux display/focus implementation | Platform adapter | clean | keep |  |
| `src/os/display/macos.rs` | 208 | macOS display/focus implementation | Platform adapter | clean | keep |  |
| `src/os/display/mod.rs` | 53 | Display/focus platform facade | Platform facade | clean | keep |  |
| `src/os/mod.rs` | 1 | OS abstraction module export | Platform facade | clean | keep |  |
| `src/paths.rs` | 229 | Config/data path resolution and path safety primitives | Filesystem boundary | clean | keep |  |
| `src/plugins/action_executor.rs` | 121 | Action executor facade and public API | Execution boundary | clean | keep | Resolution, execution, and tracking concerns extracted |
| `src/plugins/action_executor/execution.rs` | 135 | Runtime and daemon action execution flow | Execution boundary | clean | keep |  |
| `src/plugins/action_executor/resolution.rs` | 178 | Action target resolution and runtime-fallback policy | Resolution boundary | clean | keep |  |
| `src/plugins/action_executor/tests.rs` | 308 | Action executor tests | Test boundary | clean | keep |  |
| `src/plugins/action_executor/tracking.rs` | 180 | Action process tracking, reservation, and cleanup | Lifecycle boundary | review | keep | Tracking is isolated; consider platform helper extraction if this grows |
| `src/plugins/action_transport/mod.rs` | 21 | Action transport facade | Transport boundary | clean | keep |  |
| `src/plugins/action_transport/platform/mod.rs` | 20 | Action transport platform dispatch and fallback policy | Platform facade | clean | keep | Windows fallback is inlined; unsupported targets fail at compile time |
| `src/plugins/action_transport/platform/unix_common.rs` | 65 | Unix action transport implementation | Platform adapter | clean | keep |  |
| `src/plugins/action_transport/protocol.rs` | 62 | Action transport protocol framing | Protocol boundary | clean | keep |  |
| `src/plugins/config/mod.rs` | 71 | Plugin config facade and plugin-local restore/set flow | Storage boundary | clean | keep | Storage and tests extracted |
| `src/plugins/config/store.rs` | 44 | Plugin config file IO helpers | Storage boundary | clean | keep |  |
| `src/plugins/config/tests.rs` | 138 | Plugin config tests | Test boundary | clean | keep |  |
| `src/plugins/daemon_lifecycle/log_relay.rs` | 85 | Plugin daemon stdout/stderr relay and per-line suppression | Logging boundary | clean | keep |  |
| `src/plugins/daemon_lifecycle/mod.rs` | 38 | Plugin daemon lifecycle facade and daemon registration | Lifecycle boundary | clean | keep | Spawn, relay, and readiness concerns extracted |
| `src/plugins/daemon_lifecycle/readiness.rs` | 123 | Plugin daemon readiness, socket wait, and shutdown completion | Lifecycle boundary | clean | keep |  |
| `src/plugins/daemon_lifecycle/spawn.rs` | 89 | Plugin daemon process spawn, env setup, and log relay configuration | Process launch boundary | clean | keep |  |
| `src/plugins/daemon_tracker/mod.rs` | 164 | Daemon pid/socket tracking facade | Lifecycle boundary | clean | keep |  |
| `src/plugins/daemon_tracker/platform/linux.rs` | 69 | Linux daemon tracking implementation | Platform adapter | clean | keep | Socket cleanup delegated to shared Unix helper |
| `src/plugins/daemon_tracker/platform/macos.rs` | 96 | macOS daemon tracking implementation | Platform adapter | clean | keep | Socket cleanup delegated to shared Unix helper |
| `src/plugins/daemon_tracker/platform/mod.rs` | 55 | Daemon tracker platform dispatch and Windows fallback policy | Platform facade | clean | keep | Windows fallback is inlined; unsupported targets fail at compile time |
| `src/plugins/daemon_tracker/platform/socket_cleanup.rs` | 127 | Shared Unix stale-socket cleanup policy | Platform shared boundary | clean | keep | Shared by Linux and macOS adapters |
| `src/plugins/execution_contract.rs` | 150 | Plugin command resolution and execution-contract validation | Contract boundary | clean | keep | Path resolution and binary-presence checks extracted from root plugin module |
| `src/plugins/execution_contract_tests.rs` | 51 | Execution-contract path resolution tests | Test boundary | clean | keep |  |
| `src/plugins/loader/manifest_loader.rs` | 45 | Plugin manifest read, parse, and contract validation | Filesystem boundary | clean | keep |  |
| `src/plugins/loader/mod.rs` | 59 | Plugin loader facade and entrypoints | Filesystem boundary | clean | keep | Scan and manifest loading concerns extracted |
| `src/plugins/loader/scan.rs` | 129 | Plugin directory scan, load diagnostics, and platform skip handling | Filesystem boundary | clean | keep |  |
| `src/plugins/loader/tests.rs` | 264 | Plugin loader tests | Test boundary | clean | keep |  |
| `src/plugins/log_control.rs` | 153 | Per-plugin log muting/filtering policy persistence | Config boundary | clean | keep |  |
| `src/plugins/manager/autostart.rs` | 128 | Plugin daemon autostart policy and startup worker fan-out | Lifecycle boundary | clean | keep | Dev-linked autostart policy and tests extracted from manager facade |
| `src/plugins/manager/dev_registry.rs` | 125 | Dev-link registry loading and legacy symlink migration | Dev registry boundary | clean | keep | Dev-link migration and backup restoration extracted from manager facade |
| `src/plugins/manager/loading.rs` | 78 | Plugin manager load pipeline and resolved-source registration | Service boundary | clean | keep | Load orchestration extracted from manager facade |
| `src/plugins/manager/mod.rs` | 50 | Plugin manager facade and public API | Service boundary | clean | keep | Load and runtime concerns extracted |
| `src/plugins/manager/runtime.rs` | 67 | Plugin manager reload, shutdown, and daemon restart operations | Lifecycle boundary | clean | keep | Runtime control extracted from manager facade |
| `src/plugins/manifest/mod.rs` | 37 | Manifest facade and shared traversal/platform helpers | Contract boundary | clean | keep |  |
| `src/plugins/manifest/schema.rs` | 105 | Manifest schema and serde model types | Contract schema boundary | clean | keep |  |
| `src/plugins/manifest/schema_tests.rs` | 318 | Manifest schema parsing and defaulting tests | Test boundary | clean | keep |  |
| `src/plugins/manifest/validation.rs` | 292 | Manifest validation rules and contract enforcement | Contract validation boundary | review | keep | Validation is now isolated; split again only if rules keep growing |
| `src/plugins/manifest/validation_tests.rs` | 281 | Manifest validation tests | Test boundary | clean | keep |  |
| `src/plugins/mod.rs` | 64 | Plugin domain facade and Plugin owner type | Plugin domain boundary | clean | keep | Daemon lifecycle and execution contract were extracted |
| `src/plugins/resolver.rs` | 208 | Plugin source resolution (installed vs linked) | Resolution boundary | clean | keep |  |
| `src/process_utils/mod.rs` | 15 | Process helper facade | Process boundary | clean | keep |  |
| `src/process_utils/platform/mod.rs` | 40 | Process helper platform dispatch and fallback policy | Platform facade | clean | keep | Unsupported targets fail at compile time |
| `src/process_utils/platform/unix_common.rs` | 40 | Unix process helper implementation | Platform adapter | clean | keep |  |
| `src/process_utils/platform/windows.rs` | 49 | Windows process helper implementation | Platform adapter | clean | keep |  |
| `src/runtime/channel.rs` | 11 | Runtime channel contracts | Runtime contract boundary | clean | keep |  |
| `src/runtime/channels/cursor.rs` | 46 | Cursor sampling channel | Runtime polling boundary | clean | keep |  |
| `src/runtime/channels/focus.rs` | 52 | Focus sampling channel | Runtime polling boundary | clean | keep |  |
| `src/runtime/channels/mod.rs` | 3 | Runtime channel composition | Runtime polling boundary | clean | keep |  |
| `src/runtime/channels/monitors.rs` | 41 | Monitor sampling channel | Runtime polling boundary | clean | keep |  |
| `src/runtime/mod.rs` | 7 | Runtime state module exports | Runtime boundary | clean | keep |  |
| `src/runtime/poller.rs` | 45 | Adaptive poll scheduling | Runtime core boundary | clean | keep |  |
| `src/runtime/server.rs` | 430 | Runtime socket server and state publish | Runtime service boundary | review | keep | Medium-large; verify boundary remains focused |
| `src/runtime/state.rs` | 103 | Runtime input/monitor selection state model | Runtime core boundary | clean | keep |  |
| `src/signal.rs` | 68 | Signal handling for process lifecycle | OS/runtime boundary | clean | keep |  |
| `src/tray/icon.rs` | 43 | Tray icon rendering and variants | UI/system tray boundary | clean | keep |  |
| `src/tray/mod.rs` | 37 | Tray manager facade | UI/system tray boundary | clean | keep |  |
| `src/tray/platform/linux.rs` | 121 | Linux tray implementation | Platform adapter | clean | keep |  |
| `src/tray/platform/macos.rs` | 64 | macOS tray implementation | Platform adapter | clean | keep |  |
| `src/tray/platform/mod.rs` | 149 | Tray platform dispatch | Platform facade | clean | keep |  |
| `src/tray/platform/windows.rs` | 50 | Windows tray implementation | Platform adapter | clean | keep |  |
| `src/updates/mod.rs` | 51 | Update check/install orchestration | Update feature boundary | clean | keep |  |
| `src/updates/platform/linux.rs` | 107 | Linux update install implementation | Platform adapter | clean | keep |  |
| `src/updates/platform/mod.rs` | 27 | Update platform dispatch and browser-fallback policy | Platform facade | clean | keep | macOS and Windows fallback is inlined; unsupported targets fail at compile time |
| `src/version.rs` | 155 | Version parsing and normalization helpers | Shared utility | clean | keep |  |
| `ui/api/client.js` | 63 | Fetch wrappers and API request helpers | HTTP client boundary | clean | keep |  |
| `ui/assets/qol-tray.png` | 130 | Brand/icon raster asset | UI asset boundary | clean | keep | Static asset |
| `ui/assets/qol-tray.svg` | 5 | Brand/icon vector asset | UI asset boundary | clean | keep | Static asset |
| `ui/auto-config.html` | 1037 | Plugin settings fallback page | UI fallback boundary | mixed | split | Large mixed file; identify 2-4 extractable seams |
| `ui/components/App.js` | 296 | Top-level app shell and route hosting | UI composition boundary | review | keep | Medium-large; verify boundary remains focused |
| `ui/components/FeedbackPreact.js` | 6 | Feedback message component | UI component boundary | clean | keep |  |
| `ui/components/ModalPreact.js` | 11 | Modal primitive component | UI component boundary | clean | keep |  |
| `ui/components/PageHeader.js` | 14 | Reusable page header component | UI component boundary | clean | keep |  |
| `ui/components/ShortcutLegendPreact.js` | 12 | Preact shortcut legend component | UI component boundary | clean | keep |  |
| `ui/components/SidebarFooter.js` | 100 | Sidebar footer component | UI component boundary | clean | keep |  |
| `ui/components/SidebarNav.js` | 29 | Sidebar navigation component | UI component boundary | clean | keep |  |
| `ui/components/shortcut-legend.js` | 8 | HTML shortcut legend renderer helper | UI helper boundary | clean | keep |  |
| `ui/events.js` | 43 | SSE subscription and reconnect bus | UI event boundary | clean | keep |  |
| `ui/features/task-runner/style.css` | 274 | Task-runner feature styles | UI styling boundary | clean | keep |  |
| `ui/hooks/useAsyncToken.js` | 8 | Async token hook for stale-request guards | UI state boundary | clean | keep |  |
| `ui/hooks/useFeedback.js` | 8 | Feedback state hook | UI state boundary | clean | keep |  |
| `ui/hooks/useFooterShortcuts.js` | 10 | Footer shortcut rendering hook | UI behavior boundary | clean | keep |  |
| `ui/hooks/useGridNav.js` | 70 | Grid keyboard navigation hook | UI behavior boundary | clean | keep |  |
| `ui/hooks/useInstalling.js` | 8 | Install-state hook | UI state boundary | clean | keep |  |
| `ui/hooks/useKeyboard.js` | 12 | Global keyboard handling hook | UI behavior boundary | clean | keep |  |
| `ui/hooks/usePersistedIndex.js` | 19 | Persisted index hook | UI storage boundary | clean | keep |  |
| `ui/hooks/useRefreshOnFocus.js` | 12 | Refresh-on-focus hook | UI behavior boundary | clean | keep |  |
| `ui/hooks/useRouter.js` | 82 | Hash/local route management hook | UI routing boundary | clean | keep |  |
| `ui/hooks/useSSE.js` | 20 | SSE hook primitive | UI event boundary | clean | keep |  |
| `ui/hooks/useSSEDebounce.js` | 17 | Debounced SSE hook | UI event boundary | clean | keep |  |
| `ui/hooks/useScrollIntoView.js` | 9 | Auto-scroll hook | UI behavior boundary | clean | keep |  |
| `ui/hooks/useStateRef.js` | 8 | State + ref synchronization hook | UI state boundary | clean | keep |  |
| `ui/index.html` | 22 | Web UI host shell document | UI host boundary | clean | keep |  |
| `ui/installing.js` | 31 | Shared install-progress helper state | UI shared state boundary | clean | keep |  |
| `ui/lib/hooks.module.js` | 2 | Third-party hooks runtime bundle | Vendor boundary | clean | keep | Vendor file |
| `ui/lib/htm.module.js` | 1 | Third-party HTM runtime bundle | Vendor boundary | clean | keep | Vendor file |
| `ui/lib/html.js` | 3 | HTM binding helper | UI runtime boundary | clean | keep |  |
| `ui/lib/preact.module.js` | 2 | Third-party Preact runtime bundle | Vendor boundary | clean | keep | Vendor file |
| `ui/main.js` | 4 | UI bootstrap and root app mount | UI composition boundary | clean | keep |  |
| `ui/styles.css` | 5 | Legacy/global stylesheet entry | UI styling boundary | clean | keep |  |
| `ui/styles/STYLE_GUIDE.md` | 91 | UI style governance and conventions | UI documentation boundary | clean | keep | Architecture/style reference |
| `ui/styles/common-components.css` | 749 | Reusable shared component styles | UI styling boundary | mixed | split | Large mixed file; identify 2-4 extractable seams |
| `ui/styles/dev-page.css` | 702 | Developer page specific styles | UI styling boundary | mixed | split | Large mixed file; identify 2-4 extractable seams |
| `ui/styles/page-header.css` | 53 | Global page-header styles | UI styling boundary | clean | keep |  |
| `ui/styles/styles.css` | 805 | Global consolidated style tokens/rules | UI styling boundary | mixed | split | Large mixed file; identify 2-4 extractable seams |
| `ui/styles/table.css` | 66 | Reusable table row/column styles | UI styling boundary | clean | keep |  |
| `ui/utils/escape-html.js` | 26 | HTML/attribute escaping helpers | UI security utility boundary | clean | keep |  |
| `ui/utils/keys.js` | 15 | Key-dispatch utility helpers | UI utility boundary | clean | keep |  |
| `ui/utils/plugins.js` | 13 | Plugin payload normalization helpers | UI utility boundary | clean | keep |  |
| `ui/utils/progress.js` | 33 | Progress normalization/math helpers | UI utility boundary | clean | keep |  |
| `ui/views/dev/README.md` | 25 | Dev view behavior notes and operational constraints | UI documentation boundary | clean | keep | Page-specific behavior contract |
| `ui/views/dev/build-animation.js` | 12 | Build animation constants/timings | UI animation boundary | clean | keep |  |
| `ui/views/dev/build-controller.js` | 183 | Dev build progress orchestration | UI page controller boundary | clean | keep |  |
| `ui/views/dev/build-overlay.js` | 617 | Per-row build overlay animation control | UI animation boundary | review | split | High complexity by size; validate single responsibility |
| `ui/views/dev/build/reducer.js` | 64 | Dev build reducer/state transitions | UI reducer boundary | clean | keep |  |
| `ui/views/dev/discovery-controller.js` | 63 | Dev discovery data orchestration | UI page controller boundary | clean | keep |  |
| `ui/views/dev/discovery/reducer.js` | 27 | Dev discovery reducer/state transitions | UI reducer boundary | clean | keep |  |
| `ui/views/dev/index.js` | 938 | Dev page controller/state/event wiring | UI page controller boundary | review | split | Large mixed file; identify 2-4 extractable seams; route/controller/view composition should stay thin |
| `ui/views/dev/mock-controller.js` | 234 | Dev mock flow orchestration | UI page controller boundary | clean | keep |  |
| `ui/views/dev/mock/api.js` | 99 | Dev mock API calls | UI API boundary | clean | keep |  |
| `ui/views/dev/mock/local-build.js` | 80 | Local mock build simulation | UI simulation boundary | clean | keep |  |
| `ui/views/dev/mock/reducer.js` | 129 | Dev mock reducer/state transitions | UI reducer boundary | clean | keep |  |
| `ui/views/dev/plugin-model.js` | 123 | Dev plugin view model transforms | UI page domain boundary | clean | keep |  |
| `ui/views/dev/template.js` | 246 | Dev page template rendering | UI view/render boundary | review | split | Route/controller/view composition should stay thin |
| `ui/views/dev/view.js` | 37 | Dev page mount adapter | UI route boundary | clean | keep |  |
| `ui/views/hotkeys-view.js` | 336 | Hotkeys page behavior and modal flow | UI page boundary | review | keep | Medium-large; verify boundary remains focused |
| `ui/views/plugins-view.js` | 267 | Installed plugins page behavior | UI page boundary | clean | keep |  |
| `ui/views/store-view.js` | 277 | Store page component and interactions | UI page boundary | clean | keep |  |
| `ui/views/store/effects.js` | 36 | Store page effects/API orchestration | UI effects boundary | clean | keep |  |
| `ui/views/store/reducer.js` | 95 | Store page reducer/selectors | UI reducer boundary | clean | keep |  |
| `ui/views/task-runner-view.js` | 374 | Task runner page behavior | UI page boundary | review | keep | Medium-large; verify boundary remains focused |

## Initial Coalescing / Rename Signals

These are only signals from file-level concerns and size, not final moves:

- Coalesce candidates (small facade-only modules): `src/*/mod.rs` files that only re-export or route with minimal logic.
- Split-first hotspots: `src/doctor/mod.rs`, `src/hotkeys/mod.rs`, `src/dev/build/cargo_build.rs`, `src/plugins/manifest/validation.rs`.
- UI unification hotspots: `ui/views/dev/index.js` + `ui/views/dev/template.js` (controller + string-template rendering) should align with component/reducer style used by other pages.
- Platform boundary check: keep OS API usage confined to `platform/*` and `os/*` adapter files.
