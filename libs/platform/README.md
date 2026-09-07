<div align="center">

# QoL Platform

Platform detection and capability reporting for qol-tools.

</div>

## Quick start

```toml
[dependencies]
qol-platform.workspace = true
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
