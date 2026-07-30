#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

import plugin_matrix
import plugin_version

TAG_RE = re.compile(
    r"^(?P<unit>[a-z0-9][a-z0-9-]*)-v"
    r"(?P<version>[0-9]+\.[0-9]+\.[0-9]+)$"
)
REPO_RE = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
PACKAGE_RE = re.compile(r"^[A-Za-z0-9_-]+$")
TARGET_RE = re.compile(r"^[A-Za-z0-9_.-]+$")
ATTESTATION_PREFIX = "qol/release-candidate"
HOST_ID = "qol-tray"
REQUIRED_CI_JOBS = {
    "Plan affected crates",
    "lint + test (ubuntu-latest)",
    "lint + test (macos-latest)",
}


@dataclass(frozen=True)
class ReleaseTag:
    value: str
    unit_id: str
    version: str


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def parse_release_tags(value: str) -> list[ReleaseTag]:
    values = value.split()
    if not values:
        raise ValueError("at least one release tag is required")
    if len(values) != len(set(values)):
        raise ValueError("release tags must be unique")

    tags = []
    for raw in values:
        match = TAG_RE.fullmatch(raw)
        if not match:
            raise ValueError(f"invalid release tag: {raw!r}")
        tags.append(
            ReleaseTag(
                value=raw,
                unit_id=match.group("unit"),
                version=match.group("version"),
            )
        )
    return tags


def package_version(path: Path) -> str:
    return str(tomllib.loads(path.read_text())["package"]["version"])


def candidate_plan(root: Path, tags: list[ReleaseTag]) -> dict:
    plugin_entries = []
    host_tags = []
    for tag in tags:
        if tag.unit_id == HOST_ID:
            host_tags.append(tag)
            continue
        outputs = plugin_matrix.release_outputs(root, tag.unit_id, tag.version)
        matrix = json.loads(outputs["matrix"])["include"]
        plugin_entries.extend(
            {**entry, "package": outputs["package"], "tag": tag.value}
            for entry in matrix
        )

    if len(host_tags) > 1:
        raise ValueError("only one qol-tray candidate is allowed")
    if host_tags:
        actual = package_version(root / "apps/qol-tray/Cargo.toml")
        expected = host_tags[0].version
        if actual != expected:
            raise ValueError(
                f"tag expects version {expected}, got cargo={actual}"
            )

    host_tag = host_tags[0].value if host_tags else ""
    return {
        "tags": [tag.value for tag in tags],
        "plugin_matrix": {"include": plugin_entries},
        "has_plugins": bool(plugin_entries),
        "has_qol_tray": bool(host_tags),
        "qol_tray_tag": host_tag,
    }


def attestation_context(tag: ReleaseTag) -> str:
    context = f"{ATTESTATION_PREFIX}/{tag.value}"
    if len(context) > 100:
        raise ValueError(f"release tag is too long for an attestation: {tag.value}")
    return context


def validate_repo(repo: str) -> str:
    if not REPO_RE.fullmatch(repo):
        raise ValueError(f"invalid GitHub repository: {repo!r}")
    return repo


def validate_sha(sha: str) -> str:
    value = sha.lower()
    if not SHA_RE.fullmatch(value):
        raise ValueError(f"invalid commit SHA: {sha!r}")
    return value


def successful_ci_run(runs: list[dict], sha: str) -> dict:
    matches = [
        run
        for run in runs
        if run.get("head_sha") == sha and run.get("conclusion") == "success"
    ]
    if not matches:
        raise RuntimeError(f"CI has not succeeded for {sha}")
    return max(matches, key=lambda run: run.get("run_started_at", ""))


def require_source_ci(sha: str, runs: list[dict], jobs: list[dict]) -> dict:
    run = successful_ci_run(runs, sha)
    conclusions = {job.get("name"): job.get("conclusion") for job in jobs}
    missing = sorted(
        name for name in REQUIRED_CI_JOBS if conclusions.get(name) != "success"
    )
    if missing:
        raise RuntimeError(
            f"CI run {run.get('id')} lacks successful jobs: {', '.join(missing)}"
        )
    return run


