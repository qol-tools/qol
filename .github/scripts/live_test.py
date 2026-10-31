#!/usr/bin/env python3

import os
import re
import sys
from pathlib import Path

from queue_verdict import gh_json

CHECK_NAME = "live test"
PATHS_FILE = Path(__file__).resolve().parents[1] / "live-test-paths"
NEEDED_TITLE = "Live test needed"
NOT_NEEDED_TITLE = "No live test needed"
LISTED_FILES = 50


def patterns(text: str) -> list[str]:
    return [line.strip() for line in text.splitlines() if line.strip()]


def pattern_regex(pattern: str) -> re.Pattern[str]:
    out = []
    index = 0
    while index < len(pattern):
        if pattern.startswith("**/", index):
            out.append("(?:.*/)?")
            index += 3
        elif pattern.startswith("**", index):
            out.append(".*")
            index += 2
        elif pattern[index] == "*":
            out.append("[^/]*")
            index += 1
        elif pattern[index] == "?":
            out.append("[^/]")
            index += 1
        else:
            out.append(re.escape(pattern[index]))
            index += 1
    return re.compile("".join(out) + r"\Z")


def needing(files: list[str], listed: list[str]) -> list[str]:
    compiled = [pattern_regex(pattern) for pattern in listed]
    return [path for path in files if any(regex.match(path) for regex in compiled)]


def summary(found: list[str]) -> tuple[str, str]:
    if not found:
        return NOT_NEEDED_TITLE, "No changed file is on `.github/live-test-paths`, so passing checks are enough."
    shown = [f"- `{path}`" for path in found[:LISTED_FILES]]
    if len(found) > LISTED_FILES:
        shown.append(f"- and {len(found) - LISTED_FILES} more")
    lead = "These changed files are on `.github/live-test-paths`, so a person must see the change working before it merges:"
    return NEEDED_TITLE, "\n".join([lead, "", *shown])


def changed_files(repo: str, pr: str) -> list[str]:
    pages = gh_json(["--paginate", "--slurp", f"repos/{repo}/pulls/{pr}/files?per_page=100"])
    return [entry["filename"] for page in pages for entry in page]


def post(repo: str, sha: str, title: str, text: str, url: str) -> None:
    gh_json(
        [
            "--method",
            "POST",
            f"repos/{repo}/check-runs",
            "-f",
            f"name={CHECK_NAME}",
            "-f",
            f"head_sha={sha}",
            "-f",
            "status=completed",
            "-f",
            "conclusion=success",
            "-f",
            f"details_url={url}",
            "-f",
            f"output[title]={title}",
            "-f",
            f"output[summary]={text}",
        ]
    )


def main() -> int:
    repo = os.environ["REPO"]
    files = changed_files(repo, os.environ["PR_NUMBER"])
    found = needing(files, patterns(PATHS_FILE.read_text(encoding="utf-8")))
    title, text = summary(found)
    post(repo, os.environ["HEAD_SHA"], title, text, os.environ.get("RUN_URL", ""))
    print(f"{title}: {len(found)} of {len(files)} changed files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
