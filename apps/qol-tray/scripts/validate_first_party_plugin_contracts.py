#!/usr/bin/env python3
import argparse
import pathlib
import sys
import tomllib


def is_valid_action_id(value: str) -> bool:
    if not value or len(value) > 64 or value.startswith("-"):
        return False
    return all(char.isascii() and (char.isalnum() or char in "-_") for char in value)


def is_valid_command_name(value: str) -> bool:
    if not value:
        return False
    if value.strip() != value:
        return False
    if "\0" in value:
        return False
    if value.startswith("-"):
        return False
    if value.endswith(".sh"):
        return False
    if len(value) > 64:
        return False
    return all(char.isascii() and (char.isalnum() or char in "-_") for char in value)


def validate_socket(value: str) -> list[str]:
    errors: list[str] = []
    if not value:
        return ["daemon.socket cannot be empty"]
    if value.strip() != value:
        errors.append("daemon.socket cannot have leading/trailing whitespace")
    if "\0" in value:
        errors.append("daemon.socket cannot contain null bytes")

    pure = pathlib.PurePosixPath(value)
    if not pure.is_absolute():
        errors.append("daemon.socket must be absolute")
    if ".." in pure.parts:
        errors.append("daemon.socket cannot contain parent traversal")
    if len([part for part in pure.parts if part not in ("/", ".", "")]) == 0:
        errors.append("daemon.socket must reference a socket file")
    return errors


def collect_menu_actions(items: list[object]) -> tuple[set[str], set[str], list[str]]:
    all_ids: set[str] = set()
    executable_ids: set[str] = set()
    errors: list[str] = []

    def walk(entries: list[object]) -> None:
        for item in entries:
            if not isinstance(item, dict):
                errors.append("menu.items must contain table entries")
                continue

            item_type = item.get("type")
            if item_type == "separator":
                continue
            if item_type == "submenu":
                nested = item.get("items")
                if not isinstance(nested, list):
                    errors.append("submenu.items must be an array")
                    continue
                walk(nested)
                continue
            if item_type not in {"action", "checkbox"}:
                errors.append(f"unsupported menu item type: {item_type!r}")
                continue

            action_id = item.get("id")
            if not isinstance(action_id, str) or not is_valid_action_id(action_id):
                errors.append(f"menu action id invalid: {action_id!r}")
                continue
            if action_id in all_ids:
                errors.append(f"menu action id duplicated: {action_id!r}")
                continue
            all_ids.add(action_id)
            if item_type == "action":
                executable_ids.add(action_id)

    walk(items)
    return all_ids, executable_ids, errors


def validate_runtime_actions(
    plugin_id: str,
    runtime_actions: object,
    executable_menu_action_ids: set[str],
) -> list[str]:
    errors: list[str] = []
    if not isinstance(runtime_actions, dict) or len(runtime_actions) == 0:
        return [f"{plugin_id}: runtime.actions must be a non-empty table"]

    mapped_action_ids: set[str] = set()
    for action_id, args in runtime_actions.items():
        if not isinstance(action_id, str) or not is_valid_action_id(action_id):
            errors.append(f"{plugin_id}: invalid runtime.actions key {action_id!r}")
            continue

        mapped_action_ids.add(action_id)
        if not isinstance(args, list):
            errors.append(f"{plugin_id}: runtime.actions[{action_id!r}] must be an array")
            continue

        for arg in args:
            if not isinstance(arg, str):
                errors.append(f"{plugin_id}: runtime.actions[{action_id!r}] args must be strings")
                continue
            if "\0" in arg:
                errors.append(
                    f"{plugin_id}: runtime.actions[{action_id!r}] cannot contain null bytes"
                )

    for action_id in sorted(executable_menu_action_ids):
        if action_id not in mapped_action_ids:
            errors.append(
                f"{plugin_id}: runtime.actions missing executable menu action {action_id!r}"
            )

    return errors


def validate_runtime_section(
    plugin_id: str,
    runtime: object,
    executable_menu_action_ids: set[str],
) -> tuple[str | None, list[str]]:
    if runtime is None:
        return None, []
    if not isinstance(runtime, dict):
        return None, [f"{plugin_id}: [runtime] must be a table"]

    errors: list[str] = []
    runtime_command: str | None = None

    command = runtime.get("command")
    if not isinstance(command, str) or not is_valid_command_name(command):
        errors.append(f"{plugin_id}: invalid runtime.command {command!r}")
    else:
        runtime_command = command

    runtime_actions = runtime.get("actions")
    if runtime_actions is not None:
        errors.extend(
            validate_runtime_actions(plugin_id, runtime_actions, executable_menu_action_ids)
        )

    return runtime_command, errors