def latest_attestation(tag: ReleaseTag, statuses: list[dict]) -> dict | None:
    context = attestation_context(tag)
    matches = [status for status in statuses if status.get("context") == context]
    if not matches:
        return None
    return max(matches, key=lambda status: status.get("created_at", ""))


def require_attestation(
    tag: ReleaseTag, statuses: list[dict], repo: str
) -> dict:
    status = latest_attestation(tag, statuses)
    if status is None:
        raise RuntimeError(f"{tag.value} has no release-candidate attestation")
    if status.get("state") != "success":
        raise RuntimeError(
            f"{tag.value} release-candidate attestation is {status.get('state')}"
        )
    target_url = str(status.get("target_url") or "")
    pattern = re.compile(
        rf"^https://github\.com/{re.escape(repo)}/actions/runs/[0-9]+(?:/.*)?$"
    )
    if not pattern.fullmatch(target_url):
        raise RuntimeError(
            f"{tag.value} attestation does not point to a trusted workflow run"
        )
    return status


def build_commands(
    kind: str, package: str | None, target: str | None
) -> list[list[str]]:
    verify = [
        "cargo",
        "run",
        "--quiet",
        "-p",
        "qol-build-identity",
        "--bin",
        "qol-build-identity",
        "--",
        "verify",
        "production",
    ]
    if kind == "qol-tray-linux":
        return [["cargo", "deb", "-p", "qol-tray"], verify]
    if kind == "qol-tray-macos":
        return [
            [
                "cargo",
                "build",
                "--release",
                "--target",
                "aarch64-apple-darwin",
                "--bin",
                "qol-tray",
            ],
            [
                "cargo",
                "build",
                "--release",
                "--target",
                "x86_64-apple-darwin",
                "--bin",
                "qol-tray",
            ],
            verify,
        ]
    if kind != "plugin":
        raise ValueError(f"unknown release build kind: {kind}")
    if package is None or not PACKAGE_RE.fullmatch(package):
        raise ValueError(f"invalid package: {package!r}")
    if target is None or not TARGET_RE.fullmatch(target):
        raise ValueError(f"invalid target: {target!r}")
    return [
        [
            "cargo",
            "build",
            "--release",
            "-p",
            package,
            "--target",
            target,
        ]
    ]


def write_json(path: str | None, value: dict) -> None:
    if not path:
        return
    output = Path(path)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def emit_github_output(path: str | None, values: dict) -> None:
    if not path:
        return
    with open(path, "a", encoding="utf-8") as handle:
        for key, value in values.items():
            rendered = value
            if isinstance(value, bool):
                rendered = json.dumps(value)
            if isinstance(value, (dict, list)):
                rendered = json.dumps(value, separators=(",", ":"))
            handle.write(f"{key}={rendered}\n")


