# QoL Runtime

Protocol types and client for the QoL Tray platform state server. Used by [qol-plugin-api](https://github.com/qol-tools/qol-plugin-api) to give plugins access to system state.

## What It Does

QoL Tray runs a local socket server that tracks the active monitor, cursor position, and focused window. This crate provides:

- **Protocol types** — request/response types for the state socket
- **Client** — connect and query platform state from plugin processes
- **Subscriptions** — watch for state changes in real time

## Usage

Most plugins should depend on `qol-plugin-api` instead of this crate directly. Use `qol-runtime` only if you need low-level access to the socket protocol.

```toml
[dependencies]
qol-runtime = { git = "https://github.com/qol-tools/qol-runtime" }
```

License: MIT
