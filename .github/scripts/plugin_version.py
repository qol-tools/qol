#!/usr/bin/env python3
"""Compute and apply monorepo plugin version bumps."""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

SEMVER_RE = re.compile(r"^([0-9]+)\.([0-9]+)\.([0-9]+)$")
BREAKING_SUBJECT_RE = re.compile(r"^[a-z0-9_-]+(\([^)]+\))?!:", re.IGNORECASE)
BREAKING_BODY_RE = re.compile(r"(^|\n)BREAKING[ -]CHANGE:", re.IGNORECASE)
FEATURE_SUBJECT_RE = re.compile(r"^feat(\([^)]+\))?:", re.IGNORECASE)
RELEASABLE_SUBJECT_RE = re.compile(r"^(feat|fix|perf)(\([^)]+\))?!?:", re.IGNORECASE)
RELEASE_SUBJECT_RE = re.compile(r"^chore\(release\):", re.IGNORECASE)

DEPENDENCY_TABLES = {"dependencies", "build-dependencies"}
PLUGIN_ID_RE = re.compile(r"^[a-z0-9][a-z0-9-]*$")
ROOT_MANIFEST = "Cargo.toml"
ROOT_LOCKFILE = "Cargo.lock"
ROOT_MANIFEST_AFFECT_ALL_SECTIONS = {"profile", "patch"}
ROOT_WORKSPACE_AFFECT_ALL_FIELDS = {"resolver", "members", "exclude", "package"}
ROOT_WORKSPACE_CLASSIFIED_FIELDS = ROOT_WORKSPACE_AFFECT_ALL_FIELDS | {"dependencies"}
GLOBAL_FILES_AFFECT_ALL = {
    "rust-toolchain",
    "rust-toolchain.toml",
}
GLOBAL_PREFIXES_AFFECT_ALL = (".cargo/",)
AUTO_EXCLUDED_PLUGIN_IDS = {"template"}


@dataclass(frozen=True)
class Commit:
    sha: str
    subject: str
    body: str


@dataclass
class Package:
    name: str
    directory: Path
    rel_dir: str
    manifest: Path
    deps: set[str]
    external_deps: set[str]


@dataclass(frozen=True)
class Plugin:
    plugin_id: str
    directory: Path
    package_name: str
    cargo_manifest: Path
    plugin_manifest: Path
    cargo_version: str
    plugin_version: str


@dataclass(frozen=True)
class ReleasePlan:
    plugin: Plugin
    old_version: str
    new_version: str
    tag: str
    bump: str
    commit_count: int


@dataclass(frozen=True)
class RootImpact:
    affects_all: bool
    dependencies: frozenset[str]


