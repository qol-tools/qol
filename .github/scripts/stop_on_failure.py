#!/usr/bin/env python3
"""Stop a pull request run at its first failed job.

Any failure means a fix and a new push, which runs every job again, so the jobs
still running and the review of the same commit only spend runners. This
watches the run's jobs and, at the first failure, cancels the review run of the
same commit and then this run. The merge gate runs even in a cancelled run, so
the pull request still shows the failure.
"""

import json
import os
import subprocess
import sys
import time
from collections.abc import Callable

from queue_verdict import gh_json

GATE = "merge gate"
REVIEW_WORKFLOW = "claude-review.yml"
FAILED = ("failure", "timed_out")
POLL_SECONDS = 15


def first_failure(jobs: list[dict]) -> dict | None:
    return next((job for job in jobs if job["name"] != GATE and job.get("conclusion") in FAILED), None)


def gate_finished(jobs: list[dict]) -> bool:
    return any(job["name"] == GATE and job.get("status") == "completed" for job in jobs)


def cancel(repo: str, run_id: int | str) -> None:
    result = subprocess.run(
        ["gh", "api", "--method", "POST", f"repos/{repo}/actions/runs/{run_id}/cancel"],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or f"could not cancel run {run_id}")


def open_reviews(repo: str, head_sha: str) -> list[int]:
    runs = gh_json(
        [f"repos/{repo}/actions/workflows/{REVIEW_WORKFLOW}/runs?head_sha={head_sha}&event=pull_request&per_page=20"]
    )["workflow_runs"]
    return [run["id"] for run in runs if run.get("status") != "completed"]


def watch(repo: str, run_id: str, attempt: str, head_sha: str, sleep: Callable[[float], None] = time.sleep) -> str:
    while True:
        jobs = gh_json([f"repos/{repo}/actions/runs/{run_id}/attempts/{attempt}/jobs?per_page=100"])["jobs"]
        failed = first_failure(jobs)
        if failed:
            reviews = open_reviews(repo, head_sha)
            for review in reviews:
                cancel(repo, review)
            cancel(repo, run_id)
            return f"{failed['name']} failed: cancelled this run and {len(reviews)} review run(s)"
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
                os.environ["HEAD_SHA"],
            )
        )
    except (RuntimeError, KeyError, json.JSONDecodeError) as error:
        print(f"::warning::stopped watching for failures: {error}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
