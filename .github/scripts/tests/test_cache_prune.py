import contextlib
import importlib.util
import io
import json
import os
import sys
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path
from unittest.mock import patch

_SPEC = importlib.util.spec_from_file_location(
    "cache_prune", Path(__file__).resolve().parents[1] / "cache_prune.py"
)
cp = importlib.util.module_from_spec(_SPEC)
sys.modules["cache_prune"] = cp
_SPEC.loader.exec_module(cp)

NOW = datetime(2025, 6, 1, 12, 0, tzinfo=timezone.utc)
_IDS = iter(range(1, 1000))


def stamp(dt: datetime) -> str:
    return dt.strftime("%Y-%m-%dT%H:%M:%SZ")


def cache_entry(key, created_at, last_accessed_at=None, size_in_bytes=1000):
    return {
        "id": next(_IDS),
        "ref": "refs/heads/main",
        "key": key,
        "version": "v1",
        "created_at": created_at,
        "last_accessed_at": (
            last_accessed_at if last_accessed_at is not None else created_at
        ),
        "size_in_bytes": size_in_bytes,
    }


class NamespaceOfKey(unittest.TestCase):
    def test_strips_trailing_hex_segment(self):
        cases = [
            ("v0-rust-ci-ubuntu-latest-Linux-x64-607b40e9-23b0bf21",
             "v0-rust-ci-ubuntu-latest-Linux-x64-607b40e9"),
            ("ci-ubuntu-latest-1a2b3c4d", "ci-ubuntu-latest"),
            ("qol-tray-linux-607b40e9-23b0bf21", "qol-tray-linux-607b40e9"),
        ]
        for key, expected in cases:
            self.assertEqual(cp.namespace_of_key(key), expected, f"key: {key}")

    def test_non_matching_keys_keep_whole_key(self):
        cases = [
            "ci-ubuntu-latest",
            "release-candidate-x86_64-unknown-linux-gnu",
            "ci-windows-sandbox",
            "key-abcdefgh",
            "key-ABCDEF12",
            "key-1234567",
        ]
        for key in cases:
            self.assertEqual(cp.namespace_of_key(key), key, f"key: {key}")


class PlanPrune(unittest.TestCase):
    def test_keep_newest_per_namespace(self):
        entries = [
            cache_entry(
                "ci-ubuntu-latest-607b40e9-11111111",
                stamp(NOW - timedelta(days=3)),
            ),
            cache_entry(
                "ci-ubuntu-latest-607b40e9-22222222",
                stamp(NOW - timedelta(days=2)),
            ),
            cache_entry(
                "ci-ubuntu-latest-607b40e9-33333333",
                stamp(NOW - timedelta(days=1)),
            ),
            cache_entry(
                "ci-macos-latest-607b40e9-99999999",
                stamp(NOW - timedelta(days=1)),
            ),
        ]
        doomed = cp.plan_prune(entries, keep=2, max_age_days=14, now=NOW)
        self.assertEqual(
            [cache["key"] for cache in doomed],
            ["ci-ubuntu-latest-607b40e9-11111111"],
        )

    def test_stale_rule_beats_keep_rule(self):
        entries = [
            cache_entry(
                "ci-macos-latest-607b40e9-11111111",
                stamp(NOW - timedelta(days=1)),
                last_accessed_at=stamp(NOW - timedelta(days=20)),
            ),
            cache_entry(
                "ci-macos-latest-607b40e9-22222222",
                stamp(NOW - timedelta(days=1)),
            ),
        ]
        doomed = cp.plan_prune(entries, keep=2, max_age_days=14, now=NOW)
        self.assertEqual(
            [cache["key"] for cache in doomed],
            ["ci-macos-latest-607b40e9-11111111"],
        )

    def test_age_boundary_exactly_max_age_kept(self):
        exactly_max_age = cache_entry(
            "ci-linux-607b40e9-11111111",
            stamp(NOW - timedelta(days=1)),
            last_accessed_at=stamp(NOW - timedelta(days=14)),
        )
        strictly_older = cache_entry(
            "ci-linux-607b40e9-22222222",
            stamp(NOW - timedelta(days=1)),
            last_accessed_at=stamp(NOW - timedelta(days=14) - timedelta(seconds=1)),
        )
        doomed = cp.plan_prune(
            [exactly_max_age, strictly_older], keep=2, max_age_days=14, now=NOW
        )
        self.assertEqual(
            [cache["key"] for cache in doomed],
            ["ci-linux-607b40e9-22222222"],
        )


