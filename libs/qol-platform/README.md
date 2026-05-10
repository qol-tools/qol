# qol-platform

[![tests](https://github.com/qol-tools/qol-platform/actions/workflows/tests.yml/badge.svg)](https://github.com/qol-tools/qol-platform/actions/workflows/tests.yml)
[![lint](https://github.com/qol-tools/qol-platform/actions/workflows/lint.yml/badge.svg)](https://github.com/qol-tools/qol-platform/actions/workflows/lint.yml)

Platform detection and capability reporting for qol-tools.

## Quick start

```toml
[dependencies]
qol-platform = { git = "https://github.com/qol-tools/qol-platform" }
```

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

## License

PolyForm Noncommercial 1.0.0
