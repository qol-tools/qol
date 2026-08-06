#!/usr/bin/env python3
"""Prune GitHub Actions build caches, keeping the newest N per key namespace.

Cache keys carry a trailing lockfile-hash segment, e.g.
`v0-rust-ci-ubuntu-latest-Linux-x64-607b40e9-23b0bf21`, so every Cargo.lock
change deposits a new cache entry under the same logical namespace (the key
with its trailing `-[0-9a-f]{8}` segment stripped). Entries are pruned down
to the newest `--keep` per namespace by creation time, and anything whose
last access is older than `--max-age-days` goes even when it would be kept.
Deletes are by cache id, never by key prefix, so a cache a concurrent build
just recreated under the same prefix is never harmed.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
from collections import defaultdict
from datetime import datetime, timedelta, timezone

MUTATION_PAUSE_SECONDS = 0.5
RETRY_PAUSE_SECONDS = 30
RETRIES = 3

LOCKFILE_HASH = re.compile(r"-[0-9a-f]{8}$")


def namespace_of_key(key: str) -> str:
    """The key with its trailing 8-hex lockfile-hash segment stripped."""
    return LOCKFILE_HASH.sub("", key)


def parse_timestamp(value: str) -> datetime:
    """Parse an ISO-8601 timestamp, treating Z and naive values as UTC."""
    text = f"{value[:-1]}+00:00" if value.endswith("Z") else value
    parsed = datetime.fromisoformat(text)
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def plan_prune(
    caches: list[dict], keep: int, max_age_days: int, now: datetime
) -> list[dict]:
    """Return the entries to delete: past the newest `keep` per namespace,
    plus anything unaccessed for more than `max_age_days`."""
    cutoff = now - timedelta(days=max_age_days)
    grouped: dict[str, list[dict]] = defaultdict(list)
    for cache in caches:
        grouped[namespace_of_key(cache["key"])].append(cache)
    doomed = []
    for entries in grouped.values():
        entries.sort(
            key=lambda cache: parse_timestamp(cache["created_at"]), reverse=True
        )
        for position, cache in enumerate(entries):
            if position >= keep:
                doomed.append(cache)
                continue
            if parse_timestamp(cache["last_accessed_at"]) < cutoff:
                doomed.append(cache)
    return sorted(doomed, key=lambda cache: cache["key"])


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


def gh_api(args: list[str]) -> str:
    result = subprocess.run(
        ["gh", "api", *args], check=True, capture_output=True, text=True
    )
    return result.stdout


GONE_MARKERS = ("HTTP 404",)


def gh_api_mutation(args: list[str]) -> None:
    for attempt in range(1, RETRIES + 1):
        try:
            gh_api(args)
            return
        except subprocess.CalledProcessError as error:
            if any(marker in (error.stderr or "") for marker in GONE_MARKERS):
                return
            if attempt == RETRIES:
                print(error.stderr, file=sys.stderr)
                raise
            time.sleep(RETRY_PAUSE_SECONDS * attempt)


def list_caches(repo: str) -> list[dict]:
    output = gh_api(["--paginate", f"/repos/{repo}/actions/caches"])
    caches = []
    for page in json_stream(output):
        caches.extend(page.get("actions_caches", []))
    return caches


def delete_cache(repo: str, cache_id: int) -> None:
    gh_api_mutation(["-X", "DELETE", f"/repos/{repo}/actions/caches/{cache_id}"])
    time.sleep(MUTATION_PAUSE_SECONDS)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", help="OWNER/NAME; defaults to $GH_REPO")
    parser.add_argument("--keep", type=int, default=2)
    parser.add_argument("--max-age-days", type=int, default=14)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    repo = args.repo or os.environ.get("GH_REPO")
    if repo is None:
        print("no repository: pass --repo OWNER/NAME or set GH_REPO", file=sys.stderr)
        return 1
    if args.keep < 1:
        print("refusing to run with --keep < 1", file=sys.stderr)
        return 1
    if args.max_age_days < 1:
        print("refusing to run with --max-age-days < 1", file=sys.stderr)
        return 1

    try:
        caches = list_caches(repo)
    except subprocess.CalledProcessError as error:
        print(error.stderr or error.stdout, file=sys.stderr)
        return 1

    doomed = plan_prune(
        caches, args.keep, args.max_age_days, datetime.now(timezone.utc)
    )
    total_bytes = sum(cache.get("size_in_bytes", 0) for cache in caches)
    freed_bytes = sum(cache.get("size_in_bytes", 0) for cache in doomed)
    print(
        f"{len(caches)} caches ({total_bytes} bytes), "
        f"keeping {len(caches) - len(doomed)}, pruning {len(doomed)}"
    )
    for cache in doomed:
        key = cache["key"]
        size = cache.get("size_in_bytes", 0)
        if args.dry_run:
            print(f"would delete {key} ({size})")
            continue
        try:
            delete_cache(repo, cache["id"])
        except subprocess.CalledProcessError:
            return 1
        print(f"deleted {key} ({size})")
    print(f"{freed_bytes} bytes freed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