def gh_json(args: list[str]) -> object:
    result = subprocess.run(
        ["gh", "api", *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "gh api failed"
        raise RuntimeError(detail)
    return json.loads(result.stdout)


def json_stream(text: str) -> list[object]:
    decoder = json.JSONDecoder()
    values = []
    index = 0
    while index < len(text):
        while index < len(text) and text[index].isspace():
            index += 1
        if index == len(text):
            return values
        value, index = decoder.raw_decode(text, index)
        values.append(value)
    return values


def gh_paginated_list(args: list[str]) -> list[dict]:
    result = subprocess.run(
        ["gh", "api", "--paginate", *args],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or "gh api failed"
        raise RuntimeError(detail)
    pages = json_stream(result.stdout)
    if any(not isinstance(page, list) for page in pages):
        raise RuntimeError("GitHub returned invalid paginated data")
    return [item for page in pages for item in page]


def completed_ci_runs(repo: str, sha: str) -> list[dict]:
    payload = gh_json(
        [
            "--method",
            "GET",
            f"/repos/{repo}/actions/workflows/ci.yml/runs",
            "-f",
            f"head_sha={sha}",
            "-f",
            "event=push",
            "-f",
            "status=completed",
            "-f",
            "per_page=20",
        ]
    )
    return list(payload.get("workflow_runs", []))


def ci_run_jobs(repo: str, run_id: int) -> list[dict]:
    payload = gh_json(
        [
            "--method",
            "GET",
            f"/repos/{repo}/actions/runs/{run_id}/jobs",
            "-f",
            "per_page=100",
        ]
    )
    return list(payload.get("jobs", []))


def source_ci_evidence(repo: str, sha: str, root: Path) -> dict:
    ci_sha = sha
    runs = completed_ci_runs(repo, ci_sha)
    try:
        run = successful_ci_run(runs, ci_sha)
    except RuntimeError as exact_error:
        if runs:
            raise
        try:
            ci_sha = plugin_version.verified_version_bump_parent(root, sha)
        except RuntimeError as bump_error:
            raise RuntimeError(
                f"{exact_error}; parent CI fallback refused: {bump_error}"
            ) from exact_error
        runs = completed_ci_runs(repo, ci_sha)
        run = successful_ci_run(runs, ci_sha)
    evidence = require_source_ci(ci_sha, runs, ci_run_jobs(repo, run["id"]))
    return {**evidence, "source_sha": sha, "ci_sha": ci_sha}


def commit_statuses(repo: str, sha: str) -> list[dict]:
    return gh_paginated_list(
        [
            "--method",
            "GET",
            f"/repos/{repo}/commits/{sha}/statuses",
            "-f",
            "per_page=100",
        ]
    )


def attest(
    repo: str,
    sha: str,
    tags: list[ReleaseTag],
    target_url: str,
) -> list[dict]:
    expected_prefix = f"https://github.com/{repo}/actions/runs/"
    if not target_url.startswith(expected_prefix):
        raise ValueError(f"invalid workflow run URL: {target_url!r}")
    statuses = []
    for tag in tags:
        statuses.append(
            gh_json(
                [
                    "--method",
                    "POST",
                    f"/repos/{repo}/statuses/{sha}",
                    "-f",
                    "state=success",
                    "-f",
                    f"context={attestation_context(tag)}",
                    "-f",
                    "description=Release candidate passed",
                    "-f",
                    f"target_url={target_url}",
                ]
            )
        )
    return statuses


def execute_build(
    root: Path,
    kind: str,
    package: str | None,
    target: str | None,
    report_path: str,
) -> None:
    started_at = utc_now()
    commands = build_commands(kind, package, target)
    report = {
        "name": "release-build",
        "started_at": started_at,
        "finished_at": "",
        "status": "failed",
        "inputs": {
            "kind": kind,
            "package": package,
            "target": target,
            "root": str(root),
        },
        "artifacts": {},
        "commands": commands,
        "next": [],
    }
    for command in commands:
        result = subprocess.run(command, cwd=root, check=False)
        if result.returncode != 0:
            report["finished_at"] = utc_now()
            write_json(report_path, report)
            raise RuntimeError(
                f"{' '.join(command)} failed with exit code {result.returncode}"
            )
    report["finished_at"] = utc_now()
    report["status"] = "pass"
    write_json(report_path, report)


def plan_command(args: argparse.Namespace) -> None:
    started_at = utc_now()
    tags = parse_release_tags(args.tags)
    plan = candidate_plan(Path(args.root).resolve(), tags)
    outputs = {
        "plugin_matrix": plan["plugin_matrix"],
        "has_plugins": plan["has_plugins"],
        "has_qol_tray": plan["has_qol_tray"],
        "qol_tray_tag": plan["qol_tray_tag"],
    }
    emit_github_output(args.github_output, outputs)
    report = {
        "name": "release-candidate-plan",
        "started_at": started_at,
        "finished_at": utc_now(),
        "status": "pass",
        "inputs": {"root": str(Path(args.root).resolve()), "tags": plan["tags"]},
        "artifacts": {"plan": plan},
        "commands": [],
        "next": ["build"],
    }
    write_json(args.report, report)
    print(json.dumps(plan, indent=2, sort_keys=True))


def source_ci_command(args: argparse.Namespace) -> None:
    repo = validate_repo(args.repo)
    sha = validate_sha(args.sha)
    started_at = utc_now()
    root = Path(args.root).resolve()
    evidence = source_ci_evidence(repo, sha, root)
    report = {
        "name": "release-source-ci",
        "started_at": started_at,
        "finished_at": utc_now(),
        "status": "pass",
        "inputs": {"repo": repo, "source_sha": sha, "root": str(root)},
        "artifacts": {
            "ci_sha": evidence["ci_sha"],
            "run_id": evidence["id"],
            "run_url": evidence["html_url"],
        },
        "commands": [],
        "next": ["version"],
    }
    write_json(args.report, report)
    print(
        f"CI passed for {sha} via {evidence['ci_sha']}: "
        f"{evidence['html_url']}"
    )


def attest_command(args: argparse.Namespace) -> None:
    repo = validate_repo(args.repo)
    sha = validate_sha(args.sha)
    tags = parse_release_tags(args.tags)
    started_at = utc_now()
    statuses = attest(repo, sha, tags, args.target_url)
    report = {
        "name": "release-candidate-attestation",
        "started_at": started_at,
        "finished_at": utc_now(),
        "status": "pass",
        "inputs": {
            "repo": repo,
            "sha": sha,
            "tags": [tag.value for tag in tags],
        },
        "artifacts": {
            "contexts": [attestation_context(tag) for tag in tags],
            "target_url": args.target_url,
            "status_ids": [status.get("id") for status in statuses],
        },
        "commands": [],
        "next": ["tag", "dispatch"],
    }
    write_json(args.report, report)


def verify_command(args: argparse.Namespace) -> None:
    repo = validate_repo(args.repo)
    sha = validate_sha(args.sha)
    tag = parse_release_tags(args.tag)[0]
    started_at = utc_now()
    status = require_attestation(tag, commit_statuses(repo, sha), repo)
    report = {
        "name": "release-candidate-verification",
        "started_at": started_at,
        "finished_at": utc_now(),
        "status": "pass",
        "inputs": {"repo": repo, "sha": sha, "tag": tag.value},
        "artifacts": {
            "context": status["context"],
            "target_url": status["target_url"],
        },
        "commands": [],
        "next": ["build", "publish"],
    }
    write_json(args.report, report)
    print(f"Verified {tag.value} at {sha}: {status['target_url']}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)

    plan = commands.add_parser("plan")
    plan.add_argument("--root", default=".")
    plan.add_argument("--tags", required=True)
    plan.add_argument("--report")
    plan.add_argument(
        "--github-output", default=os.environ.get("GITHUB_OUTPUT")
    )
    plan.set_defaults(handler=plan_command)

    source_ci = commands.add_parser("verify-source-ci")
    source_ci.add_argument("--repo", required=True)
    source_ci.add_argument("--sha", required=True)
    source_ci.add_argument("--root", default=".")
    source_ci.add_argument("--report")
    source_ci.set_defaults(handler=source_ci_command)

    create = commands.add_parser("attest")
    create.add_argument("--repo", required=True)
    create.add_argument("--sha", required=True)
    create.add_argument("--tags", required=True)
    create.add_argument("--target-url", required=True)
    create.add_argument("--report")
    create.set_defaults(handler=attest_command)

    verify = commands.add_parser("verify")
    verify.add_argument("--repo", required=True)
    verify.add_argument("--sha", required=True)
    verify.add_argument("--tag", required=True)
    verify.add_argument("--report")
    verify.set_defaults(handler=verify_command)

    build = commands.add_parser("build")
    build.add_argument(
        "--kind",
        choices=["plugin", "qol-tray-linux", "qol-tray-macos"],
        required=True,
    )
    build.add_argument("--package")
    build.add_argument("--target")
    build.add_argument("--root", default=".")
    build.add_argument("--report", required=True)
    build.set_defaults(
        handler=lambda args: execute_build(
            Path(args.root).resolve(),
            args.kind,
            args.package,
            args.target,
            args.report,
        )
    )
    return parser


def main() -> int:
    args = build_parser().parse_args()
    try:
        args.handler(args)
    except Exception as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