class Fixture(unittest.TestCase):
    NAMESPACES = [
        "ci-ubuntu-latest",
        "ci-macos-latest",
        "release-candidate-x86_64-unknown-linux-gnu",
        "release-candidate-aarch64-apple-darwin",
        "release-candidate-x86_64-apple-darwin",
        "qol-tray-candidate-linux",
        "qol-tray-candidate-macos",
        "qol-tray-linux",
        "qol-tray-macos",
        "ci-windows-sandbox",
    ]

    def test_plan_on_observed_key_shapes(self):
        caches = []
        for ns in self.NAMESPACES:
            caches.append(
                cache_entry(
                    f"{ns}-607b40e9-11111111", stamp(NOW - timedelta(days=2))
                )
            )
            caches.append(
                cache_entry(
                    f"{ns}-607b40e9-22222222", stamp(NOW - timedelta(days=1))
                )
            )
        caches.append(cache_entry("qol-tray-linux", stamp(NOW - timedelta(days=1))))
        caches.append(
            cache_entry(
                "ci-windows-sandbox-607b40e9-33333333",
                stamp(NOW - timedelta(days=1)),
                last_accessed_at=stamp(NOW - timedelta(days=20)),
            )
        )

        doomed = cp.plan_prune(caches, keep=1, max_age_days=14, now=NOW)
        doomed_keys = {cache["key"] for cache in doomed}
        self.assertEqual(len(doomed), len(self.NAMESPACES) + 1)
        for ns in self.NAMESPACES:
            self.assertIn(f"{ns}-607b40e9-11111111", doomed_keys, ns)
            self.assertNotIn(f"{ns}-607b40e9-22222222", doomed_keys, ns)
        self.assertNotIn("qol-tray-linux", doomed_keys)
        self.assertIn("ci-windows-sandbox-607b40e9-33333333", doomed_keys)


class Main(unittest.TestCase):
    def test_dry_run_prints_plan_without_deleting(self):
        now = datetime.now(timezone.utc)
        caches = [
            cache_entry(
                f"ci-ubuntu-latest-607b40e9-{hex_hash:08x}",
                stamp(now - timedelta(days=days)),
            )
            for days, hex_hash in [(0, 0xAAAA), (1, 0xBBBB), (2, 0xCCCC)]
        ]
        payload = json.dumps({"total_count": 3, "actions_caches": caches})
        with (
            patch.object(cp, "gh_api", return_value=payload) as gh,
            patch.object(cp, "delete_cache") as delete,
            patch("sys.argv", ["cache_prune.py", "--repo", "owner/repo",
                               "--dry-run", "--keep", "1"]),
            patch.dict(os.environ, {}, clear=True),
            contextlib.redirect_stdout(io.StringIO()) as out,
        ):
            self.assertEqual(cp.main(), 0)
        lines = out.getvalue().strip().splitlines()
        self.assertEqual(lines[0], "3 caches (3000 bytes), keeping 1, pruning 2")
        self.assertIn(
            "would delete ci-ubuntu-latest-607b40e9-0000bbbb (1000)", lines
        )
        self.assertIn(
            "would delete ci-ubuntu-latest-607b40e9-0000cccc (1000)", lines
        )
        self.assertEqual(lines[-1], "2000 bytes freed")
        self.assertNotIn("deleted ", out.getvalue())
        gh.assert_called_once_with(["--paginate", "/repos/owner/repo/actions/caches"])
        delete.assert_not_called()

    def test_prune_mode_deletes_by_id(self):
        now = datetime.now(timezone.utc)
        caches = [
            cache_entry(
                "ci-ubuntu-latest-607b40e9-aaaa1111",
                stamp(now - timedelta(days=1)),
                size_in_bytes=500,
            ),
            cache_entry("ci-ubuntu-latest-607b40e9-bbbb2222", stamp(now)),
        ]
        payload = json.dumps({"total_count": 2, "actions_caches": caches})
        with (
            patch.object(cp, "gh_api", return_value=payload) as gh,
            patch.object(cp, "delete_cache") as delete,
            patch("sys.argv", ["cache_prune.py", "--repo", "owner/repo",
                               "--keep", "1"]),
            patch.dict(os.environ, {}, clear=True),
            contextlib.redirect_stdout(io.StringIO()) as out,
        ):
            self.assertEqual(cp.main(), 0)
        delete.assert_called_once_with("owner/repo", caches[0]["id"])
        self.assertIn("deleted ci-ubuntu-latest-607b40e9-aaaa1111 (500)", out.getvalue())

    def test_delete_cache_hits_delete_endpoint_by_id(self):
        with patch.object(cp, "gh_api") as gh:
            cp.delete_cache("owner/repo", 42)
        gh.assert_called_once_with(
            ["-X", "DELETE", "/repos/owner/repo/actions/caches/42"]
        )

    def test_requires_repo_or_gh_repo(self):
        with (
            patch("sys.argv", ["cache_prune.py", "--dry-run"]),
            patch.dict(os.environ, {}, clear=True),
            patch.object(cp, "gh_api") as gh,
            contextlib.redirect_stderr(io.StringIO()),
        ):
            self.assertEqual(cp.main(), 1)
        gh.assert_not_called()

    def test_gh_repo_env_default(self):
        now = datetime.now(timezone.utc)
        caches = [
            cache_entry("ci-macos-latest-607b40e9-12345678", stamp(now))
        ]
        payload = json.dumps({"total_count": 1, "actions_caches": caches})
        with (
            patch("sys.argv", ["cache_prune.py", "--dry-run"]),
            patch.dict(os.environ, {"GH_REPO": "env/org-repo"}, clear=True),
            patch.object(cp, "gh_api", return_value=payload) as gh,
            contextlib.redirect_stdout(io.StringIO()),
        ):
            self.assertEqual(cp.main(), 0)
        gh.assert_called_once_with(["--paginate", "/repos/env/org-repo/actions/caches"])

    def test_keep_below_one_refused(self):
        with (
            patch("sys.argv", ["cache_prune.py", "--repo", "owner/repo", "--keep", "0"]),
            patch.dict(os.environ, {}, clear=True),
            patch.object(cp, "gh_api") as gh,
        ):
            self.assertEqual(cp.main(), 1)
        gh.assert_not_called()


if __name__ == "__main__":
    unittest.main()
