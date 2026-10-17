#!/usr/bin/env python3
"""Drop non-user-facing entries from GitHub-generated release notes.

Reads a release body on stdin and writes it to stdout without the
"What's Changed" bullets whose pull request title is not releasable under
plugin_version.py's rules (feat, fix, perf, or breaking) or whose scope is
ci. ci, refactor, docs, test, chore and the like never reach the releases
page.
"""

from __future__ import annotations

import re
import sys

import plugin_version as pv

ENTRY_RE = re.compile(r"^\* (?P<title>.+?) by @\S+ in \S+$")
SCOPE_RE = re.compile(r"^[a-z0-9_-]+\((?P<scope>[^)]+)\)!?:", re.IGNORECASE)
INTERNAL_SCOPES = {"ci"}
EMPTY_NOTE = "* No user-facing changes."


def is_user_facing(title: str) -> bool:
    scope = SCOPE_RE.match(title)
    if scope is not None and scope.group("scope").lower() in INTERNAL_SCOPES:
        return False
    return pv.is_releasable_commit(pv.Commit(sha="", subject=title, body=""))


def filter_notes(body: str) -> str:
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
            if not is_user_facing(entry.group("title")):
                continue
        kept.append(line)
    if header is not None and entries and not any(
        ENTRY_RE.match(line) for line in kept[header + 1 :]
    ):
        kept.insert(header + 1, EMPTY_NOTE)
    return "\n".join(kept) + ("\n" if body.endswith("\n") else "")


def main() -> int:
    sys.stdout.write(filter_notes(sys.stdin.read()))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
