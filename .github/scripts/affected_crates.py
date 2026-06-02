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

GLOBAL_PREFIXES = (".github/workflows/", ".github/scripts/", ".cargo/")
GLOBAL_FILES = {
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "rust-toolchain",
    "clippy.toml",
    "rustfmt.toml",
    ".rustfmt.toml",
}
MACOS_ONLY = {"keyremap"}


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True)


def emit(outputs):
    text = "".join(f"{k}={v}\n" for k, v in outputs.items())
    path = os.environ.get("GITHUB_OUTPUT")
    if path:
        with open(path, "a") as handle:
            handle.write(text)
    sys.stderr.write(text)


def full_workspace(reason):
    sys.stderr.write(f"[affected] full workspace: {reason}\n")
    emit(
        {
            "full": "true",
            "ubuntu_clippy": "--workspace --exclude keyremap --all-targets",
            "ubuntu_test": "--workspace --exclude keyremap",
            "ubuntu_skip": "false",
            "macos_clippy": "--workspace --all-targets",
            "macos_test": "--workspace",
            "macos_skip": "false",
        }
    )


def skip_all(reason):
    sys.stderr.write(f"[affected] nothing to build: {reason}\n")
    emit(
        {
            "full": "false",
            "ubuntu_clippy": "",
            "ubuntu_test": "",
            "ubuntu_skip": "true",
            "macos_clippy": "",
            "macos_test": "",
            "macos_skip": "true",
        }
    )


def changed_files(base, head):
    diff = run(["git", "diff", "--name-only", base, head])
    if diff.returncode == 0:
        return diff.stdout.splitlines()
    run(["git", "fetch", "--no-tags", "--depth=50", "origin", base])
    diff = run(["git", "diff", "--name-only", base, head])
    if diff.returncode != 0:
        return None
    return diff.stdout.splitlines()


def workspace_graph():
    meta = run(["cargo", "metadata", "--no-deps", "--format-version", "1"])
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
    macos = sorted(affected)
    ubuntu = sorted(a for a in affected if a not in MACOS_ONLY)
    sys.stderr.write(f"[affected] changed={sorted(seeds)} affected={macos}\n")
    emit(
        {
            "full": "false",
            "ubuntu_clippy": args(ubuntu, True),
            "ubuntu_test": args(ubuntu, False),
            "ubuntu_skip": "true" if not ubuntu else "false",
            "macos_clippy": args(macos, True),
            "macos_test": args(macos, False),
            "macos_skip": "true" if not macos else "false",
        }
    )


if __name__ == "__main__":
    main()
