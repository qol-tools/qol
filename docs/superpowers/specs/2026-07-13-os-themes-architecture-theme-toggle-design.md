# plugin-os-themes: Linux architecture seams + light/dark toggle (v1)

## Goal

Restructure plugin-os-themes so the two real Linux expansion axes (desktop environment, display server) have explicit seams, then ship a first theme feature on top: a light/dark toggle.
"Linux" stops being a leaf; it becomes a directory with runtime-selected backends.

## Non-goals

Wayland implementation (stub only), named theme profiles, scheduled switching, icon/wallpaper theming, Qt theming.
No behavior change to shake-to-grow.

## Architecture

Two levels of strategy, different selection mechanisms:

1. OS level stays the compile-time `platform/` pattern required by qol-arch-code (cfg-aliased `Platform` per OS, stubs return typed errors).
2. Linux-internal level is runtime-selected, because DE and display server are runtime facts of the machine, not compile targets.
   All Linux backends compile on Linux; detection picks one.

### Target layout

```
src/
  theme/
    mod.rs                    public API: ColorScheme { Light, Dark }, toggle(), set(scheme)
    platform/
      mod.rs                  ThemePlatform trait (unchanged wiring), OS stubs
      linux/
        mod.rs                detect_backend() via XDG_CURRENT_DESKTOP -> Box<dyn DesktopBackend>
        backends/
          mod.rs              trait DesktopBackend { current_scheme(), apply(scheme) }
          cinnamon.rs         gsettings org.cinnamon.* (implemented + verified on this machine)
          gnome.rs            gsettings org.gnome.desktop.interface (best-effort)
          kde.rs              typed Err stub
      macos.rs / windows.rs   existing stubs
  cursor/
    platform/
      linux/
        motion.rs             display-agnostic (pure math), stays
        scale.rs              display-agnostic, stays
        runtime.rs            stays
        display/
          mod.rs              detect via XDG_SESSION_TYPE -> x11 | wayland stub
          x11/                current x11.rs split move-only into:
            session.rs        connection + root/children cursor define/restore lifecycle
            sampling.rs       XFixes capture, hashing, cursor identity
            source.rs         named lookup, base match, ShapeCatalog
            animation.rs      frame scaling, thinning, XcursorImages build
          wayland.rs          typed Err stub
      macos.rs / windows.rs   existing stubs
```

### Backend selection

`detect_backend()` parses `XDG_CURRENT_DESKTOP` (colon-separated list, case-insensitive): contains `X-Cinnamon`/`Cinnamon` -> cinnamon, `GNOME` -> gnome, `KDE` -> kde, else typed Err naming the detected value.
Display detection: `XDG_SESSION_TYPE` = `x11` -> x11 backend, `wayland` -> stub Err, else fall back to trying X11 (matches current behavior of assuming X).

### Cinnamon backend (v1 implemented)

Reads and writes theme names through `gsettings` (spawned process, not a GIO dependency).
Exact schema keys are verified on-machine during implementation (`gsettings list-recursively | grep -i theme`); expected surface: `org.cinnamon.desktop.interface gtk-theme`, `org.cinnamon.theme name`, icon theme key left untouched in v1.
Scheme resolution: config fields name the light and dark GTK theme explicitly; empty fields fall back to a suffix heuristic (append/strip `-Dark`, verified against installed themes is out of scope - the heuristic just swaps names).
`current_scheme()` classifies the active gtk-theme by matching it against the configured/derived dark name; used by `toggle()`.

### Plugin contract changes

- `plugin.toml`: new `[action.toggle-theme]` with `label = "Toggle Light/Dark"`, `args = ["toggle-theme"]`.
- Because `daemon.command == runtime.command`, the tray routes every action to the socket; the daemon dispatcher must handle `toggle-theme` in-process (see memory: single-binary daemon must handle all actions).
- `qol-config.toml`: new `[section.theme]` with `gtk_theme_light` and `gtk_theme_dark` string fields, default empty (heuristic mode).
- `src/config.rs`: extend Config + contract test.

## Error handling

Unsupported DE, missing gsettings binary, or Wayland session return typed `anyhow` errors; the host surfaces them as toasts.
No panics, no silent fallbacks: a failed `gsettings set` propagates.

## Testing

- Backend-agnostic logic (XDG parsing, scheme classification, suffix heuristic) gets table-driven unit tests.
- x11 split is move-only; regression = existing 8 motion tests plus Xephyr rig runs (detector-feel, lowres-repro, regrow-repro, anim-repro) must match pre-refactor output.
- Cinnamon backend verified live on this machine: run `toggle-theme` twice, observe desktop flip and gsettings values.
- Contract test updated for new config fields; full gate: fmt, clippy -D warnings, build, test.

## Sequencing

1. Move-only refactor: `x11.rs` -> `display/x11/` modules + wayland stub + display detection. Commit.
2. Theme backend scaffolding: `linux/backends/` trait + detection + stubs. Commit.
3. Cinnamon backend + config fields + `toggle-theme` action + daemon dispatch. Commit.
4. Live verification on Cinnamon, then GNOME backend best-effort (untested locally, keys per upstream docs). Commit.
