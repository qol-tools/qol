# Task Runner

A `qol-tray` plugin that exposes a local HTTP API for browser extensions to execute local tasks.

## Development

```bash
# Run contract validation tests
cargo test

# Run in development mode (as a tray plugin)
# qol-tray will automatically resolve the binary from target/debug
```

## Runtime Contract

- Runtime command: `task-runner status`
- Daemon command: `task-runner` (defaults to daemon mode)
- Daemon implementation: `server.py` executed via `python3`

### Quick Examples

```bash
curl http://127.0.0.1:42710/health

curl http://127.0.0.1:42710/actions

curl -X POST http://127.0.0.1:42710/execute \
  -H "Content-Type: application/json" \
  -d '{"action": "list-apps", "params": {}}'

curl -X POST http://127.0.0.1:42710/execute \
  -H "Content-Type: application/json" \
  -d '{"action": "open-app", "params": {"app": "vscode", "path": "/path/to/project"}}'

curl -X POST http://127.0.0.1:42710/execute \
  -H "Content-Type: application/json" \
  -d '{
    "chain": [
      {"id": "checkout", "action": "git-checkout", "params": {"projectPath": "/path/to/repo", "branch": "main"}},
      {"id": "open", "action": "open-app", "params": {"app": "idea", "path": "{{checkout.tempPath}}"}}
    ]
  }'
```

## Configuration

Edit `config.json` to add custom apps or scripts.

## Dependencies

- Rust toolchain (for building `task-runner`)
- Python 3.6+ (for daemon runtime)
- Git (for `git-checkout` action)

## License

MIT
