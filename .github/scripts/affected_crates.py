#!/usr/bin/env python3
"""Resolve which workspace crates a diff affects, for selective CI.

Emits per-OS clippy/test argument strings to GITHUB_OUTPUT. The result is the
set of changed crates plus every crate that (transitively) depends on them, so a
shared-lib edit still rebuilds its dependents. Anything ambiguous - a global
file changed, no usable base, cargo metadata unavailable - falls back to the
full workspace. The bias is always toward over-building, never toward skipping
something that should run.
"""
import json
import os
import subprocess
import sys
import tomllib
from pathlib import Path

GLOBAL_PREFIXES = (".github/workflows/", ".github/scripts/", ".cargo/")
GLOBAL_FILES = {
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "rust-toolchain",
    "clippy.toml",
    "rustfmt.toml",
    ".rustfmt.toml",
    ".gitattributes",
    ".gitmodules",
}
REPO_ROOT = Path(__file__).resolve().parents[2]
WORKTREE_HEAD = "WORKTREE"


def platform_excludes():
    ubuntu, macos = set(), set()
    for manifest in sorted(REPO_ROOT.glob("plugins/*/plugin.toml")):
        platforms = (
            tomllib.loads(manifest.read_text())
            .get("plugin", {})
            .get("platforms", ["linux"])
        )
        name = tomllib.loads((manifest.parent / "Cargo.toml").read_text())["package"]["name"]
        if "linux" not in platforms:
            ubuntu.add(name)
        if "macos" not in platforms:
            macos.add(name)
    return ubuntu, macos


UBUNTU_EXCLUDE, MACOS_EXCLUDE = platform_excludes()


def exclude_flags(names):
    return "".join(f" --exclude {name}" for name in sorted(names))


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True)


def emit(outputs):
    text = "".join(
        f"{key}={json.dumps(value) if isinstance(value, bool) else value}\n"
        for key, value in outputs.items()
    )
    path = os.environ.get("GITHUB_OUTPUT")
    if path:
        with open(path, "a") as handle:
            handle.write(text)
    json_path = os.environ.get("QOL_AFFECTED_OUTPUT")
    if json_path:
        Path(json_path).write_text(json.dumps(outputs, indent=2, sort_keys=True) + "\n")
    sys.stderr.write(text)


def full_workspace(reason):
    sys.stderr.write(f"[affected] full workspace: {reason}\n")
    emit(
        {
            "full": True,
            "windows_process": True,
            "windows_qol": True,
            "ubuntu_clippy": f"--workspace{exclude_flags(UBUNTU_EXCLUDE)} --all-targets",
            "ubuntu_build": f"--workspace{exclude_flags(UBUNTU_EXCLUDE)}",
            "ubuntu_test": f"--workspace{exclude_flags(UBUNTU_EXCLUDE)}",
            "ubuntu_skip": False,
            "macos_clippy": f"--workspace{exclude_flags(MACOS_EXCLUDE)} --all-targets",
            "macos_build": f"--workspace{exclude_flags(MACOS_EXCLUDE)}",
            "macos_test": f"--workspace{exclude_flags(MACOS_EXCLUDE)}",
            "macos_skip": False,
        }
    )


def skip_all(reason):
    sys.stderr.write(f"[affected] nothing to build: {reason}\n")
    emit(
        {
            "full": False,
            "windows_process": False,
            "windows_qol": False,
            "ubuntu_clippy": "",
            "ubuntu_build": "",
            "ubuntu_test": "",
            "ubuntu_skip": True,
            "macos_clippy": "",
            "macos_build": "",
            "macos_test": "",
            "macos_skip": True,
        }
    )


def changed_files(base, head):
    diff_args = ["git", "diff", "--name-only", base]
    if head != WORKTREE_HEAD:
        diff_args.append(head)
    diff = run(diff_args)
    if diff.returncode == 0:
        return with_untracked(diff.stdout.splitlines(), head)
    run(["git", "fetch", "--no-tags", "--depth=50", "origin", base])
    diff = run(diff_args)
    if diff.returncode != 0:
        return None
    return with_untracked(diff.stdout.splitlines(), head)


