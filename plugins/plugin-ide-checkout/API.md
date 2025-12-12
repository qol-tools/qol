# Task Runner API Contract v1.0

A generic HTTP API standard for browser extensions to execute arbitrary tasks via a local daemon.

## Overview

The Task Runner daemon exposes a REST-like API on `127.0.0.1:{PORT}`. Browser extensions can:
1. **Discover** available actions via `GET /actions`
2. **Execute** tasks via `POST /execute`
3. **Check** daemon health via `GET /health`

## Endpoints

### GET /health

Health check endpoint.

**Response (200):**
```json
{
  "status": "ok",
  "version": "1.0.0"
}
```

### GET /actions

Discover available actions and their schemas.

**Response (200):**
```json
{
  "actions": [
    {
      "id": "git-checkout",
      "name": "Git Checkout",
      "description": "Clone/checkout a git branch to a temp directory",
      "params": {
        "projectPath": { "type": "string", "required": true, "description": "Path to local git repo" },
        "branch": { "type": "string", "required": true, "description": "Branch name to checkout" }
      },
      "returns": {
        "tempPath": { "type": "string", "description": "Path to the checked-out temp repo" }
      }
    },
    {
      "id": "open-app",
      "name": "Open Application",
      "description": "Open a file or directory in a configured application",
      "params": {
        "app": { "type": "string", "required": true, "description": "App ID from config (e.g., 'idea', 'vscode', 'cursor')" },
        "path": { "type": "string", "required": true, "description": "Path to open" }
      },
      "returns": {}
    },
    {
      "id": "run-script",
      "name": "Run Script",
      "description": "Execute a registered script with arguments",
      "params": {
        "script": { "type": "string", "required": true, "description": "Script ID from config" },
        "args": { "type": "object", "required": false, "description": "Key-value arguments passed as env vars" }
      },
      "returns": {
        "stdout": { "type": "string", "description": "Script output" },
        "exitCode": { "type": "number", "description": "Exit code" }
      }
    }
  ]
}
```

### POST /execute

Execute an action.

**Request:**
```json
{
  "action": "git-checkout",
  "params": {
    "projectPath": "/path/to/repo",
    "branch": "feature/foo"
  }
}
```

**Response (200):**
```json
{
  "success": true,
  "action": "git-checkout",
  "result": {
    "tempPath": "/tmp/task-runner/repo_feature-foo"
  }
}
```

**Error Response (4xx/5xx):**
```json
{
  "success": false,
  "error": {
    "code": "INVALID_PARAMS",
    "message": "Missing required parameter: projectPath"
  }
}
```

### POST /execute (Chained Actions)

Execute multiple actions in sequence. Each action can reference results from previous actions.

**Request:**
```json
{
  "chain": [
    {
      "id": "checkout",
      "action": "git-checkout",
      "params": {
        "projectPath": "/path/to/repo",
        "branch": "feature/foo"
      }
    },
    {
      "id": "open",
      "action": "open-app",
      "params": {
        "app": "idea",
        "path": "{{checkout.tempPath}}"
      }
    }
  ]
}
```

**Response (200):**
```json
{
  "success": true,
  "results": {
    "checkout": { "tempPath": "/tmp/task-runner/repo_feature-foo" },
    "open": {}
  }
}
```

## Configuration (config.json)

```json
{
  "apps": {
    "idea": {
      "name": "IntelliJ IDEA",
      "paths": [
        "/opt/homebrew/bin/idea",
        "/usr/local/bin/idea",
        "/snap/bin/idea-ultimate",
        "~/.local/share/JetBrains/Toolbox/scripts/idea"
      ]
    },
    "vscode": {
      "name": "VS Code",
      "paths": [
        "/usr/bin/code",
        "/opt/homebrew/bin/code",
        "/snap/bin/code"
      ]
    },
    "cursor": {
      "name": "Cursor",
      "paths": [
        "/opt/homebrew/bin/cursor",
        "/usr/bin/cursor"
      ]
    }
  },
  "scripts": {
    "build": {
      "name": "Build Project",
      "command": "./build.sh",
      "cwd": "{{params.path}}",
      "timeout": 300
    }
  },
  "tempDir": "/tmp/task-runner"
}
```

## Error Codes

| Code | Description |
|------|-------------|
| `INVALID_ACTION` | Action ID not found |
| `INVALID_PARAMS` | Missing or invalid parameters |
| `APP_NOT_FOUND` | Configured app not found on system |
| `SCRIPT_NOT_FOUND` | Script ID not registered |
| `EXECUTION_FAILED` | Action execution failed |
| `TIMEOUT` | Action exceeded timeout |

## Security Considerations

1. **Localhost only**: Daemon binds to 127.0.0.1, not 0.0.0.0
2. **Path validation**: All paths validated to prevent traversal attacks
3. **No shell injection**: Commands are executed with explicit args, not shell interpolation
4. **Script allowlist**: Only registered scripts can be executed
5. **CORS**: Configurable allowed origins (default: localhost extensions only)

## Built-in Actions

### git-checkout

Clones a git repository to a temp directory and checks out a specific branch.

**Behavior:**
1. Gets remote URL from `projectPath`
2. Creates temp dir: `{tempDir}/{repoName}_{safeBranch}`
3. If exists: fetch + checkout + pull
4. If new: clone with branch

### open-app

Opens a path in a configured application.

**Behavior:**
1. Looks up `app` in config
2. Tries each path in order until one exists
3. Spawns the app with `path` as argument

### run-script

Executes a registered script.

**Behavior:**
1. Looks up `script` in config
2. Expands `{{params.x}}` in command and cwd
3. Sets `args` as environment variables
4. Runs with timeout, captures output

## Browser Extension Integration Example

```javascript
const DAEMON_URL = 'http://127.0.0.1:42710';

async function checkoutAndOpen(projectPath, branch, ide = 'idea') {
  const response = await fetch(`${DAEMON_URL}/execute`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      chain: [
        {
          id: 'checkout',
          action: 'git-checkout',
          params: { projectPath, branch }
        },
        {
          id: 'open',
          action: 'open-app',
          params: { app: ide, path: '{{checkout.tempPath}}' }
        }
      ]
    })
  });

  return response.json();
}

async function getAvailableIDEs() {
  const response = await fetch(`${DAEMON_URL}/actions`);
  const { actions } = await response.json();
  const openApp = actions.find(a => a.id === 'open-app');
  // Extension can now show dropdown of available apps
}
```
