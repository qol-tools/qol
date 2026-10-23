<div align="center">

# Input

Set how the mouse, touchpad and keyboard respond.

</div>

## Quick start

```sh
qol-input status
qol-input set pointer-speed 60
qol-input set acceleration off
qol-input set tap-to-click on
qol-input set repeat-delay 300
qol-input toggle-touchpad
qol-input doctor
```

Input works from a terminal on its own. qol-tray is one way to use it, not a requirement.

It changes the desktop's own settings, so the desktop's settings app shows the same values. On Linux it works on Cinnamon and GNOME.

When qol is not set to stay on this computer, Input saves each setting before its first change and puts it back when qol-tray exits, or when qol next starts if it did not exit cleanly.

On Windows:

- The touchpad rows need Windows 11 24H2 or later and a Precision Touchpad.
- Turning the touchpad on or off presses Windows' own touchpad shortcut, Ctrl + Win + F24.
- A mouse has no natural scrolling setting.
- Pointer and touchpad speed move in steps of 5%, the repeat delay is 250, 500, 750 or 1000 ms, and keys repeat 3 to 30 times a second. Other values are refused.

On macOS:

- Changes apply at once, through the same preferences System Settings writes.
- The touchpad cannot be turned off, and a mouse has no button swap setting.
- Natural scrolling is one setting for the mouse and the touchpad, so changing either row changes both.
- The repeat delay moves in steps of 50 ms, and keys repeat 2, 3, 4, 5, 6, 10, 12, 15, 20 or 30 times a second. Other values are refused.

## Settings

- Mouse sets pointer speed, acceleration, swapped buttons, double-click time and natural scrolling.
- Touchpad turns the touchpad on or off and sets tap to click, natural scrolling and pointer speed. Its rows read unavailable when no touchpad is connected.
- Keyboard sets how long a held key waits before it repeats, and how fast it repeats.

A setting this computer cannot change reads unavailable, and its row says why in place of its description: on macOS, the touchpad switch and swapped mouse buttons; on Windows, natural scrolling for a mouse; with no touchpad, every touchpad row.

## Hotkeys

- Toggle Touchpad turns the touchpad off when it is on, and on when it is off.
- Turn Touchpad On and Turn Touchpad Off set it either way.

## License

[PolyForm Noncommercial 1.0.0](LICENSE)
