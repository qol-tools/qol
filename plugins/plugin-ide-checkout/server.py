#!/usr/bin/env python3
"""Task Runner Daemon - Generic HTTP API for browser extensions to execute local tasks"""

import http.server
import json
import subprocess
import os
import socketserver
import tempfile
import re
from pathlib import Path

PORT = 42710
VERSION = "1.0.0"
DEFAULT_TEMP_DIR = os.path.join(tempfile.gettempdir(), 'task-runner')

CONFIG_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'config.json')

DEFAULT_CONFIG = {
    "apps": {
        "idea": {
            "name": "IntelliJ IDEA",
            "paths": [
                "/opt/homebrew/bin/idea",
                "/usr/local/bin/idea",
                "/snap/bin/idea-ultimate",
                "/snap/bin/intellij-idea-ultimate",
                os.path.expanduser("~/.local/share/JetBrains/Toolbox/scripts/idea")
            ]
        },
        "vscode": {
            "name": "VS Code",
            "paths": [
                "/usr/bin/code",
                "/opt/homebrew/bin/code",
                "/snap/bin/code",
                "/usr/local/bin/code"
            ]
        },
        "cursor": {
            "name": "Cursor",
            "paths": [
                "/opt/homebrew/bin/cursor",
                "/usr/bin/cursor",
                "/usr/local/bin/cursor",
                os.path.expanduser("~/.local/bin/cursor")
            ]
        },
        "zed": {
            "name": "Zed",
            "paths": [
                "/opt/homebrew/bin/zed",
                "/usr/bin/zed",
                os.path.expanduser("~/.local/bin/zed")
            ]
        }
    },
    "scripts": {},
    "tempDir": DEFAULT_TEMP_DIR
}

BUILTIN_ACTIONS = [
    {
        "id": "git-checkout",
        "name": "Git Checkout",
        "description": "Clone/checkout a git branch to a temp directory",
        "params": {
            "projectPath": {"type": "string", "required": True, "description": "Path to local git repo"},
            "branch": {"type": "string", "required": True, "description": "Branch name to checkout"}
        },
        "returns": {
            "tempPath": {"type": "string", "description": "Path to the checked-out temp repo"},
            "branch": {"type": "string", "description": "Branch that was checked out"}
        }
    },
    {
        "id": "open-app",
        "name": "Open Application",
        "description": "Open a file or directory in a configured application",
        "params": {
            "app": {"type": "string", "required": True, "description": "App ID from config (e.g., 'idea', 'vscode', 'cursor')"},
            "path": {"type": "string", "required": True, "description": "Path to open"}
        },
        "returns": {}
    },
    {
        "id": "run-script",
        "name": "Run Script",
        "description": "Execute a registered script with arguments",
        "params": {
            "script": {"type": "string", "required": True, "description": "Script ID from config"},
            "args": {"type": "object", "required": False, "description": "Key-value arguments passed as env vars"}
        },
        "returns": {
            "stdout": {"type": "string", "description": "Script output"},
            "stderr": {"type": "string", "description": "Script error output"},
            "exitCode": {"type": "number", "description": "Exit code"}
        }
    },
    {
        "id": "list-apps",
        "name": "List Applications",
        "description": "List configured applications and their availability",
        "params": {},
        "returns": {
            "apps": {"type": "array", "description": "List of apps with id, name, and available status"}
        }
    }
]


def load_config():
    if os.path.exists(CONFIG_PATH):
        try:
            with open(CONFIG_PATH, 'r') as f:
                user_config = json.load(f)
                merged = DEFAULT_CONFIG.copy()
                if 'apps' in user_config:
                    merged['apps'] = {**DEFAULT_CONFIG['apps'], **user_config['apps']}
                if 'scripts' in user_config:
                    merged['scripts'] = user_config['scripts']
                if 'tempDir' in user_config:
                    merged['tempDir'] = user_config['tempDir']
                return merged
        except (json.JSONDecodeError, IOError) as e:
            print(f"[task-runner] Warning: Could not load config: {e}", flush=True)
    return DEFAULT_CONFIG


