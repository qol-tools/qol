# qol-platform

Platform detection and capability reporting for qol-tools. Detects the Linux display backend (X11, Wayland, or unknown) and reports what features are available on the current platform.

## Usage

```rust
use qol_platform::{current_capabilities, linux_display_backend, LinuxDisplayBackend};

let caps = current_capabilities();
if caps.can_global_hotkey {
    // register hotkeys
}

if linux_display_backend() == LinuxDisplayBackend::Wayland {
    // wayland-specific path
}
```

On macOS, all capabilities are reported as available. On Linux, it depends on the display backend.

`launch_working_dir()` returns a sensible working directory for spawning child processes (home dir, falling back to cwd).

## License

PolyForm Noncommercial 1.0.0
