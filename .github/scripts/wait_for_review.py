#!/usr/bin/env python3
"""Hold a pull request run until the review of its commit passes.

A review that does not pass means a fix and a new push, which runs every job
again, so a pull request run lints, tests and analyzes only a commit whose
review passed. A dependabot pull request is not reviewed, and its review run
still passes.
"""

import json
import os
import sys
import time
from collections.abc import Callable

from queue_verdict import gh_json

REVIEW_WORKFLOW = "claude-review.yml"
POLL_SECONDS = 15
FAILED_POLLS_ALLOWED = 4


def wait(repo: str, head_sha: str, sleep: Callable[[float], None] = time.sleep) -> dict:
    failed_polls = 0
    while True:
        try:
            runs = gh_json(
                [f"repos/{repo}/actions/workflows/{REVIEW_WORKFLOW}/runs?head_sha={head_sha}&event=pull_request&per_page=20"]
            )["workflow_runs"]
        except (RuntimeError, KeyError, json.JSONDecodeError) as error:
            failed_polls += 1
            if failed_polls > FAILED_POLLS_ALLOWED:
                raise RuntimeError(f"{failed_polls} polls of the review failed in a row, the last with: {error}") from error
            sleep(POLL_SECONDS)
            continue
        failed_polls = 0
        latest = max(runs, key=lambda run: run["id"], default=None)
        if latest and latest.get("status") == "completed":
            return latest
        sleep(POLL_SECONDS)


def main() -> int:
    try:
        review = wait(os.environ["GITHUB_REPOSITORY"], os.environ["HEAD_SHA"])
    except RuntimeError as error:
        print(f"::error::{error}")
        return 1
    if review.get("conclusion") != "success":
        print(f"::error::the review ended {review.get('conclusion')}, so this run stops: {review['html_url']}")
        return 1
    print(f"the review passed: {review['html_url']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