def validate_daemon_section(plugin_id: str, daemon: object) -> tuple[str | None, list[str]]:
    if daemon is None:
        return None, []
    if not isinstance(daemon, dict):
        return None, [f"{plugin_id}: [daemon] must be a table"]

    errors: list[str] = []
    daemon_command: str | None = None

    if daemon.get("enabled", False):
        command = daemon.get("command")
        if not isinstance(command, str) or not is_valid_command_name(command):
            errors.append(f"{plugin_id}: invalid daemon.command {command!r}")
        else:
            daemon_command = command

    socket_value = daemon.get("socket")
    if socket_value is not None:
        if not isinstance(socket_value, str):
            errors.append(f"{plugin_id}: daemon.socket must be a string")
        else:
            errors.extend(f"{plugin_id}: {error}" for error in validate_socket(socket_value))

    return daemon_command, errors


def collect_binary_names(plugin_id: str, dependencies: object) -> tuple[set[str], list[str]]:
    if dependencies is None:
        return set(), []
    if not isinstance(dependencies, dict):
        return set(), [f"{plugin_id}: [dependencies] must be a table"]

    binaries = dependencies.get("binaries", [])
    if not isinstance(binaries, list):
        return set(), [f"{plugin_id}: dependencies.binaries must be an array"]

    names: set[str] = set()
    for entry in binaries:
        if not isinstance(entry, dict):
            continue
        name = entry.get("name")
        if isinstance(name, str):
            names.add(name)
    return names, []


def validate_plugin(plugin_id: str, plugin_dir: pathlib.Path) -> list[str]:
    manifest_path = plugin_dir / "plugin.toml"
    if not manifest_path.is_file():
        return [f"{plugin_id}: missing plugin.toml"]

    try:
        data = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    except Exception as error:
        return [f"{plugin_id}: failed to parse plugin.toml: {error}"]

    menu = data.get("menu")
    if not isinstance(menu, dict):
        return [f"{plugin_id}: missing [menu] section"]
    menu_items = menu.get("items")
    if not isinstance(menu_items, list):
        return [f"{plugin_id}: menu.items must be an array"]

    _, executable_menu_action_ids, menu_errors = collect_menu_actions(menu_items)
    errors = [f"{plugin_id}: {error}" for error in menu_errors]

    runtime_command, runtime_errors = validate_runtime_section(
        plugin_id, data.get("runtime"), executable_menu_action_ids
    )
    errors.extend(runtime_errors)

    daemon_command, daemon_errors = validate_daemon_section(plugin_id, data.get("daemon"))
    errors.extend(daemon_errors)

    binary_names, dependency_errors = collect_binary_names(plugin_id, data.get("dependencies"))
    errors.extend(dependency_errors)

    for command in (runtime_command, daemon_command):
        if command and command not in binary_names:
            errors.append(
                f"{plugin_id}: command {command!r} missing matching dependencies.binaries name"
            )

    return errors


def discover_plugins(root: pathlib.Path, explicit_plugins: list[str] | None) -> list[str]:
    if explicit_plugins:
        return explicit_plugins
    return sorted(
        child.name
        for child in root.iterdir()
        if child.is_dir()
        and not child.name.startswith(".")
        and (child / "plugin.toml").is_file()
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--plugins-root", required=True)
    parser.add_argument("--plugins", nargs="*")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = pathlib.Path(args.plugins_root)
    if not root.is_dir():
        print(f"plugins root does not exist: {root}", file=sys.stderr)
        return 2

    plugin_ids = discover_plugins(root, args.plugins)
    if not plugin_ids:
        print(f"no plugin directories discovered under: {root}", file=sys.stderr)
        return 2

    all_errors: list[str] = []
    for plugin_id in plugin_ids:
        all_errors.extend(validate_plugin(plugin_id, root / plugin_id))

    if all_errors:
        print("First-party plugin contract validation failed:")
        for error in all_errors:
            print(f"- {error}")
        return 1

    print(f"Validated {len(plugin_ids)} first-party plugin contracts")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
