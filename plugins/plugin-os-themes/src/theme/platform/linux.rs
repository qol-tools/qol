// NOTE: This is an extremely ambitious feature area.
// OS-wide theming on Linux has no unified API — GTK, Qt, KDE, GNOME, icon themes,
// cursor themes, and window manager decorations all have separate mechanisms.
// No implementation is planned in the near future. This module exists to hold
// the platform abstraction boundary so the strategy pattern is in place when
// the time comes.
//
// Likely future entry points:
//   - GTK: ~/.config/gtk-3.0/settings.ini + gsettings org.gnome.desktop.interface
//   - Qt:  ~/.config/qt5ct/qt5ct.conf
//   - Icons: ~/.local/share/icons / ~/.icons/default/index.theme
//   - Cursors: see cursor/platform/linux.rs (first implementation)
