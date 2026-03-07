# Agent Handoff — plugin-os-themes

## Vision

This plugin is inspired by macOS's shake-to-grow cursor feature — shake the mouse
and the cursor temporarily grows so you can find it. The goal is to bring that to
Linux, and eventually expand into broader OS theming (cursor packs, GTK/Qt themes,
icon themes). The scope is deliberately ambitious, and **no one expects it to be
done quickly**. Build incrementally and correctly.

The plugin lives at: `/media/kmrh47/WD_SN850X/Git/qol-tools/plugin-os-themes`
It is a qol-tray plugin. Understand qol-tray conventions before touching anything.
Reference repo: `/media/kmrh47/WD_SN850X/Git/qol-tools/qol-tray`
Reference template: `/media/kmrh47/WD_SN850X/Git/qol-tools/plugin-template`

---

## Architecture Decisions (already made — do not revisit)

**Strategy pattern** is the core design. Each feature domain (`cursor/`, `theme/`)
owns a trait and owns its platform abstraction internally:

```
src/
  main.rs
  cursor/
    mod.rs              — CursorEffect trait
    platform/
      mod.rs
      linux.rs          — X11 implementation (stub)
  theme/
    mod.rs              — ThemeStrategy trait
    platform/
      mod.rs
      linux.rs          — placeholder, see notes inside
```

This mirrors how qol-tray structures its own modules. Platform abstraction lives
inside the feature module, not at the root. Adding macOS support later means adding
`macos.rs` inside the relevant `platform/` — no restructuring needed.

---

## Current State

- Scaffold only. Nothing is implemented.
- `cursor/platform/linux.rs` has a `todo!()` with detailed notes on the X11 API
  surface needed (XFixesGetCursorImage, XcursorImageCreate, XQueryPointer).
- `theme/platform/linux.rs` has a large comment explaining why it's empty and what
  the future entry points are (gsettings, gtk settings.ini, qt5ct, icon themes).
- The plugin compiles but `run()` panics with `todo!()`.
- `plugin.toml` is wired up: action `run` → cursor grow, action `settings` → opens
  the plugin settings URL.

---

## The Only Goal Right Now: shake-to-grow (X11)

Everything else in this repo (`theme/`, the roadmap section) is scaffolding that
exists so the architecture is in place later. Do not touch it. Do not implement it.
The one and only deliverable is shake-to-grow working on X11.

## shake-to-grow (X11)

This is the first real feature. Implement it in `src/cursor/platform/linux.rs`.

### Algorithm

1. Connect to X11 display (`XOpenDisplay`)
2. Poll `XQueryPointer` in a loop (~60Hz is fine)
3. Accumulate pointer velocity over a rolling window (~150ms)
4. When velocity exceeds threshold (tune empirically, ~3000 px/s is a starting point):
   - Load a scaled cursor via `XcursorLibraryLoadCursor` or by scaling the current
     cursor image from `XFixesGetCursorImage`
   - Set it with `XFixesSetCursor` or `XDefineCursor`
5. After ~600ms of low velocity, restore the original cursor
6. Loop forever; handle SIGTERM/SIGINT for clean teardown

### Dependencies to add to Cargo.toml

```toml
[target.'cfg(target_os = "linux")'.dependencies]
x11 = { version = "2", features = ["xlib", "xfixes", "xcursor"] }
```

The `x11` crate provides raw bindings. Alternatively `x11rb` is a safer higher-level
option but has more boilerplate for cursor manipulation.

### Key X11 types/functions

- `XOpenDisplay(null)` — connect to display
- `XQueryPointer` — sample cursor position each tick
- `XFixesGetCursorImage` — get the current cursor as pixel data (requires XFixes extension)
- `XcursorImageCreate(w, h)` / `XcursorImageDestroy` — create scaled cursor image
- `XcursorImageLoadCursor(display, image)` — get a `Cursor` handle from image
- `XDefineCursor(display, window, cursor)` — set cursor on root window
- `XFreeCursor` — release cursor handle when restoring

### Wayland

Do not attempt Wayland support. There is no compositor-agnostic cursor override API.
KDE and wlroots compositors have separate DBus/protocol interfaces that diverge
significantly. This can be revisited years from now.

---

## Coding Conventions

Read `/media/kmrh47/WD_SN850X/Git/CLAUDE.md` — those rules apply here too.
Key points relevant to this plugin:

- No comments in code (the handoff doc is different — that's docs, not code comments)
- Conventional commits: `feat:`, `fix:`, `refactor:` etc.
- No dead code; gate unfinished things behind `todo!()` or feature flags
- Max 50 lines per function; single responsibility
- Exhaustive match arms — no `_ =>`
- No builds or tests unless explicitly asked

---

## Future Roadmap (do not implement, just context)

Once shake-to-grow works:

1. **Cursor themes** — switch the active Xcursor theme by writing
   `~/.icons/default/index.theme` and broadcasting a theme change event
   (`XChangeProperty` on root window with `XCURSOR_THEME` atom, or via XSETTINGS)

2. **OS-wide theming** — see `theme/platform/linux.rs` for the entry points.
   GTK settings, Qt5ct config, icon themes. Each gets its own `ThemeStrategy` impl.
   This is very ambitious; treat each as a separate feature branch.

The strategy pattern means each new theme type is a new struct implementing
`ThemeStrategy` — `apply()` writes the config, `revert()` restores it.
