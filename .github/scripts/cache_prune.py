#!/usr/bin/env python3
"""Prune GitHub Actions build caches.

Dependency cache keys carry a trailing lockfile-hash segment, e.g.
`v0-rust-ci-ubuntu-latest-Linux-x64-607b40e9-23b0bf21`, so every Cargo.lock
change deposits a new entry under the same logical namespace (the key with
its trailing `-[0-9a-f]{8}` segment stripped). Entries are grouped per ref
and namespace, pruned to the newest `--keep` by creation time, dropped when
unaccessed for `--max-age-days`, and then evicted least recently used first
until the total fits `--max-bytes`.
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
MAX_CACHE_BYTES = 9 * 1024**3

LOCKFILE_HASH = re.compile(r"-[0-9a-f]{8}$")


def namespace_of_key(key: str) -> str:
    return LOCKFILE_HASH.sub("", key)


def parse_timestamp(value: str) -> datetime:
    """Parse an ISO-8601 timestamp, treating Z and naive values as UTC."""
    text = f"{value[:-1]}+00:00" if value.endswith("Z") else value
    parsed = datetime.fromisoformat(text)
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


def cache_order(cache: dict) -> tuple[datetime, int]:
    return parse_timestamp(cache["created_at"]), cache["id"]


def plan_prune(
    caches: list[dict], keep: int, max_age_days: int, now: datetime,
    max_bytes: int = MAX_CACHE_BYTES,
) -> list[dict]:
    cutoff = now - timedelta(days=max_age_days)
    grouped: dict[tuple[str, str], list[dict]] = defaultdict(list)
    for cache in caches:
        grouped[(cache.get("ref", ""), namespace_of_key(cache["key"]))].append(cache)
    doomed = []
    newest_ids = set()
    for entries in grouped.values():
        entries.sort(key=cache_order, reverse=True)
        fresh = [cache for cache in entries if parse_timestamp(cache["last_accessed_at"]) >= cutoff]
        doomed.extend(cache for cache in entries if parse_timestamp(cache["last_accessed_at"]) < cutoff)
        if not fresh:
            continue
        newest_ids.add(fresh[0]["id"])
        doomed.extend(fresh[keep:])
    doomed_ids = {cache["id"] for cache in doomed}
    survivors = [cache for cache in caches if cache["id"] not in doomed_ids]
    doomed.extend(over_budget(survivors, newest_ids, max_bytes))
    return sorted(doomed, key=lambda cache: cache["key"])


def over_budget(caches: list[dict], newest_ids: set[int], max_bytes: int) -> list[dict]:
    remaining_bytes = sum(cache.get("size_in_bytes", 0) for cache in caches)
    candidates = sorted(
        caches,
        key=lambda cache: (
            cache["id"] in newest_ids,
            parse_timestamp(cache["last_accessed_at"]),
            parse_timestamp(cache["created_at"]),
            cache["id"],
        ),
    )
    doomed = []
    for cache in candidates:
        if remaining_bytes <= max_bytes:
            break
        doomed.append(cache)
        remaining_bytes -= cache.get("size_in_bytes", 0)
    return doomed


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
    parser = argparse.ArgumentParser(
        description="Prune GitHub Actions caches by age, revision count, and total bytes."
    )
    parser.add_argument("--repo", help="OWNER/NAME; defaults to $GH_REPO")
    parser.add_argument("--keep", type=int, default=1)
    parser.add_argument("--max-age-days", type=int, default=14)
    parser.add_argument("--max-bytes", type=int, default=MAX_CACHE_BYTES)
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
    if args.max_bytes < 1:
        print("refusing to run with --max-bytes < 1", file=sys.stderr)
        return 1

    try:
        caches = list_caches(repo)
    except subprocess.CalledProcessError as error:
        print(error.stderr or error.stdout, file=sys.stderr)
        return 1

    doomed = plan_prune(
        caches, args.keep, args.max_age_days, datetime.now(timezone.utc), args.max_bytes
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
