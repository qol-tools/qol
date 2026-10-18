#!/usr/bin/env python3
"""Stop a pull request or merge queue run at its first failed job.

Any failure means a fix and a new push, or a queue entry sent back, so the jobs
still running only spend runners the queue entries behind it need. This watches
the run's jobs and, at the first failure, cancels this run. The gate and the
queue failure report run even in a cancelled run, so the pull request still
shows the failure.
"""

import json
import os
import subprocess
import sys
import time
from collections.abc import Callable

from queue_verdict import gh_json

GATES = ("merge gate", "queue gate")
FAILED = ("failure", "timed_out")
POLL_SECONDS = 15


def first_failure(jobs: list[dict]) -> dict | None:
    return next((job for job in jobs if job["name"] not in GATES and job.get("conclusion") in FAILED), None)


def gate_finished(jobs: list[dict]) -> bool:
    return any(job["name"] in GATES and job.get("status") == "completed" for job in jobs)


def cancel(repo: str, run_id: int | str) -> None:
    result = subprocess.run(
        ["gh", "api", "--method", "POST", f"repos/{repo}/actions/runs/{run_id}/cancel"],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or f"could not cancel run {run_id}")


def watch(repo: str, run_id: str, attempt: str, sleep: Callable[[float], None] = time.sleep) -> str:
    while True:
        jobs = gh_json([f"repos/{repo}/actions/runs/{run_id}/attempts/{attempt}/jobs?per_page=100"])["jobs"]
        failed = first_failure(jobs)
        if failed:
            cancel(repo, run_id)
            return f"{failed['name']} failed: cancelled this run"
        if gate_finished(jobs):
            return "every job finished"
        sleep(POLL_SECONDS)


def main() -> int:
    try:
        print(
            watch(
                os.environ["GITHUB_REPOSITORY"],
                os.environ["GITHUB_RUN_ID"],
                os.environ["GITHUB_RUN_ATTEMPT"],
            )
        )
    except (RuntimeError, KeyError, json.JSONDecodeError) as error:
        print(f"::warning::stopped watching for failures: {error}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