def run_git(args: list[str], root: Path) -> str:
    result = subprocess.run(
        ["git", "-C", str(root), *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        message = result.stderr.strip() or result.stdout.strip() or "git failed"
        command = " ".join(["git", "-C", str(root), *args])
        raise RuntimeError(f"{command} failed: {message}")
    return result.stdout


def git_output(args: list[str], root: Path) -> str | None:
    result = subprocess.run(
        ["git", "-C", str(root), *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        return None
    return result.stdout.strip()


def parse_semver(version: str) -> tuple[int, int, int]:
    match = SEMVER_RE.match(version.strip())
    if not match:
        raise ValueError(f"Invalid semver: {version}")
    return int(match.group(1)), int(match.group(2)), int(match.group(3))


def increment_semver(version: str, bump: str) -> str:
    major, minor, patch = parse_semver(version)
    if bump == "major":
        return f"{major + 1}.0.0"
    if bump == "minor":
        return f"{major}.{minor + 1}.0"
    if bump == "patch":
        return f"{major}.{minor}.{patch + 1}"
    raise ValueError(f"Invalid bump: {bump}")


def version_greater(left: str, right: str) -> bool:
    return parse_semver(left) > parse_semver(right)


def next_available_version(version: str, existing_tags: Iterable[str], prefix: str) -> str:
    major, minor, patch = parse_semver(version)
    tags = {tag.strip() for tag in existing_tags if tag.strip()}
    candidate = f"{major}.{minor}.{patch}"
    while f"{prefix}{candidate}" in tags:
        patch += 1
        candidate = f"{major}.{minor}.{patch}"
    return candidate


def is_release_commit(commit: Commit) -> bool:
    return RELEASE_SUBJECT_RE.match(commit.subject.strip()) is not None


def is_releasable_commit(commit: Commit) -> bool:
    if is_release_commit(commit):
        return False
    subject = commit.subject.strip()
    body = commit.body or ""
    if BREAKING_SUBJECT_RE.match(subject) or BREAKING_BODY_RE.search(body):
        return True
    return RELEASABLE_SUBJECT_RE.match(subject) is not None


def detect_bump(commits: list[Commit]) -> str:
    bump = "patch"
    for commit in commits:
        subject = commit.subject.strip()
        body = commit.body or ""
        if BREAKING_SUBJECT_RE.match(subject) or BREAKING_BODY_RE.search(body):
            return "major"
        if bump == "patch" and FEATURE_SUBJECT_RE.match(subject):
            bump = "minor"
    return bump


def toml_at(path: Path) -> dict:
    return tomllib.loads(path.read_text())


def package_version(manifest: Path) -> str:
    return str(toml_at(manifest)["package"]["version"])


def plugin_version(manifest: Path) -> str:
    return str(toml_at(manifest)["plugin"]["version"])


def update_table_version(path: Path, table: str, version: str) -> None:
    text = path.read_text()
    match = re.search(rf"(?ms)^\[{re.escape(table)}\]\n.*?(?=^\[|\Z)", text)
    if not match:
        raise RuntimeError(f"Could not find [{table}] in {path}")
    section = match.group(0)
    next_text, count = re.subn(
        r'(?m)^version = "[^"]+"',
        f'version = "{version}"',
        section,
        count=1,
    )
    if count != 1:
        raise RuntimeError(f"Could not update [{table}].version in {path}")
    path.write_text(text[: match.start()] + next_text + text[match.end() :])


def update_lock_version(lockfile: Path, package: str, old: str, new: str) -> None:
    if not lockfile.exists():
        return
    text = lockfile.read_text()
    blocks = text.split("[[package]]")
    changed = False
    for index, block in enumerate(blocks):
        if index == 0:
            continue
        if f'name = "{package}"' not in block:
            continue
        next_block, count = re.subn(
            rf'(?m)^version = "{re.escape(old)}"$',
            f'version = "{new}"',
            block,
            count=1,
        )
        if count:
            blocks[index] = next_block
            changed = True
            break
    if not changed:
        raise RuntimeError(
            f"Could not update {package} {old}->{new} in {lockfile.relative_to(lockfile.parent)}"
        )
    lockfile.write_text("[[package]]".join(blocks))


def workspace_members(root: Path) -> list[Path]:
    workspace = toml_at(root / "Cargo.toml")["workspace"]
    manifests: list[Path] = []
    for member in workspace.get("members", []):
        for path in sorted(root.glob(member)):
            manifest = path / "Cargo.toml"
            if manifest.exists():
                manifests.append(manifest)
    return manifests


def dependency_tables(value: object) -> Iterable[dict]:
    if not isinstance(value, dict):
        return
    for key, item in value.items():
        if key in DEPENDENCY_TABLES and isinstance(item, dict):
            yield item
        elif key == "target" and isinstance(item, dict):
            for target in item.values():
                yield from dependency_tables(target)


def load_packages(root: Path) -> dict[str, Package]:
    packages: dict[str, Package] = {}
    manifests = workspace_members(root)
    manifest_to_name: dict[Path, str] = {}

    for manifest in manifests:
        data = toml_at(manifest)
        name = str(data["package"]["name"])
        directory = manifest.parent.resolve()
        rel_dir = directory.relative_to(root).as_posix()
        packages[name] = Package(
            name=name,
            directory=directory,
            rel_dir=rel_dir,
            manifest=manifest,
            deps=set(),
            external_deps=set(),
        )
        manifest_to_name[manifest.resolve()] = name

    for package in packages.values():
        data = toml_at(package.manifest)
        for deps in dependency_tables(data):
            for dep_key, spec in deps.items():
                dep_name = dep_key
                if isinstance(spec, dict):
                    if "path" in spec:
                        manifest = (package.directory / str(spec["path"]) / "Cargo.toml").resolve()
                        resolved = manifest_to_name.get(manifest)
                        if resolved:
                            package.deps.add(resolved)
                        continue
                    dep_name = str(spec.get("package", dep_key))
                if dep_name in packages:
                    package.deps.add(dep_name)
                else:
                    package.external_deps.add(dep_name)

    return packages


def owning_package(path: str, packages: dict[str, Package]) -> str | None:
    best_name: str | None = None
    best_len = -1
    for name, package in packages.items():
        rel = package.rel_dir
        if path == rel or path.startswith(rel + "/"):
            if len(rel) > best_len:
                best_name = name
                best_len = len(rel)
    return best_name


def dependency_closure(package_name: str, packages: dict[str, Package]) -> set[str]:
    closure: set[str] = set()
    stack = [package_name]
    while stack:
        current = stack.pop()
        if current in closure:
            continue
        closure.add(current)
        stack.extend(packages[current].deps)
    return closure


def cached_dependency_closure(
    package_name: str,
    packages: dict[str, Package],
    closure_cache: dict[str, set[str]],
) -> set[str]:
    if package_name not in closure_cache:
        closure_cache[package_name] = dependency_closure(package_name, packages)
    return closure_cache[package_name]


def package_uses_dependency(
    package_name: str,
    dependency: str,
    packages: dict[str, Package],
    closure_cache: dict[str, set[str]],
) -> bool:
    closure = cached_dependency_closure(package_name, packages, closure_cache)
    if dependency in closure:
        return True
    return any(dependency in packages[name].external_deps for name in closure)


def parent_of_commit(root: Path, sha: str) -> str | None:
    output = run_git(["rev-list", "--parents", "-n", "1", sha], root).strip()
    parts = output.split()
    return parts[1] if len(parts) > 1 else None


def file_at_commit(root: Path, sha: str, path: str) -> str | None:
    return git_output(["show", f"{sha}:{path}"], root)


def toml_from_git(root: Path, sha: str, path: str) -> dict | None:
    text = file_at_commit(root, sha, path)
    if text is None:
        return None
    return tomllib.loads(text)


def changed_workspace_dependencies(before: dict, after: dict) -> frozenset[str]:
    before_deps = before.get("workspace", {}).get("dependencies", {})
    after_deps = after.get("workspace", {}).get("dependencies", {})
    keys = set(before_deps) | set(after_deps)
    return frozenset(key for key in keys if before_deps.get(key) != after_deps.get(key))


def root_manifest_impact(root: Path, sha: str) -> RootImpact:
    parent = parent_of_commit(root, sha)
    if parent is None:
        return RootImpact(affects_all=True, dependencies=frozenset())
    before = toml_from_git(root, parent, ROOT_MANIFEST)
    after = toml_from_git(root, sha, ROOT_MANIFEST)
    if before is None or after is None:
        return RootImpact(affects_all=True, dependencies=frozenset())

    for section in ROOT_MANIFEST_AFFECT_ALL_SECTIONS:
        if before.get(section) != after.get(section):
            return RootImpact(affects_all=True, dependencies=frozenset())

    before_workspace = before.get("workspace", {})
    after_workspace = after.get("workspace", {})
    for key in ROOT_WORKSPACE_AFFECT_ALL_FIELDS:
        if before_workspace.get(key) != after_workspace.get(key):
            return RootImpact(affects_all=True, dependencies=frozenset())

    before_root_keys = set(before) - {"workspace"}
    after_root_keys = set(after) - {"workspace"}
    classified_root_keys = ROOT_MANIFEST_AFFECT_ALL_SECTIONS
    if before_root_keys - classified_root_keys or after_root_keys - classified_root_keys:
        return RootImpact(affects_all=True, dependencies=frozenset())

    before_workspace_keys = set(before_workspace)
    after_workspace_keys = set(after_workspace)
    if (
        before_workspace_keys - ROOT_WORKSPACE_CLASSIFIED_FIELDS
        or after_workspace_keys - ROOT_WORKSPACE_CLASSIFIED_FIELDS
    ):
        return RootImpact(affects_all=True, dependencies=frozenset())

    return RootImpact(
        affects_all=False,
        dependencies=changed_workspace_dependencies(before, after),
    )


def lock_package_state(text: str | None) -> dict[str, set[str]]:
    if text is None:
        return {}
    state: dict[str, set[str]] = {}
    for block in text.split("[[package]]")[1:]:
        name_match = re.search(r'(?m)^name = "([^"]+)"$', block)
        if not name_match:
            continue
        name = name_match.group(1)
        state.setdefault(name, set()).add(block.strip())
    return state


def changed_lock_packages(root: Path, sha: str) -> frozenset[str]:
    parent = parent_of_commit(root, sha)
    if parent is None:
        return frozenset()
    before = lock_package_state(file_at_commit(root, parent, ROOT_LOCKFILE))
    after = lock_package_state(file_at_commit(root, sha, ROOT_LOCKFILE))
    names = set(before) | set(after)
    return frozenset(name for name in names if before.get(name) != after.get(name))


def dependency_names_affect_plugin(
    dependency_names: frozenset[str],
    package_name: str,
    packages: dict[str, Package],
    closure_cache: dict[str, set[str]],
) -> bool:
    return any(
        package_uses_dependency(package_name, dependency, packages, closure_cache)
        for dependency in dependency_names
    )


def path_affects_plugin(
    root: Path,
    commit: Commit,
    path: str,
    package_name: str,
    packages: dict[str, Package],
    closure_cache: dict[str, set[str]],
    root_manifest_cache: dict[str, RootImpact],
    lockfile_cache: dict[str, frozenset[str]],
) -> bool:
    if path in GLOBAL_FILES_AFFECT_ALL or path.startswith(GLOBAL_PREFIXES_AFFECT_ALL):
        return True
    if path == ROOT_MANIFEST:
        impact = root_manifest_cache.setdefault(
            commit.sha, root_manifest_impact(root, commit.sha)
        )
        if impact.affects_all:
            return True
        return dependency_names_affect_plugin(
            impact.dependencies, package_name, packages, closure_cache
        )
    if path == ROOT_LOCKFILE:
        changed_packages = lockfile_cache.setdefault(
            commit.sha, changed_lock_packages(root, commit.sha)
        )
        return dependency_names_affect_plugin(
            changed_packages, package_name, packages, closure_cache
        )
    owner = owning_package(path, packages)
    if owner is None:
        return False
    return owner in cached_dependency_closure(package_name, packages, closure_cache)


def load_commits(root: Path, rev_range: str) -> list[Commit]:
    output = run_git(["log", "--format=%H%x1f%s%x1f%b%x1e", rev_range], root)
    commits: list[Commit] = []
    for raw in output.split("\x1e"):
        raw = raw.strip("\n")
        if not raw:
            continue
        parts = raw.split("\x1f", 2)
        if len(parts) == 2:
            sha, subject = parts
            body = ""
        else:
            sha, subject, body = parts
        commits.append(Commit(sha=sha.strip(), subject=subject.strip(), body=body.rstrip("\n")))
    return commits


def changed_paths_for_commit(root: Path, sha: str) -> list[str]:
    output = run_git(["diff-tree", "--root", "--no-commit-id", "--name-only", "-r", sha], root)
    return [line.strip() for line in output.splitlines() if line.strip()]


def last_tag(root: Path, prefix: str) -> str | None:
    return git_output(["describe", "--tags", "--abbrev=0", "--match", f"{prefix}[0-9]*"], root)


def tag_version(prefix: str, tag: str) -> str:
    value = tag.removeprefix(prefix)
    parse_semver(value)
    return value


def existing_tags(root: Path, prefix: str) -> list[str]:
    output = run_git(["tag", "--list", f"{prefix}*"], root)
    return [line.strip() for line in output.splitlines() if line.strip()]


def normalize_plugin_id(value: str) -> str:
    value = value.strip()
    if value.startswith("plugin-"):
        value = value[len("plugin-") :]
    if not PLUGIN_ID_RE.match(value):
        raise RuntimeError(f"Invalid plugin id: {value}")
    return value


def discover_plugins(root: Path, packages: dict[str, Package], selected: str | None) -> list[Plugin]:
    plugin_dirs = sorted((root / "plugins").glob("plugin-*"))
    selected_id = normalize_plugin_id(selected) if selected else None
    plugins: list[Plugin] = []
    for directory in plugin_dirs:
        plugin_id = directory.name.removeprefix("plugin-")
        if not PLUGIN_ID_RE.match(plugin_id):
            raise RuntimeError(f"Invalid plugin directory id: {plugin_id}")
        if selected_id is None and plugin_id in AUTO_EXCLUDED_PLUGIN_IDS:
            continue
        if selected_id and plugin_id != selected_id:
            continue
        cargo_manifest = directory / "Cargo.toml"
        plugin_manifest = directory / "plugin.toml"
        if not cargo_manifest.exists() or not plugin_manifest.exists():
            continue
        plugin_data = toml_at(plugin_manifest)["plugin"]
        manifest_id = str(plugin_data["id"])
        if manifest_id != f"plugin-{plugin_id}":
            raise RuntimeError(
                f"Plugin id mismatch in {plugin_manifest}: expected plugin-{plugin_id}, got {manifest_id}"
            )
        package_name = str(toml_at(cargo_manifest)["package"]["name"])
        if package_name not in packages:
            raise RuntimeError(f"{package_name} is not a workspace package")
        plugins.append(
            Plugin(
                plugin_id=plugin_id,
                directory=directory,
                package_name=package_name,
                cargo_manifest=cargo_manifest,
                plugin_manifest=plugin_manifest,
                cargo_version=package_version(cargo_manifest),
                plugin_version=plugin_version(plugin_manifest),
            )
        )
    if selected_id and not plugins:
        raise RuntimeError(f"Unknown plugin: {selected}")
    return plugins


def relevant_commits(
    root: Path,
    commits: list[Commit],
    plugin: Plugin,
    packages: dict[str, Package],
    changed_paths_cache: dict[str, list[str]],
    closure_cache: dict[str, set[str]],
    root_manifest_cache: dict[str, RootImpact],
    lockfile_cache: dict[str, frozenset[str]],
) -> list[Commit]:
    relevant: list[Commit] = []
    for commit in commits:
        paths = changed_paths_cache.setdefault(
            commit.sha, changed_paths_for_commit(root, commit.sha)
        )
        if any(
            path_affects_plugin(
                root,
                commit,
                path,
                plugin.package_name,
                packages,
                closure_cache,
                root_manifest_cache,
                lockfile_cache,
            )
            for path in paths
        ):
            relevant.append(commit)
    return relevant


def compute_plans(root: Path, selected: str | None) -> list[ReleasePlan]:
    packages = load_packages(root)
    plugins = discover_plugins(root, packages, selected)
    commits_by_range: dict[str, list[Commit]] = {}
    changed_paths_cache: dict[str, list[str]] = {}
    closure_cache: dict[str, set[str]] = {}
    root_manifest_cache: dict[str, RootImpact] = {}
    lockfile_cache: dict[str, frozenset[str]] = {}
    plans: list[ReleasePlan] = []

    for plugin in plugins:
        prefix = f"plugin-{plugin.plugin_id}-v"
        tag = last_tag(root, prefix)
        if tag is None and selected is None:
            continue
        if plugin.cargo_version != plugin.plugin_version:
            raise RuntimeError(
                f"Manifest versions differ for {plugin.plugin_id}: "
                f"cargo={plugin.cargo_version} plugin={plugin.plugin_version}"
            )

        base_version = plugin.cargo_version
        rev_range = "HEAD"
        if tag:
            base_version = tag_version(prefix, tag)
            rev_range = f"{tag}..HEAD"
            if run_git(["rev-list", rev_range, "--count"], root).strip() == "0":
                continue

        if rev_range not in commits_by_range:
            commits_by_range[rev_range] = load_commits(root, rev_range)
        commits = relevant_commits(
            root,
            commits_by_range[rev_range],
            plugin,
            packages,
            changed_paths_cache,
            closure_cache,
            root_manifest_cache,
            lockfile_cache,
        )
        release_commits = [commit for commit in commits if is_releasable_commit(commit)]
        if not release_commits:
            continue

        bump = detect_bump(release_commits)
        next_version = next_available_version(
            increment_semver(base_version, bump),
            existing_tags(root, prefix),
            prefix,
        )
        if version_greater(plugin.cargo_version, next_version):
            next_version = next_available_version(plugin.cargo_version, existing_tags(root, prefix), prefix)

        plans.append(
            ReleasePlan(
                plugin=plugin,
                old_version=plugin.cargo_version,
                new_version=next_version,
                tag=f"{prefix}{next_version}",
                bump=bump,
                commit_count=len(release_commits),
            )
        )

    return plans


def apply_plans(root: Path, plans: list[ReleasePlan]) -> bool:
    changed = False
    lockfile = root / "Cargo.lock"
    for plan in plans:
        if plan.old_version == plan.new_version:
            continue
        update_table_version(plan.plugin.cargo_manifest, "package", plan.new_version)
        update_table_version(plan.plugin.plugin_manifest, "plugin", plan.new_version)
        update_lock_version(lockfile, plan.plugin.package_name, plan.old_version, plan.new_version)
        changed = True
    return changed


def emit_github_output(path: str | None, values: dict[str, str]) -> None:
    if not path:
        return
    with open(path, "a", encoding="utf-8") as handle:
        for key, value in values.items():
            handle.write(f"{key}={value}\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--plugin-id")
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--tag-file")
    parser.add_argument("--github-output", default=os.environ.get("GITHUB_OUTPUT"))
    args = parser.parse_args()

    root = Path.cwd().resolve()
    plans = compute_plans(root, args.plugin_id)
    manifest_changed = apply_plans(root, plans) if args.apply else False

    if args.tag_file:
        Path(args.tag_file).write_text("\n".join(plan.tag for plan in plans) + ("\n" if plans else ""))

    summary = ", ".join(
        f"{plan.plugin.plugin_id} {plan.old_version}->{plan.new_version} ({plan.bump})"
        for plan in plans
    )
    emit_github_output(
        args.github_output,
        {
            "should_release": "true" if plans else "false",
            "manifest_changed": "true" if manifest_changed else "false",
            "tags": " ".join(plan.tag for plan in plans),
            "summary": summary,
        },
    )

    if plans:
        for plan in plans:
            print(
                f"{plan.plugin.plugin_id}: {plan.old_version} -> {plan.new_version} "
                f"({plan.bump}, {plan.commit_count} commits, {plan.tag})"
            )
    else:
        print("No plugin releases required")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"::error::{error}", file=sys.stderr)
        raise SystemExit(1)
