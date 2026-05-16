// Runtime Architecture Map — qol-tray (top-down, v5)
//
// Corrections from Increment 2 (m0044):
//   • Registry split from Plugin lock — different files, different purposes.
//     Registry (plugin-registry.json) → ReleaseAsset/DevLink/WorktreeLink slots.
//     Plugin lock (profile/plugins.lock.json) → cloud-sync profile inventory.
//   • Region 04 grows to 3 rows: READ, EXECUTE, SUPERVISE. New bricks:
//     action_transport (qol_runtime protocol), capabilities, daemon_tracker.
//   • Action executor is a fan-in: Menu router, Hotkey listener, axum HTTP
//     all converge here. Drawn as explicit edges from each.
//   • Supervisor → global_hotkey fallback reload signal on daemon transitions
//     (5s tick, 5-strike retry).
//   • Profile sync expanded to expose features/profile = core · http · sync.
//   • Persistence: profile/plugins.lock.json (lock) and ~/.config/qol-tray/
//     plugin-registry.json (registry) are now SEPARATE stores.

window.QOL_DIAGRAM = (() => {
  const CANVAS = { w: 1320, h: 2360 };

  // `accent` is a CSS custom property name (without the `--` prefix) defined
  // in styles.css. Picks the region's full color identity (background, border,
  // label accent, watermark, flow chevron) via the --region-accent token the
  // Region and Node components set inline. `entry: true` marks the region
  // that gets the "▶ entry" pill in compact levels.
  const REGIONS = [
    { id: "r1", ord: "01", title: "User input",           caption: "Runtime input surfaces — all fan into pl-exec", lifetime: "per-action",  x:  60, y:  100, w: 1200, h: 140, accent: "rose"  },
    { id: "r2", ord: "02", title: "Platform integration", caption: "Per-OS adapters; dispatch rejoins after this row", lifetime: "boot-once", x: 60, y: 300, w: 1200, h: 220, accent: "amber" },
    { id: "r3", ord: "03", title: "Daemon core",          caption: "Shared crate — 3 binaries · Tokio",            lifetime: "long-lived",  x:  60, y:  580, w: 1200, h: 460, accent: "violet" },
    { id: "r4", ord: "04", title: "Plugin system",        caption: "read → execute → supervise",                   lifetime: "long-lived",  x:  60, y: 1100, w: 1200, h: 460, accent: "teal"   },
    { id: "r5", ord: "05", title: "Plugin processes",     caption: "Standalone Rust binaries · qol_runtime · supervised", lifetime: "supervised", x: 60, y: 1620, w: 1200, h: 220, boundary: "external", accent: "sage" },
    { id: "r6", ord: "06", title: "Persistence",          caption: "Four conversational quadrants — scope × lifetime", lifetime: "on-disk", x: 60, y: 1880, w: 1200, h: 440, accent: "indigo" },
  ];

  const NODES = [
    // ── 01 User input — 5 chips ───────────────────────────────────
    // Tray and Dashboard source-x SWAPPED so that in the minimal-view
    // row-pack (sorted by source x), u-tray lands in the centered slot
    // and aligns vertically with the platform/router/executor dispatch spine.
    // Detailed view simply reads "Dashboard UI · Tray icon · Global hotkey"
    // left-to-right, with tray correctly in the central role.
    { id: "u-dash",   region: "r1", x:  60, y: 152, w: 220, h: 72, kind: "input", label: "Dashboard UI",  sub: "browser · :42700" },
    { id: "u-tray",   region: "r1", x: 305, y: 152, w: 220, h: 72, kind: "input", label: "Tray icon",     sub: "click · menu" },
    { id: "u-hotkey", region: "r1", x: 550, y: 152, w: 220, h: 72, kind: "input", label: "Global hotkey", sub: "kernel evdev · global_hotkey fallback" },
    { id: "u-short",  region: "r1", x: 795, y: 152, w: 220, h: 72, kind: "input", label: "Shortcut",      sub: "CLI exec shortcut · disk-stored" },
    { id: "u-cli",    region: "r1", x:1040, y: 152, w: 220, h: 72, kind: "input", label: "CLI",           sub: "qol-tray exec …" },

    // ── 02 Platform — 3 cards, 3 bullets each ─────────────────────
    { id: "p-lin", region: "r2", x:  60, y: 328, w: 380, h: 180, kind: "platform", platform: "linux",   label: "Linux",
      bullets: ["AppIndicator + GTK tray", "evdev/uinput hotkey *", ".desktop autostart"],
      note: "* feature linux_evdev → kernel capture, else global_hotkey", code: "src/tray/platform/linux/" },
    { id: "p-mac", region: "r2", x: 470, y: 328, w: 380, h: 180, kind: "platform", platform: "macos",   label: "macOS",
      bullets: ["NSApplication menu bar", "Carbon RegisterEventHotKey", "launchd autostart"],
      note: "objc2-app-kit · mach2", code: "src/tray/platform/macos/" },
    { id: "p-win", region: "r2", x: 880, y: 328, w: 380, h: 180, kind: "platform", platform: "windows", label: "Windows",
      bullets: ["windows-sys tray loop", "RegisterHotKey", "registry Run autostart"],
      note: "daemon socket RPC and state socket are Unix-only", code: "src/tray/platform/windows/" },

    // Synthetic "platform layer" anchor — rendered ONLY in minimal level
    // so the spine arrows don't have to lie by picking a specific OS card
    // (Linux looks "special" if both tray and hotkey arrow into it while
    // macOS and Windows sit visible-but-unconnected). The minimalOnly
    // flag is respected by computeDescriptiveLayout and the detailed
    // pass so this node disappears in those views, where the three real
    // OS cards take over.
    { id: "p-os", region: "r2", x: 460, y: 380, w: 400, h: 60, kind: "core",
      label: "Platform layer",
      sub: "Linux · macOS · Windows · per-OS adapter",
      minimalOnly: true },

    // ── 03 Daemon core — pre-tokio / tokio anchor / services / IPC entries

    // Pre-tokio strip (main thread, sync)
    { id: "d-boot",   region: "r3", x:  75, y: 612, w: 270, h: 70, kind: "core", label: "Bootstrap",   sub: "main() · single-instance · install", code: "src/main.rs · installer/", phase: "pre-tokio" },
    { id: "d-paths",  region: "r3", x: 375, y: 612, w: 270, h: 70, kind: "core", label: "Runtime dirs",sub: "wipe /tmp · recreate",       code: "src/paths.rs",            phase: "pre-tokio" },
    { id: "d-house",  region: "r3", x: 675, y: 612, w: 270, h: 70, kind: "core", label: "Housekeeping",sub: "migrations · cleanup",       code: "src/housekeeping.rs",     phase: "pre-tokio" },
    { id: "d-doctor", region: "r3", x: 975, y: 612, w: 270, h: 70, kind: "core", label: "Doctor",      sub: "2 checks · 8 fixes · safe|with_de_fixes policy",       code: "src/doctor/",             phase: "pre-tokio" },

    // Tokio anchor
    { id: "d-tokio", region: "r3", x: 75, y: 718, w: 1170, h: 72, kind: "core", label: "Tokio runtime",
      sub: "multi-thread · spawn · select! · broadcast", code: "tokio 1.35 · runtime::Builder" },

    // Long-lived async services
    { id: "d-bus",    region: "r3", x:  75, y: 810, w: 280, h: 80, kind: "core", label: "Event bus",
      sub: "tokio::broadcast · cap 64", code: "src/daemon/events.rs",
      bullets: ["PluginsChanged · Manifest/Resolved/Unavailable", "UpdateProgress · Complete · Failed", "dev: Discovery · Build · Cpu · SelfRecompile"] },
    { id: "d-feat",   region: "r3", x: 365, y: 810, w: 280, h: 80, kind: "core", label: "Features",
      sub: "MenuProvider trait + direct-call subs", code: "src/features/",
      bullets: ["trait: plugin_store · mode_toggle*", "direct: 4 subs + dev tools*"] },
    { id: "d-update", region: "r3", x: 655, y: 810, w: 280, h: 80, kind: "core", label: "Update check",
      sub: "tarball · self-replace · 2s timeout", code: "src/updates/" },
    { id: "d-sync",   region: "r3", x: 945, y: 810, w: 300, h: 80, kind: "core", label: "Profile sync",
      sub: "GitHub gist · Folder providers", code: "src/features/profile/{core,http,sync}/",
      bullets: ["pull_on_launch (boot spawn)", "auto_push_if_dirty (timer loop)"] },

    // Request entry surfaces — 3 IPC + 1 router. Router and api source-x
    // SWAPPED so that in minimal, d-router (the tray-click spine node)
    // takes the centered slot and aligns vertically with u-tray/p-os/
    // pl-exec/px-target. Detailed view reads "axum HTTP · Menu
    // router · Runtime state socket" left-to-right.
    { id: "d-api",    region: "r3", x:  75, y: 920, w: 380, h: 100, kind: "api",    label: "axum HTTP",
      sub: "plugins · profile · sync · meta · logs · dev",       code: "src/features/plugin_store/ · axum 0.8", ipc: "POST /api/plugins/:id/actions/:id · /api/events · /api/dev/* gated" },
    { id: "d-router", region: "r3", x: 465, y: 920, w: 380, h: 100, kind: "router", label: "Menu router",
      sub: "OS std::thread — not tokio",  code: "src/menu/router.rs",     ipc: "tray-icon::MenuEvent → action_executor" },
    { id: "d-state",  region: "r3", x: 855, y: 920, w: 390, h: 100, kind: "state",  label: "Runtime state socket",
      sub: "/tmp/qol-tray-state.sock — unix only", code: "src/runtime/server.rs",
      ipc: "monitor · cursor · focus feed · 2× std::thread · STATE_SOCKET env" },
    { id: "d-hotkey", region: "r3", x: 855, y: 920, w: 380, h: 72, kind: "router", label: "Hotkey callback",
      sub: "hotkeys module → action_executor",
      minimalOnly: true },

    // ── 04 Plugin system — 3 rows: READ, EXECUTE, SUPERVISE ──────
    // Row 1 — READ pipeline (top-row anchor mini-label "READ" sits on the row)
    { id: "pl-disc", region: "r4", x:  70, y: 1140, w: 280, h: 88, kind: "plug", lane: "READ", label: "GitHub discovery",    sub: "reqwest · qol-tools org",    code: "src/features/plugin_store/" },
    { id: "pl-load", region: "r4", x: 370, y: 1140, w: 280, h: 88, kind: "plug", label: "Loader",              sub: "scan · manifest_loader", code: "src/plugins/loader/" },
    { id: "pl-mani", region: "r4", x: 670, y: 1140, w: 280, h: 88, kind: "plug", label: "Manifest validation", sub: "plugin.toml · binary-in-dir · candidate ladder",  code: "src/plugins/manifest/",
      bullets: ["execution_contract.rs gates the binary", "source-aware: primary vs target/debug|release", "Windows suffix: primary .exe", "is_allowed_candidate canonicalises (symlink-safe)"] },
    { id: "pl-reg",  region: "r4", x: 970, y: 1140, w: 280, h: 88, kind: "plug", label: "Registry",            sub: "active · fallback · live slots", code: "src/plugins/registry/",
      bullets: ["SlotSource: ReleaseAsset · DevLink · WorktreeLink", "PluginSource: Installed · DevLinked", "active-worktree.txt can rewrite dev paths in memory"] },

    // Row 2 — EXECUTE pipeline
    { id: "pl-res",   region: "r4", x:  70, y: 1248, w: 280, h: 96, kind: "plug", lane: "EXEC", label: "Action resolver",         sub: "socket · runtime.command · path equality",
      code: "src/plugins/action_executor/resolution.rs",
      bullets: ["no daemon socket → runtime if command exists", "daemon socket + no runtime → daemon only", "different runtime/daemon paths → runtime fallback allowed", "same path → fallback only when socket is unreachable"] },
    { id: "pl-exec",  region: "r4", x: 370, y: 1248, w: 280, h: 96, kind: "plug-anchor", label: "Action executor", sub: "fan-in · daemon|runtime fork",
      code: "src/plugins/action_executor.rs",
      bullets: ["from: menu router · hotkey · axum POST · axum GET", "daemon_socket=Some → execute_via_daemon", "daemon_socket=None → execute_via_runtime"] },
    { id: "pl-trans", region: "r4", x: 670, y: 1248, w: 280, h: 96, kind: "plug", label: "Action transport", sub: "unix socket · ndjson · 10s timeout",
      code: "src/plugins/action_transport/", note: "Unix: qol_runtime JSON request/response · Windows: unavailable" },
    { id: "pl-cap",   region: "r4", x: 970, y: 1248, w: 280, h: 96, kind: "plug", label: "Capabilities", sub: "registry · near-empty",
      code: "src/plugins/capabilities.rs",
      bullets: ["framework: CapabilityRegistry exists", "enforcement: almost nothing host-side", "platform: serial-only · linux"] },

    // Row 3 — SUPERVISE pipeline
    { id: "pl-life", region: "r4", x:  70, y: 1364, w: 280, h: 96, kind: "plug", lane: "SUPER", label: "Daemon lifecycle",
      sub: "spawn · setsid · 2-mode readiness", code: "src/plugins/daemon_lifecycle/",
      note: "env: DAEMON_SOCKET · STATE_SOCKET · REPLACE_EXISTING=1" },
    { id: "pl-track",region: "r4", x: 370, y: 1364, w: 280, h: 96, kind: "plug", label: "Daemon tracker",
      sub: "PID files · orphan kill",            code: "src/plugins/daemon_tracker/", note: "/tmp/qol-tray/pids/<id>.pid" },
    { id: "pl-sup",  region: "r4", x: 670, y: 1364, w: 280, h: 96, kind: "plug", label: "Supervisor",
      sub: "5s tick · 5-strike retry",           code: "src/plugins/daemon_supervisor.rs", note: "transition → global_hotkey reload signal" },
    { id: "pl-mgr",  region: "r4", x: 970, y: 1364, w: 280, h: 96, kind: "plug", label: "PluginManager",
      sub: "HashMap<PluginId, Plugin> + ResolutionReport", code: "src/plugins/manager/",
      bullets: ["in-memory · load · reload · shutdown", "ensure/restart daemon"] },

    // ── 05 Plugin processes — runtime.command + daemon.command ───
    // Generic A/B/C examples are for descriptive/detailed levels. Minimal
    // uses px-target so it does not imply one action dispatches to every
    // visible plugin.
    { id: "px-b", region: "r5", x: 105, y: 1648, w: 360, h: 180, kind: "ext", label: "plugin · B", sub: "manifest · menu · capabilities", pid: "PID 4012", daemon: true,
      internals: ["daemon.command + .socket", "actions (no runtime)"] },
    { id: "px-a", region: "r5", x: 495, y: 1648, w: 360, h: 180, kind: "ext", label: "plugin · A", sub: "manifest · menu · capabilities", pid: "PID 4011", daemon: true,
      internals: ["daemon.command + .socket", "runtime.command (one-shot)", "qol_runtime::DaemonResponse"] },
    { id: "px-c", region: "r5", x: 885, y: 1648, w: 360, h: 180, kind: "ext", label: "plugin · C", sub: "manifest · ephemeral",            pid: "—",        daemon: false,
      internals: ["runtime.command (one-shot)", "no daemon socket"] },
    { id: "px-target", region: "r5", x: 495, y: 1648, w: 360, h: 72, kind: "ext", label: "selected plugin", sub: "daemon or runtime path", pid: "target", daemon: true,
      minimalOnly: true },

    // ── 06 Persistence — 4 quadrants × ~4 files each ─────────────
    // Quadrants laid out 2×2 inside the band. Each quadrant body is rendered
    // as a sub-region (see META.quadrants). Coordinates here are individual
    // file chips. Compact node-kind `ps-file` renders label + path + badges.

    // TL  — CONFIG · machine-local · durable
    { id: "ps-regfile", region: "r6", x:  85, y: 1968, w: 280, h: 64, kind: "ps-file", quad: "tl", label: "Plugin registry",
      path: "config/plugin-registry.json", reads: ["resolver"], writes: ["installer", "dev-link"] },
    { id: "ps-mode",    region: "r6", x: 375, y: 1968, w: 270, h: 64, kind: "ps-file", quad: "tl", label: "Mode",
      path: "config/mode.json",            reads: ["bootstrap"], writes: ["--write-mode", "mode_toggle"] },
    { id: "ps-gh",      region: "r6", x:  85, y: 2042, w: 280, h: 64, kind: "ps-file", quad: "tl", label: "GitHub auth",
      path: "config/.github-{token,auth}.json", reads: ["discovery"], writes: ["oauth flow"] },
    { id: "ps-marker",  region: "r6", x: 375, y: 2042, w: 270, h: 64, kind: "ps-file", quad: "tl", label: "Install markers",
      path: "config/.first-run-done · data/active-install-id", reads: ["bootstrap"], writes: ["installer"] },

    // TR — PROFILE · portable · durable
    { id: "ps-prof",    region: "r6", x: 670, y: 1968, w: 285, h: 64, kind: "ps-file", quad: "tr", label: "Profile manifest",
      path: "profile/manifest.json",       reads: ["profile-sync"], writes: ["profile-sync"] },
    { id: "ps-lock",    region: "r6", x: 965, y: 1968, w: 285, h: 64, kind: "ps-file", quad: "tr", label: "Plugin lock",
      path: "profile/plugins.lock.json",   reads: ["profile-sync"], writes: ["installer", "profile-sync"] },
    { id: "ps-core",    region: "r6", x: 670, y: 2042, w: 285, h: 64, kind: "ps-file", quad: "tr", label: "Core configs",
      path: "profile/core/{hotkeys,shortcuts,task-runner}.json", reads: ["hotkeys", "shortcuts", "task_runner"], writes: ["user edit"] },
    { id: "ps-pcfg",    region: "r6", x: 965, y: 2042, w: 285, h: 64, kind: "ps-file", quad: "tr", label: "Plugin configs",
      path: "profile/plugin-configs/<id>.json", reads: ["plugin proc"], writes: ["plugin proc", "profile-sync"] },

    // BL — RUNTIME · machine-local · ephemeral
    { id: "ps-pids",    region: "r6", x:  85, y: 2148, w: 280, h: 64, kind: "ps-file", quad: "bl", label: "Plugin PIDs",
      path: "/tmp/qol-tray/pids/<id>.pid", reads: ["supervisor", "doctor"], writes: ["daemon_tracker"] },
    { id: "ps-cache",   region: "r6", x: 375, y: 2148, w: 270, h: 64, kind: "ps-file", quad: "bl", label: "Plugin store cache",
      path: "/tmp/qol-tray/cache/",        reads: ["discovery"], writes: ["discovery"] },
    { id: "ps-statesk", region: "r6", x:  85, y: 2222, w: 280, h: 64, kind: "ps-file", quad: "bl", label: "State socket",
      path: "/tmp/qol-tray-state.sock",    reads: ["plugin daemons"], writes: ["runtime::server"], env: "QOL_TRAY_STATE_SOCKET" },
    { id: "ps-pluginsk",region: "r6", x: 375, y: 2222, w: 270, h: 64, kind: "ps-file", quad: "bl", label: "Plugin sockets",
      path: "/tmp/qol-tray-<id>.sock",     reads: ["action_transport"], writes: ["plugin daemon", "clean_stale_sockets"], env: "QOL_TRAY_DAEMON_SOCKET" },

    // BR — SYNC + LOGS · portable / regenerable
    { id: "ps-sync",    region: "r6", x: 670, y: 2148, w: 285, h: 64, kind: "ps-file", quad: "br", label: "Sync state",
      path: "config/sync/state.json",      reads: ["profile-sync"], writes: ["profile-sync"] },
    { id: "ps-backup",  region: "r6", x: 965, y: 2148, w: 285, h: 64, kind: "ps-file", quad: "br", label: "Sync backups",
      path: "config/sync/backups/*",       reads: ["recovery"], writes: ["profile-sync"] },
    { id: "ps-logs",    region: "r6", x: 670, y: 2222, w: 580, h: 64, kind: "ps-file", quad: "br", label: "Daily logs",
      path: "logs/qol-tray.YYYY-MM-DD",    reads: ["dashboard"], writes: ["tracing"] },
  ];

  // Band gutters
  const GUTTERS = [
    { fromY: 240,  toY: 300,  label: "input"      },
    { fromY: 520,  toY: 580,  label: "events · OS thread"     },
    { fromY: 1040, toY: 1100, label: "actions"    },
    { fromY: 1560, toY: 1620, label: "ipc · process boundary", dashed: true, tone: "slate" },
    { fromY: 1840, toY: 1880, label: "reads ↑   writes ↓" },
  ];

  const EDGES = [
    // ── bypasses: dashboard + CLI talk to the API directly ───────
    { from: "u-dash", fromSide: "right", to: "d-api", toSide: "right", tone: "ink", bypass: true },
    { from: "u-cli",  fromSide: "right", to: "d-api", toSide: "right", tone: "ink", bypass: true, hairline: true },
    // Shortcuts is a stored config the CLI loads + executes (qol-tray exec shortcut <id>).
    // Edge documents that it's a CLI-fed surface, not an independent input.
    { from: "u-short", fromSide: "right", to: "u-cli", toSide: "left", tone: "slate", hairline: true, dashed: true },

    // ── 03 daemon core internal wiring ───────────────────────────
    // Pre-tokio sync chain
    { from: "d-boot",  fromSide: "right", to: "d-paths",  toSide: "left", tone: "ink", internal: true },
    { from: "d-paths", fromSide: "right", to: "d-house",  toSide: "left", tone: "ink", internal: true },
    { from: "d-house", fromSide: "right", to: "d-doctor", toSide: "left", tone: "ink", internal: true },
    // Tokio spine → long-lived services
    { from: "d-tokio", fromSide: "bottom", to: "d-bus",    toSide: "top", tone: "ink", internal: true },
    { from: "d-tokio", fromSide: "bottom", to: "d-feat",   toSide: "top", tone: "ink", internal: true, hairline: true },
    { from: "d-tokio", fromSide: "bottom", to: "d-update", toSide: "top", tone: "ink", internal: true, hairline: true },
    { from: "d-tokio", fromSide: "bottom", to: "d-sync",   toSide: "top", tone: "ink", internal: true, hairline: true },
    // Feature registry mounts axum.
    { from: "d-feat", fromSide: "bottom", to: "d-api", toSide: "top", tone: "ink", internal: true, hairline: true },

    // ── 04 plugin system internal wiring ─────────────────────────
    // Row 1 chain (READ): Discovery → Loader → Manifest → Registry
    { from: "pl-disc", fromSide: "right",  to: "pl-load", toSide: "left", tone: "ink", internal: true },
    { from: "pl-load", fromSide: "right",  to: "pl-mani", toSide: "left", tone: "ink", internal: true },
    { from: "pl-mani", fromSide: "right",  to: "pl-reg",  toSide: "left", tone: "ink", internal: true },
    // Wrap to Row 2: Registry → Resolver
    { from: "pl-reg",  fromSide: "bottom", to: "pl-res",  toSide: "top",  tone: "ink", internal: true, wrap: true },
    // Row 2 chain (EXECUTE): Resolver → Executor → Transport
    { from: "pl-res",  fromSide: "right",  to: "pl-exec", toSide: "left", tone: "ink", internal: true },
    { from: "pl-exec", fromSide: "right",  to: "pl-trans",toSide: "left", tone: "ink", internal: true },
    // Capabilities sits as a sidecar — checked by Executor pre-dispatch.
    { from: "pl-exec", fromSide: "bottom", to: "pl-cap",  toSide: "top",  tone: "ink", internal: true, hairline: true, dashed: true },
    // Wrap to Row 3 (SUPERVISE) — Manager owns the chain.
    { from: "pl-mgr",  fromSide: "top",    to: "pl-cap",  toSide: "bottom", tone: "ink", internal: true, hairline: true, dashed: true },
    // Row 3 chain (SUPERVISE): Lifecycle → Tracker → Supervisor → Manager
    { from: "pl-life", fromSide: "right",  to: "pl-track",toSide: "left", tone: "ink", internal: true },
    { from: "pl-track",fromSide: "right",  to: "pl-sup",  toSide: "left", tone: "ink", internal: true },
    { from: "pl-sup",  fromSide: "right",  to: "pl-mgr",  toSide: "left", tone: "ink", internal: true, hairline: true },

    // ── Fan-in: hotkey, router, axum all converge on action_executor
    { from: "d-router", fromSide: "bottom", to: "pl-exec", toSide: "top", tone: "amber", internal: true },
    { from: "d-api",    fromSide: "bottom", to: "pl-exec", toSide: "top", tone: "ink",   internal: true, hairline: true },
    // Hotkey listener fans in directly from User input across all bands —
    // drawn as a long right-rail amber dashed line so the path is legible.
    { from: "u-hotkey", fromSide: "bottom", to: "pl-exec", toSide: "top", tone: "amber", longRail: "left", dashed: true },

    // ── Supervisor → hotkey reload (transition triggers retable) ─
    { from: "pl-sup", fromSide: "top", to: "u-hotkey", toSide: "bottom", tone: "amber", longRail: "right", dashed: true, hairline: true, reverse: true },

    // ── persistence drains (thin dashed) ─────────────────────────
    // Edges into the TL · CONFIG quadrant
    { from: "pl-reg",  fromSide: "bottom", to: "ps-regfile", toSide: "top", tone: "ink", hairline: true, dashed: true },
    { from: "d-house", fromSide: "bottom", to: "ps-mode",    toSide: "top", tone: "ink", hairline: true, dashed: true },
    { from: "d-doctor",fromSide: "bottom", to: "ps-marker",  toSide: "top", tone: "ink", hairline: true, dashed: true },
    // Edges into the TR · PROFILE quadrant
    { from: "d-sync",  fromSide: "bottom", to: "ps-prof",    toSide: "top", tone: "ink", hairline: true, dashed: true },
    { from: "d-sync",  fromSide: "bottom", to: "ps-lock",    toSide: "top", tone: "ink", hairline: true, dashed: true },
    // Edges into the BL · RUNTIME quadrant
    { from: "pl-track",fromSide: "bottom", to: "ps-pids",     toSide: "top", tone: "ink", hairline: true, dashed: true },
    { from: "d-state", fromSide: "bottom", to: "ps-statesk",  toSide: "top", tone: "ink", hairline: true, dashed: true },
    { from: "pl-trans",fromSide: "bottom", to: "ps-pluginsk", toSide: "top", tone: "slate", hairline: true, dashed: true },
  ];

  const TRACES = [
    // T0 — Boot (the trace that gives the pre-tokio/tokio bands meaning)
    {
      id: "t-boot", ord: "T0", label: "Cold start",
      steps: ["d-boot","d-paths","d-house","d-doctor","d-tokio","d-update","d-sync","d-feat","d-api","pl-load","pl-life","pl-track","pl-sup","px-a","p-lin","u-tray"],
      narrative: "Pre-tokio (main thread, synchronous): bootstrap → init /tmp runtime dirs → housekeeping migrations → doctor auto-fix. Then Tokio multi-thread spins up: 2s update check, profile-sync pull on launch, feature registry mounts axum on :42700, plugin loader walks ~/.config/qol-tray/plugins/, lifecycle spawns daemons (tracker writes PIDs under /tmp/qol-tray/pids/), supervisor begins its 5s tick with 5-strike retry budget. Control then returns to the main thread, which builds the native TrayManager and enters the OS event loop — tray icon attaches at step 20, NOT step 1.",
    },
    // T1 — Tray click → daemon plugin
    {
      id: "t-tray-daemon", ord: "T1", label: "Tray → daemon",
      steps: ["u-tray","p-lin","d-router","pl-exec","pl-trans","px-a"],
      narrative: "OS-native tray callback fires on tray-icon's std::thread receiver — not tokio. EventRouter::route looks up the menu id; action_executor resolves the action (daemon_socket=Some) and execution::execute_via_daemon hands off to action_transport. It writes a qol_runtime::protocol::DaemonRequest { action: action_id } JSON line over the plugin's Unix socket and reads back a DaemonResponse { status, data? }. The event bus is NOT on this path.",
    },
    // T2 — Tray click → ephemeral plugin (the spawn-and-exit case)
    {
      id: "t-tray-ephemeral", ord: "T2", label: "Tray → ephemeral",
      steps: ["u-tray","p-lin","d-router","pl-exec","px-c"],
      narrative: "Same fan-in as T1, but resolve_action returns daemon_socket=None, or a daemon response allows fallback and runtime.command exists. execution::execute_via_runtime validates the path (relative, no '..'), then std::process::Command::new(command_path).env('QOL_TRAY_DAEMON_SOCKET', …).spawn(). A fresh std::thread waits for the child to exit and untracks it. No daemon socket RPC is used for the one-shot path.",
    },
    // T3 — Hotkey (fan-in)
    {
      id: "t-hotkey", ord: "T3", label: "Hotkey", tone: "amber",
      steps: ["u-hotkey","p-lin","pl-exec","pl-trans","px-b"],
      narrative: "Two backends, one convergence. Linux with feature linux_evdev: kernel reads /dev/input/event*, matches the combo, re-emits via /dev/uinput. Everywhere else: the global_hotkey crate fires its registered callback (Carbon on macOS, RegisterHotKey on Windows). Either backend's callback invokes action_executor::execute_action — same node as T1. On the global_hotkey fallback path, supervisor transitions can signal a binding-table reload (see T6).",
    },
    // T4 — Dashboard / CLI → axum POST → executor
    {
      id: "t-axum", ord: "T4", label: "Dashboard / CLI → axum",
      steps: ["u-dash","d-api","pl-exec","pl-trans","px-a"],
      narrative: "POST /api/plugins/:id/actions/:action lands on a tokio worker. plugin_handlers::execute_plugin_action calls try_execute_action SYNCHRONOUSLY — not async — so the worker thread BLOCKS on the plugin's Unix socket I/O. A slow plugin daemon starves the axum pool. The CLI is identical: a second qol-tray binary writes a raw HTTP POST to :42700, reads the response, exits.",
    },
    // T5 — Install cascade. THE multi-region trace — exercises Event bus, SSE,
    // tray rebuild, supervisor catch-up, hotkey reload, and three persistence writes.
    {
      id: "t-install", ord: "T5", label: "Install cascade",
      steps: ["u-dash","d-api","d-feat","pl-disc","pl-mani","pl-reg","ps-regfile","ps-lock","d-bus","px-a","u-hotkey"],
      narrative: "POST /api/install/:id → plugin_services::install_plugin. operation_lock per-plugin. installer::source resolves a GitHub release URL; reqwest streams tarball+sha256 into staging; staging untars to /tmp; manifest + execution_contract validate the staged plugin (binary-in-dir, semver); atomic rename → ~/.config/qol-tray/plugins/<id>/. registry::save_registry writes plugin-registry.json; profile::storage updates plugins.lock.json. PluginManager.reload_plugins() stops old daemons, resolves registry/fallback slots, cleans stale sockets, autostarts daemon-enabled installed plugins, writes pid files, and refreshes launcher state. Then hotkeys::trigger_reload signals the global_hotkey fallback listener and EventBus.send_plugins_changed broadcasts to /api/events SSE and tray subscribers.",
    },
    // T6 — Hotkey reload — the back-edge from Plugin system to User input.
    {
      id: "t-hotkey-reload", ord: "T6", label: "Hotkey reload", tone: "amber",
      steps: ["pl-sup","u-hotkey","ps-core"],
      narrative: "The system's most counter-intuitive edge, with a backend caveat. On daemon state transitions, supervisor calls hotkeys::trigger_reload; that signal is consumed only when the global_hotkey fallback listener is running. The listener rebuilds bindings from on-disk hotkeys.json and the catalog of currently loaded plugin actions, then re-registers enabled bindings. The Linux kernel evdev capture path currently has no equivalent reload channel.",
    },
    // T7 — Query (read-only sibling of T1/T4)
    {
      id: "t-query", ord: "T7", label: "Query (read-only)",
      steps: ["u-dash","d-api","pl-exec","pl-trans","px-a"],
      narrative: "axum GET /api/plugins/:id/queries/:query. plugin_handlers::query_plugin → action_executor::dispatch_query goes through action_transport, but the plugin's DaemonResponse.data payload is RETURNED as the HTTP response body. Actions are fire-and-forget-with-ack; queries are read-and-return.",
    },
    // T8 — Stale socket recovery (3-way protocol)
    {
      id: "t-stale-socket", ord: "T8", label: "Stale socket recovery",
      steps: ["pl-load", "pl-track", "pl-life", "ps-pluginsk", "px-a"],
      narrative: "Three-way reconciliation. (a) During plugin load, after registry resolution and manifest loading, PluginManager calls daemon_tracker::clean_stale_sockets(&plugins). It scans known plugin.toml sockets AND walks runtime temp dirs for managed qol-*.sock orphans. For each candidate, has_live_listener() opens a connect() probe: success → leave alone (a real daemon is bound), failure (ECONNREFUSED/ENOENT) → unlink. (b) When a plugin daemon spawns with REPLACE_EXISTING=1 in env, the plugin itself can unlink the socket path before bind() so a surviving file does not block startup. (c) The probe + env-var convention reconcile any race: 'file exists' is unreliable, 'listener exists' is canonical.",
    },
  ];

  const META = {
    binaries: "qol-tray  ·  qol-tray-install  ·  qol-tray-doctor — same crate, three entry points",
    tokioBoundary: { y: 700 },
    // Region 06 quadrant frame. The Quadrants component reads these.
    quadrants: [
      { id: "tl", x:  70, y: 1920, w: 580, h: 180, ord: "TL", title: "Config",      axisX: "machine-local", axisY: "durable",     glyph: "⚙" },
      { id: "tr", x: 655, y: 1920, w: 605, h: 180, ord: "TR", title: "Profile",     axisX: "portable",      axisY: "durable",     glyph: "👤" },
      { id: "bl", x:  70, y: 2105, w: 580, h: 195, ord: "BL", title: "Runtime",     axisX: "machine-local", axisY: "ephemeral",   glyph: "⌛" },
      { id: "br", x: 655, y: 2105, w: 605, h: 195, ord: "BR", title: "Sync · Logs", axisX: "portable",      axisY: "regenerable", glyph: "☁" },
    ],

    // ── Presentation policy (edit here, refresh the page) ──────────────────
    // Node IDs visible in the minimal level. Everything else is hidden by
    // the compact layout pass. Pick the canonical, "most important" node per
    // region (one or a few).
    tier1: [
      "u-tray", "u-dash", "u-hotkey",
      // r2: ONE synthetic "platform layer" card in minimal. The three real
      // OS cards (p-lin, p-mac, p-win) are tier-2 — they appear in
      // descriptive + detailed, where the user can see the per-OS detail.
      // In minimal we want the arrow to read as "into the platform layer",
      // not "into Linux specifically".
      "p-os",
      "d-api", "d-router", "d-hotkey",
      "pl-exec",
      "px-target",
    ],

    // Synthesized flow edges for minimal. The source EDGES walk every
    // intermediate node; in minimal those intermediaries are filtered out so
    // this transitive set is rendered instead. Tones are CSS classes
    // (ink|amber|slate) used by the Edges component for stroke + arrow color.
    minimalFlow: [
      { from: "u-tray",   to: "p-os",     tone: "ink" },
      { from: "p-os",     to: "d-router", tone: "ink" },
      { from: "d-router", to: "pl-exec",  tone: "ink" },
      { from: "pl-exec",  to: "px-target", tone: "ink" },

      { from: "u-dash", to: "d-api",   tone: "slate", dashed: true },
      { from: "d-api",  to: "pl-exec", tone: "slate" },

      { from: "u-hotkey", to: "p-os",      tone: "amber", dashed: true },
      { from: "p-os",     to: "d-hotkey",  tone: "amber", dashed: true },
      { from: "d-hotkey", to: "pl-exec",   tone: "amber", dashed: true },
    ],

    // Per-kind card sizes for the compact levels. minimalW/H apply when the
    // level is "minimal" (cards collapse to label-only). descriptiveH applies
    // when the level is "descriptive" (label + sub, no inline details).
    // Detailed uses each node's source w/h from the NODES table above.
    kindStyles: {
      input:         { minimalW: 220, minimalH: 48, descriptiveH: 56 },
      platform:      { minimalW: 260, minimalH: 56, descriptiveH: 60 },
      core:          { minimalW: 300, minimalH: 48, descriptiveH: 56 },
      router:        { minimalW: 280, minimalH: 48, descriptiveH: 68 },
      api:           { minimalW: 280, minimalH: 48, descriptiveH: 68 },
      state:         { minimalW: 280, minimalH: 48, descriptiveH: 68 },
      plug:          { minimalW: 260, minimalH: 48, descriptiveH: 60 },
      "plug-anchor": { minimalW: 280, minimalH: 56, descriptiveH: 64 },
      ext:           { minimalW: 260, minimalH: 52, descriptiveH: 72 },
      "ps-file":     { minimalW: 280, minimalH: 40, descriptiveH: 44 },
      store:         { minimalW: 240, minimalH: 48, descriptiveH: 56 },
    },
  };

  return { REGIONS, NODES, EDGES, TRACES, GUTTERS, CANVAS, META };
})();
