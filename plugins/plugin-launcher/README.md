# plugin-launcher

A [qol-tray](https://github.com/qol-tools/qol-tray) plugin that provides a fast, keyboard-driven application and file launcher. Built with [GPUI](https://github.com/zed-industries/zed).

## Features

- **App search** — finds installed applications (`.desktop` entries on Linux, `.app` bundles on macOS)
- **File search** — indexes common directories with fuzzy matching
- **Adjustable fuzziness** — strict, balanced, or loose matching
- **Daemon mode** — stays resident for instant popup via hotkey

## Platforms

Linux, macOS

## Controls

| Key | Action |
|-----|--------|
| Type | Fuzzy search |
| Tab / Shift+Tab | Switch mode (Apps / Files) |
| Ctrl+Up / Ctrl+Down | Adjust fuzziness |
| Up / Down | Navigate results |
| Enter | Launch selected |
| Esc | Dismiss |

## Development

```bash
make dev      # build and install binary to plugin root
make test     # run tests
```

## License

MIT
