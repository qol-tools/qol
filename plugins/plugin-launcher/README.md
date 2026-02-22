# Launcher Plugin for QoL Tray

A high-performance, keyboard-driven application and file launcher for [QoL Tray](https://github.com/qol-tools/qol-tray). Built with the GPUI framework for fluid transitions and sub-millisecond responsiveness.

## Key Features

- **Blazing Fast Search**: Uses a multi-pass fuzzy matching algorithm (Boundary, Contiguous, and Greedy passes) to find what you're looking for with minimal keystrokes.
- **Frecency-based Ranking**: Automatically learns your habits. Search results are ranked by a combination of frequency and recency, ensuring your most-used items are always at the top.
- **Intelligent Indexing**:
  - **Applications**: Scans standard desktop entries (XDG), Flatpaks, and Snaps on Linux; standard `.app` bundles on macOS.
  - **Files**: Automatically indexes `Desktop`, `Documents`, `Downloads`, `Projects`, and `.config` directories.
- **Action Modifiers**: perform different actions on a result without leaving the keyboard:
  - `Enter`: Launch/Open.
  - `Ctrl + Enter`: Open in Terminal.
  - `Shift + Enter`: Open containing folder.
  - `Alt + Enter`: Copy absolute path to clipboard.
- **Monitor-Aware**: Intelligent positioning that follows your active focus across multi-monitor setups.
- **Daemon Architecture**: Stays resident in memory via Unix sockets for near-instant popup via global hotkeys.

## Controls

| Key | Action |
|-----|--------|
| **Character Keys** | Continuous fuzzy search |
| **Tab / Shift+Tab** | Toggle between **Apps** and **Files** modes |
| **Up / Down** | Navigate through search results |
| **Enter** | Launch/Open the selection |
| **Esc** | Dismiss the launcher |

## Development

```bash
# Run contract validation tests
cargo test

# Run in development mode (as a tray plugin)
# qol-tray will automatically resolve the binary from target/debug
```

## Internal Architecture

- **Providers**: Pluggable indexing system for different OS backends.
- **Fuzzy Matching**: Custom scoring weights for boundary matches and contiguous segments.
- **Frecency**: Exponential decay scoring to balance long-term usage with recent activity.
- **Rendering**: Hardware-accelerated 2D rendering via GPUI.

License: MIT