def find_app_executable(app_id, config):
    apps = config.get('apps', {})
    if app_id not in apps:
        return None, f"Unknown app: {app_id}"

    app_config = apps[app_id]
    paths = app_config.get('paths', [])

    for path in paths:
        expanded = os.path.expanduser(path)
        if os.path.exists(expanded) and os.access(expanded, os.X_OK):
            return expanded, None

    return None, f"App '{app_id}' not found. Tried: {', '.join(paths)}"


def interpolate_template(value, context):
    if not isinstance(value, str):
        return value

    pattern = r'\{\{(\w+)\.(\w+)\}\}'

    def replacer(match):
        ref_id, field = match.groups()
        if ref_id in context and field in context[ref_id]:
            return str(context[ref_id][field])
        return match.group(0)

    return re.sub(pattern, replacer, value)


def interpolate_params(params, context):
    result = {}
    for key, value in params.items():
        if isinstance(value, dict):
            result[key] = interpolate_params(value, context)
        elif isinstance(value, list):
            result[key] = [interpolate_template(v, context) for v in value]
        else:
            result[key] = interpolate_template(value, context)
    return result


def validate_path(path):
    if not path:
        return False, "Path is empty"
    if '\x00' in path:
        return False, "Path contains null bytes"
    if '..' in path:
        return False, "Path contains directory traversal"
    return True, None


def action_git_checkout(params, config):
    project_path = params.get('projectPath')
    branch = params.get('branch')

    if not project_path:
        return None, "INVALID_PARAMS", "Missing required parameter: projectPath"
    if not branch:
        return None, "INVALID_PARAMS", "Missing required parameter: branch"

    valid, err = validate_path(project_path)
    if not valid:
        return None, "INVALID_PARAMS", f"Invalid projectPath: {err}"

    if not os.path.isdir(project_path):
        return None, "INVALID_PARAMS", f"Project path does not exist: {project_path}"

    print(f"[task-runner] git-checkout: {project_path} -> {branch}", flush=True)

    remote_result = subprocess.run(
        ['git', 'remote', 'get-url', 'origin'],
        cwd=project_path,
        capture_output=True,
        text=True,
        timeout=30
    )

    if remote_result.returncode != 0:
        return None, "EXECUTION_FAILED", "Could not get git remote URL"

    remote_url = remote_result.stdout.strip()
    print(f"[task-runner] Remote URL: {remote_url}", flush=True)

    temp_base = config.get('tempDir', DEFAULT_TEMP_DIR)
    repo_name = os.path.basename(project_path)
    safe_branch = re.sub(r'[^a-zA-Z0-9_-]', '-', branch)
    temp_dir_name = f"{repo_name}_{safe_branch}"
    temp_path = os.path.join(temp_base, temp_dir_name)

    os.makedirs(temp_base, exist_ok=True)

    try:
        if os.path.isdir(temp_path):
            print(f"[task-runner] Temp repo exists, fetching...", flush=True)
            subprocess.run(['git', 'fetch', '--all'], cwd=temp_path, capture_output=True, timeout=120)
            subprocess.run(['git', 'checkout', branch], cwd=temp_path, capture_output=True, timeout=30)
            subprocess.run(['git', 'pull', '--ff-only'], cwd=temp_path, capture_output=True, timeout=120)
        else:
            print(f"[task-runner] Cloning to {temp_path}...", flush=True)
            clone_result = subprocess.run(
                ['git', 'clone', '--branch', branch, '--single-branch', remote_url, temp_path],
                capture_output=True,
                text=True,
                timeout=300
            )

            if clone_result.returncode != 0:
                print(f"[task-runner] Single-branch clone failed, trying full clone...", flush=True)
                subprocess.run(
                    ['git', 'clone', remote_url, temp_path],
                    capture_output=True,
                    text=True,
                    timeout=300
                )
                subprocess.run(
                    ['git', 'checkout', branch],
                    cwd=temp_path,
                    capture_output=True,
                    text=True,
                    timeout=30
                )
    except subprocess.TimeoutExpired:
        return None, "TIMEOUT", "Git operation timed out"

    print(f"[task-runner] Temp repo ready at {temp_path}", flush=True)

    return {"tempPath": temp_path, "branch": branch}, None, None


