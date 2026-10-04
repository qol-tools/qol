#!/usr/bin/env python3
"""Decide whether a run can reuse a verdict this workflow already reached.

A queue entry whose tree is byte-identical to a pull request head that already
passed this workflow has nothing new to lint or test. Any other tree (main moved,
or the entry is batched with other pull requests) runs the full workflow.

A push to main whose commit already passed a merge queue run is the commit the
queue tested, because the queue fast-forwards main to it, so it reruns nothing.
"""

import json
import os
import re
import subprocess
import sys

QUEUE_BRANCH = re.compile(r"/pr-(\d+)-[0-9a-f]{40}$")


def queued_pull(head_ref: str) -> int | None:
    match = QUEUE_BRANCH.search(head_ref)
    return int(match.group(1)) if match else None


def passed(runs: list[dict], workflow: str) -> bool:
    return any(
        run.get("name") == workflow and run.get("conclusion") == "success"
        for run in runs
    )


def reusable(queue_tree: str, pull_tree: str, runs: list[dict], workflow: str) -> bool:
    return queue_tree == pull_tree and passed(runs, workflow)


def gh_json(args: list[str]) -> dict:
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


def tree_of(repo: str, sha: str) -> str:
    return gh_json([f"repos/{repo}/git/commits/{sha}"])["tree"]["sha"]


def passed_runs(repo: str, sha: str, event: str) -> list[dict]:
    return gh_json(
        [
            "--method",
            "GET",
            f"repos/{repo}/actions/runs",
            "-f",
            f"head_sha={sha}",
            "-f",
            f"event={event}",
            "-f",
            "status=success",
        ]
    )["workflow_runs"]


def verdict(repo: str, head_ref: str, queue_sha: str, workflow: str) -> bool:
    number = queued_pull(head_ref)
    if number is None:
        return False
    pull_sha = gh_json([f"repos/{repo}/pulls/{number}"])["head"]["sha"]
    runs = passed_runs(repo, pull_sha, "pull_request")
    return reusable(tree_of(repo, queue_sha), tree_of(repo, pull_sha), runs, workflow)


def landed_verdict(repo: str, sha: str, workflow: str) -> bool:
    return passed(passed_runs(repo, sha, "merge_group"), workflow)


def main() -> int:
    try:
        repo = os.environ["GITHUB_REPOSITORY"]
        sha = os.environ["GITHUB_SHA"]
        workflow = os.environ["GITHUB_WORKFLOW"]
        if os.environ["GITHUB_EVENT_NAME"] == "push":
            reused = landed_verdict(repo, sha, workflow)
        else:
            reused = verdict(repo, os.environ["HEAD_REF"], sha, workflow)
    except (RuntimeError, KeyError, json.JSONDecodeError) as error:
        print(f"::warning::running the full workflow: {error}", file=sys.stderr)
        reused = False
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as handle:
        handle.write(f"reused={json.dumps(reused)}\n")
    print(f"reused={json.dumps(reused)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
