#!/usr/bin/env python3
"""Drop non-user-facing entries from GitHub-generated release notes.

Reads a release body on stdin and writes it to stdout without the
"What's Changed" bullets whose pull request title is not releasable under
plugin_version.py's rules (feat, fix, perf, or breaking) or whose scope is
ci. ci, refactor, docs, test, chore and the like never reach the releases
page.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

import plugin_version as pv

ENTRY_RE = re.compile(r"^\* (?P<title>.+?) by @\S+ in (?P<link>\S+)$")
PULL_LINK_RE = re.compile(r"/pull/(?P<number>[0-9]+)$")
PULL_SUBJECT_RE = re.compile(r"\(#(?P<number>[0-9]+)\)$")
INTERNAL_SCOPES = {"ci"}
EMPTY_NOTE = "* No user-facing changes."


def is_user_facing(title: str) -> bool:
    scope = pv.SCOPE_RE.match(title)
    if scope is not None and scope.group("scope").lower() in INTERNAL_SCOPES:
        return False
    return pv.is_releasable_commit(pv.Commit(sha="", subject=title, body=""))


def pull_numbers(commits: list[pv.Commit]) -> set[str]:
    return {
        match.group("number")
        for commit in commits
        if (match := PULL_SUBJECT_RE.search(commit.subject))
    }


def unit_pulls(root: Path, unit_id: str, rev_range: str) -> set[str]:
    packages = pv.load_packages(root)
    [unit] = pv.discover_release_units(root, packages, unit_id)
    return pull_numbers(
        pv.relevant_commits(
            root, pv.load_commits(root, rev_range), unit, packages, {}, {}, {}, {}
        )
    )


def belongs(link: str, pulls: set[str] | None) -> bool:
    if pulls is None:
        return True
    match = PULL_LINK_RE.search(link)
    return match is None or match.group("number") in pulls


def filter_notes(body: str, pulls: set[str] | None = None) -> str:
    kept: list[str] = []
    header = None
    entries = 0
    in_changes = False
    for line in body.splitlines():
        if line.startswith("## "):
            in_changes = line.strip() == "## What's Changed"
            if in_changes:
                header = len(kept)
        entry = ENTRY_RE.match(line) if in_changes else None
        if entry is not None:
            entries += 1
            if not is_user_facing(entry.group("title")) or not belongs(
                entry.group("link"), pulls
            ):
                continue
        kept.append(line)
    if header is not None and entries and not any(
        ENTRY_RE.match(line) for line in kept[header + 1 :]
    ):
        kept.insert(header + 1, EMPTY_NOTE)
    return "\n".join(kept) + ("\n" if body.endswith("\n") else "")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--unit")
    parser.add_argument("--range", dest="rev_range")
    args = parser.parse_args()
    if bool(args.unit) != bool(args.rev_range):
        parser.error("--unit and --range go together")
    pulls = (
        unit_pulls(args.root.resolve(), args.unit, args.rev_range) if args.unit else None
    )
    sys.stdout.write(filter_notes(sys.stdin.read(), pulls))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