def action_open_app(params, config):
    app_id = params.get('app')
    path = params.get('path')

    if not app_id:
        return None, "INVALID_PARAMS", "Missing required parameter: app"
    if not path:
        return None, "INVALID_PARAMS", "Missing required parameter: path"

    valid, err = validate_path(path)
    if not valid:
        return None, "INVALID_PARAMS", f"Invalid path: {err}"

    executable, err = find_app_executable(app_id, config)
    if not executable:
        return None, "APP_NOT_FOUND", err

    print(f"[task-runner] open-app: {app_id} ({executable}) -> {path}", flush=True)

    try:
        subprocess.Popen([executable, path])
    except Exception as e:
        return None, "EXECUTION_FAILED", f"Failed to launch app: {e}"

    return {}, None, None


def action_run_script(params, config):
    script_id = params.get('script')
    args = params.get('args', {})

    if not script_id:
        return None, "INVALID_PARAMS", "Missing required parameter: script"

    scripts = config.get('scripts', {})
    if script_id not in scripts:
        return None, "SCRIPT_NOT_FOUND", f"Unknown script: {script_id}"

    script_config = scripts[script_id]
    command = script_config.get('command')
    cwd = script_config.get('cwd', os.getcwd())
    timeout = script_config.get('timeout', 60)

    if not command:
        return None, "INVALID_PARAMS", f"Script '{script_id}' has no command"

    for key, value in args.items():
        cwd = cwd.replace(f"{{{{params.{key}}}}}", str(value))
        command = command.replace(f"{{{{params.{key}}}}}", str(value))

    env = os.environ.copy()
    for key, value in args.items():
        env[f"TASK_ARG_{key.upper()}"] = str(value)

    print(f"[task-runner] run-script: {script_id} ({command}) in {cwd}", flush=True)

    try:
        result = subprocess.run(
            command,
            shell=True,
            cwd=cwd,
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env
        )
        return {
            "stdout": result.stdout,
            "stderr": result.stderr,
            "exitCode": result.returncode
        }, None, None
    except subprocess.TimeoutExpired:
        return None, "TIMEOUT", f"Script exceeded {timeout}s timeout"
    except Exception as e:
        return None, "EXECUTION_FAILED", f"Script execution failed: {e}"


def action_list_apps(params, config):
    apps = config.get('apps', {})
    result = []

    for app_id, app_config in apps.items():
        executable, _ = find_app_executable(app_id, config)
        result.append({
            "id": app_id,
            "name": app_config.get('name', app_id),
            "available": executable is not None,
            "path": executable
        })

    return {"apps": result}, None, None


ACTION_HANDLERS = {
    "git-checkout": action_git_checkout,
    "open-app": action_open_app,
    "run-script": action_run_script,
    "list-apps": action_list_apps
}


def execute_action(action_id, params, config, context=None):
    if context is None:
        context = {}

    interpolated_params = interpolate_params(params, context)

    if action_id not in ACTION_HANDLERS:
        return None, "INVALID_ACTION", f"Unknown action: {action_id}"

    handler = ACTION_HANDLERS[action_id]
    return handler(interpolated_params, config)