def with_untracked(files, head):
    if head != WORKTREE_HEAD:
        return files
    untracked = run(["git", "ls-files", "--others", "--exclude-standard"])
    if untracked.returncode != 0:
        return None
    return sorted(set(files) | set(untracked.stdout.splitlines()))


def workspace_graph():
    meta = run(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"]
    )
    if meta.returncode != 0:
        return None
    data = json.loads(meta.stdout)
    root = data.get("workspace_root", os.getcwd())
    members = set(data["workspace_members"])
    pkgs = {}
    for pkg in data["packages"]:
        if pkg["id"] not in members:
            continue
        rel = os.path.relpath(os.path.dirname(pkg["manifest_path"]), root)
        rel = "" if rel == "." else rel.replace(os.sep, "/")
        pkgs[pkg["name"]] = {"dir": rel, "deps": set()}
    names = set(pkgs)
    for pkg in data["packages"]:
        if pkg["name"] not in pkgs:
            continue
        for dep in pkg.get("dependencies", []):
            if dep["name"] in names and dep["name"] != pkg["name"]:
                pkgs[pkg["name"]]["deps"].add(dep["name"])
    return pkgs


def owning_package(path, pkgs):
    best, best_len = None, -1
    for name, info in pkgs.items():
        directory = info["dir"]
        if not directory:
            continue
        if path.startswith(directory + "/") and len(directory) > best_len:
            best, best_len = name, len(directory)
    return best


def dependents_closure(seeds, pkgs):
    reverse = {name: set() for name in pkgs}
    for name, info in pkgs.items():
        for dep in info["deps"]:
            if dep in reverse:
                reverse[dep].add(name)
    affected, stack = set(), list(seeds)
    while stack:
        current = stack.pop()
        if current in affected:
            continue
        affected.add(current)
        stack.extend(reverse.get(current, ()))
    return affected


def args(packages, all_targets):
    flags = " ".join(f"-p {pkg}" for pkg in packages)
    return f"{flags} --all-targets" if all_targets else flags


def main():
    base = os.environ.get("BASE_SHA", "").strip()
    head = os.environ.get("HEAD_SHA", "").strip() or "HEAD"
    if not base or set(base) == {"0"}:
        return full_workspace("no usable base sha")

    files = changed_files(base, head)
    if files is None:
        return full_workspace("cannot diff against base")
    files = [f for f in files if f.strip()]
    if not files:
        return skip_all("empty diff")

    for path in files:
        if path in GLOBAL_FILES or any(path.startswith(p) for p in GLOBAL_PREFIXES):
            return full_workspace(f"global file changed: {path}")

    pkgs = workspace_graph()
    if not pkgs:
        return full_workspace("cargo metadata unavailable")

    seeds = {owner for owner in (owning_package(f, pkgs) for f in files) if owner}
    if not seeds:
        return skip_all("no crate-owning changes (docs/non-build files only)")

    affected = dependents_closure(seeds, pkgs)
    macos = sorted(a for a in affected if a not in MACOS_EXCLUDE)
    ubuntu = sorted(a for a in affected if a not in UBUNTU_EXCLUDE)
    sys.stderr.write(f"[affected] changed={sorted(seeds)} affected={macos}\n")
    emit(
        {
            "full": False,
            "windows_process": "qol-process" in affected,
            "windows_qol": "qol" in affected,
            "ubuntu_clippy": args(ubuntu, True),
            "ubuntu_build": args(ubuntu, False),
            "ubuntu_test": args(ubuntu, False),
            "ubuntu_skip": not ubuntu,
            "macos_clippy": args(macos, True),
            "macos_build": args(macos, False),
            "macos_test": args(macos, False),
            "macos_skip": not macos,
        }
    )


if __name__ == "__main__":
    main()
