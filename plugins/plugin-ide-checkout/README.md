# Task Runner

A qol-tray plugin that provides a generic HTTP API for browser extensions to execute local tasks.

## Features

- **Action Discovery**: Extensions call `GET /actions` to learn available actions
- **Multi-IDE Support**: Configure any app with fallback paths (idea, vscode, cursor, zed)
- **Chained Execution**: Run multiple actions in sequence with result interpolation
- **Script Registration**: Register custom scripts in config.json

## API

See [API.md](API.md) for the full API documentation.

### Quick Examples

```bash
# Health check
curl http://127.0.0.1:42710/health

# List available actions
curl http://127.0.0.1:42710/actions

# List available apps
curl -X POST http://127.0.0.1:42710/execute \
  -H "Content-Type: application/json" \
  -d '{"action": "list-apps", "params": {}}'

# Open a project in VS Code
curl -X POST http://127.0.0.1:42710/execute \
  -H "Content-Type: application/json" \
  -d '{"action": "open-app", "params": {"app": "vscode", "path": "/path/to/project"}}'

# Checkout branch and open in IDE
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

Edit `config.json` to add custom apps or scripts:

```json
{
  "apps": {
    "myide": {
      "name": "My IDE",
      "paths": ["/usr/bin/myide", "~/.local/bin/myide"]
    }
  },
  "scripts": {
    "build": {
      "name": "Build Project",
      "command": "make build",
      "cwd": "{{params.path}}",
      "timeout": 300
    }
  },
  "tempDir": "/tmp/task-runner"
}
```

## Dependencies

- Python 3.6+
- Git (for git-checkout action)

## License

MIT