class TaskRunnerHandler(http.server.BaseHTTPRequestHandler):
    config = None

    def log_message(self, format, *args):
        print(f"[task-runner] {args[0]}", flush=True)

    def send_cors_headers(self):
        self.send_header('Access-Control-Allow-Origin', '*')
        self.send_header('Access-Control-Allow-Methods', 'GET, POST, OPTIONS')
        self.send_header('Access-Control-Allow-Headers', 'Content-Type')

    def send_json(self, code, data):
        self.send_response(code)
        self.send_header('Content-Type', 'application/json')
        self.send_cors_headers()
        self.end_headers()
        self.wfile.write(json.dumps(data).encode())

    def do_OPTIONS(self):
        self.send_response(200)
        self.send_cors_headers()
        self.end_headers()

    def do_GET(self):
        if self.path == '/health':
            self.send_json(200, {'status': 'ok', 'version': VERSION})
        elif self.path == '/actions':
            self.send_json(200, {'actions': BUILTIN_ACTIONS})
        else:
            self.send_json(404, {'error': 'Not found'})

    def do_POST(self):
        content_length = int(self.headers.get('Content-Length', 0))
        body = self.rfile.read(content_length).decode('utf-8')

        try:
            data = json.loads(body)
        except json.JSONDecodeError:
            self.send_json(400, {'success': False, 'error': {'code': 'INVALID_JSON', 'message': 'Invalid JSON'}})
            return

        if self.path == '/checkout':
            self.handle_legacy_checkout(data)
        elif self.path == '/execute':
            self.handle_execute(data)
        else:
            self.send_json(404, {'error': 'Not found'})

    def handle_legacy_checkout(self, data):
        project_path = data.get('projectPath')
        branch = data.get('branch')

        result, err_code, err_msg = action_git_checkout(
            {'projectPath': project_path, 'branch': branch},
            self.config
        )

        if err_code:
            self.send_json(400 if err_code == 'INVALID_PARAMS' else 500, {'error': err_msg})
            return

        app_id = data.get('app', 'idea')
        open_result, _, _ = action_open_app({'app': app_id, 'path': result['tempPath']}, self.config)

        self.send_json(200, {
            'success': True,
            'branch': result['branch'],
            'tempPath': result['tempPath'],
            'projectPath': project_path
        })

    def handle_execute(self, data):
        if 'chain' in data:
            self.handle_chain_execute(data['chain'])
        elif 'action' in data:
            self.handle_single_execute(data)
        else:
            self.send_json(400, {
                'success': False,
                'error': {'code': 'INVALID_REQUEST', 'message': 'Missing action or chain'}
            })

    def handle_single_execute(self, data):
        action_id = data.get('action')
        params = data.get('params', {})

        result, err_code, err_msg = execute_action(action_id, params, self.config)

        if err_code:
            status = 400 if err_code in ('INVALID_PARAMS', 'INVALID_ACTION') else 500
            self.send_json(status, {
                'success': False,
                'error': {'code': err_code, 'message': err_msg}
            })
            return

        self.send_json(200, {
            'success': True,
            'action': action_id,
            'result': result
        })

    def handle_chain_execute(self, chain):
        context = {}
        results = {}

        for step in chain:
            step_id = step.get('id', step.get('action'))
            action_id = step.get('action')
            params = step.get('params', {})

            result, err_code, err_msg = execute_action(action_id, params, self.config, context)

            if err_code:
                status = 400 if err_code in ('INVALID_PARAMS', 'INVALID_ACTION') else 500
                self.send_json(status, {
                    'success': False,
                    'failedStep': step_id,
                    'error': {'code': err_code, 'message': err_msg},
                    'completedResults': results
                })
                return

            context[step_id] = result
            results[step_id] = result

        self.send_json(200, {
            'success': True,
            'results': results
        })


class ReuseAddrServer(socketserver.TCPServer):
    allow_reuse_address = True


if __name__ == '__main__':
    config = load_config()
    TaskRunnerHandler.config = config

    print(f"[task-runner] Starting daemon v{VERSION} on port {PORT}", flush=True)
    print(f"[task-runner] Temp directory: {config.get('tempDir', DEFAULT_TEMP_DIR)}", flush=True)
    print(f"[task-runner] Config: {CONFIG_PATH}", flush=True)

    with ReuseAddrServer(('127.0.0.1', PORT), TaskRunnerHandler) as httpd:
        httpd.serve_forever()
