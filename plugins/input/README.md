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

On Windows:

- The touchpad rows need Windows 11 24H2 or later.
- Turning the touchpad on or off presses Windows' own touchpad shortcut, Ctrl + Win + F24.
- A mouse has no natural scrolling setting.
- Pointer and touchpad speed move in steps of 5%, the repeat delay is 250, 500, 750 or 1000 ms, and keys repeat 3 to 30 times a second. Other values are refused.

## Settings

- Mouse sets pointer speed, acceleration, swapped buttons, double-click time and natural scrolling.
- Touchpad turns the touchpad on or off and sets tap to click, natural scrolling and pointer speed. Its rows read unavailable when no touchpad is connected.
- Keyboard sets how long a held key waits before it repeats, and how fast it repeats.

## Hotkeys

- Toggle Touchpad turns the touchpad off when it is on, and on when it is off.
- Turn Touchpad On and Turn Touchpad Off set it either way.

## License

[PolyForm Noncommercial 1.0.0](LICENSE)
