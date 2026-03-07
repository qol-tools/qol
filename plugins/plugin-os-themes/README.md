# plugin-os-themes

A [qol-tray](https://github.com/qol-tools/qol-tray) plugin for OS-wide theming. GTK, Qt, icons, cursors, and more.

OS theming on Linux has no unified API. This plugin aims to be the single place to manage it all from qol-tray. The scope is broad; implementation is incremental.

## Current Features

**Shake-to-grow:** shake your cursor to temporarily scale it up, then it smoothly animates back to normal.

## Planned

- GTK theme switching
- Qt theme switching
- Icon theme management
- Cursor theme management
- Wayland support (deferred)

## Build

- `make dev` builds and installs to the plugin root
- `make release` optimized build

## Configuration

Settings are editable via the qol-tray UI under OS Themes.

## License

MIT
