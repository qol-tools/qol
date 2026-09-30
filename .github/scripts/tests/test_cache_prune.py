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
    def test_generated_compiler_keys_match_the_restore_namespace(self):
        for runner, arch in [("Linux", "X64"), ("macOS", "ARM64")]:
            config = cp.compiler_cache_config({
                "RUNNER_OS": runner, "RUNNER_ARCH": arch,
                "RUNNER_TEMP": "/tmp/compiler cache", "GITHUB_SHA": "a" * 40,
            })
            self.assertEqual(cp.namespace_of_key(config["key"]) + "-", config["prefix"])
            self.assertEqual(config["capacity"], str(cp.COMPILER_CACHE_BYTES))

    def test_strips_trailing_hex_segment(self):
        cases = [
            ("v0-rust-ci-ubuntu-latest-Linux-x64-607b40e9-23b0bf21",
             "v0-rust-ci-ubuntu-latest-Linux-x64-607b40e9"),
            ("ci-ubuntu-latest-1a2b3c4d", "ci-ubuntu-latest"),
            ("qol-tray-linux-607b40e9-23b0bf21", "qol-tray-linux-607b40e9"),
            (f"qol-compiler-v1-Linux-X64-{'a' * 40}", "qol-compiler-v1-Linux-X64"),
            (f"qol-compiler-v1-macOS-ARM64-{'b' * 40}", "qol-compiler-v1-macOS-ARM64"),
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
            f"unrelated-{'a' * 40}",
            f"qol-compiler-v1-Linux-X64-{'g' * 40}",
        ]
        for key in cases:
            self.assertEqual(cp.namespace_of_key(key), key, f"key: {key}")


class PlanPrune(unittest.TestCase):
    def test_budget_edge_cases(self):
        old = cache_entry("ci-linux-11111111", stamp(NOW), size_in_bytes=1000)
        new = cache_entry("ci-linux-22222222", stamp(NOW), size_in_bytes=1000)
        unknown = cache_entry("ci-macos-11111111", stamp(NOW))
        del unknown["size_in_bytes"]
        cases = [
            ([], 1, []),
            ([unknown], 1, []),
            ([old, new], 2000, []),
            ([new, old], 1000, [old]),
            ([old, new], 1000, [old]),
            ([unknown, old], 1000, []),
        ]
        for entries, budget, expected in cases:
            with self.subTest(entries=[c["id"] for c in entries], budget=budget):
                self.assertEqual(cp.plan_prune(entries, 2, 14, NOW, budget), expected)

    def test_compiler_cache_yields_to_current_dependency_cache(self):
        release = cache_entry("plugin-release-linux-11111111", stamp(NOW - timedelta(days=5)))
        compiler = cache_entry(f"{cp.COMPILER_CACHE_PREFIX}Linux-X64-{'a' * 40}", stamp(NOW))
        self.assertEqual(cp.plan_prune([release, compiler], 2, 14, NOW, 1000), [compiler])

    def test_retention_counts_do_not_mix_cache_refs(self):
        key = f"{cp.COMPILER_CACHE_PREFIX}Linux-X64-"
        main = cache_entry(key + "a" * 40, stamp(NOW - timedelta(days=2)))
        old_pr = cache_entry(key + "b" * 40, stamp(NOW - timedelta(days=1)))
        new_pr = cache_entry(key + "c" * 40, stamp(NOW))
        old_pr["ref"] = new_pr["ref"] = "refs/pull/44/merge"
        self.assertEqual(cp.plan_prune([main, old_pr, new_pr], 1, 14, NOW), [old_pr])

    def test_size_budget_discards_superseded_caches_before_distinct_namespaces(self):
        entries = [
            cache_entry("ci-linux-11111111", stamp(NOW - timedelta(days=2))),
            cache_entry("ci-linux-22222222", stamp(NOW - timedelta(days=1))),
            cache_entry("ci-macos-11111111", stamp(NOW - timedelta(days=3))),
        ]
        for ceiling, expected in [(3000, []), (2000, [entries[0]])]:
            with self.subTest(ceiling=ceiling):
                self.assertEqual(cp.plan_prune(entries, 2, 14, NOW, ceiling), expected)

    def test_size_budget_falls_back_to_oldest_access_for_distinct_namespaces(self):
        entries = [
            cache_entry("ci-linux-11111111", stamp(NOW - timedelta(days=3)), stamp(NOW)),
            cache_entry("ci-macos-11111111", stamp(NOW), stamp(NOW - timedelta(days=1))),
        ]
        doomed = cp.plan_prune(entries, 2, 14, NOW, max_bytes=1000)
        self.assertEqual(doomed, [entries[1]])

    def test_expired_cache_bytes_do_not_evict_healthy_caches_twice(self):
        entries = [
            cache_entry("ci-linux-11111111", stamp(NOW - timedelta(days=20))),
            cache_entry("ci-macos-11111111", stamp(NOW)),
        ]
        doomed = cp.plan_prune(entries, 2, 14, NOW, max_bytes=1000)
        self.assertEqual(doomed, [entries[0]])

    def test_oversized_single_cache_cannot_exceed_the_budget(self):
        entry = cache_entry("ci-linux-11111111", stamp(NOW), size_in_bytes=1001)
        self.assertEqual(cp.plan_prune([entry], 2, 14, NOW, max_bytes=1000), [entry])

    def test_compiler_archives_keep_recent_commits_per_platform(self):
        linux = "qol-compiler-v1-Linux-X64"
        macos = "qol-compiler-v1-macOS-ARM64"
        entries = [
            cache_entry(f"{linux}-{'a' * 40}", stamp(NOW - timedelta(days=3))),
            cache_entry(f"{linux}-{'b' * 40}", stamp(NOW - timedelta(days=2))),
            cache_entry(f"{linux}-{'c' * 40}", stamp(NOW - timedelta(days=1))),
            cache_entry(f"{macos}-{'a' * 40}", stamp(NOW - timedelta(days=3))),
            cache_entry("ci-linux-607b40e9-12345678", stamp(NOW - timedelta(days=3))),
        ]
        doomed = cp.plan_prune(entries, keep=2, max_age_days=14, now=NOW)
        self.assertEqual([entry["key"] for entry in doomed], [entries[0]["key"]])

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
    def test_compiler_config_needs_no_repository_or_github_access(self):
        environment = {"RUNNER_OS": "Linux", "RUNNER_ARCH": "X64",
                       "RUNNER_TEMP": "/tmp/compiler cache", "GITHUB_SHA": "a" * 40}
        with (
            patch("sys.argv", ["cache_prune.py", "--compiler-cache-config"]),
            patch.dict(os.environ, environment, clear=True),
            patch.object(cp, "gh_api") as gh,
            contextlib.redirect_stdout(io.StringIO()) as out,
        ):
            self.assertEqual(cp.main(), 0)
        config = dict(line.split("=", 1) for line in out.getvalue().splitlines())
        self.assertEqual(config, cp.compiler_cache_config(environment))
        gh.assert_not_called()

    def test_compiler_config_rejects_missing_or_multiline_inputs(self):
        base = {"RUNNER_OS": "Linux", "RUNNER_ARCH": "X64",
                "RUNNER_TEMP": "/tmp/cache", "GITHUB_SHA": "a" * 40}
        for overrides in [{"RUNNER_TEMP": "/tmp/cache\ninjected=1"}, {"GITHUB_SHA": "bad"}, {"RUNNER_OS": "Linux\n"}]:
            with self.subTest(overrides=overrides), self.assertRaises(ValueError):
                cp.compiler_cache_config(base | overrides)
        with self.assertRaises(KeyError):
            cp.compiler_cache_config({})

    def test_max_bytes_reaches_the_deletion_plan(self):
        now = datetime.now(timezone.utc)
        entries = [
            cache_entry("ci-linux-11111111", stamp(now - timedelta(days=1))),
            cache_entry("ci-linux-22222222", stamp(now)),
        ]
        with (
            patch("sys.argv", ["cache_prune.py", "--repo", "owner/repo", "--max-bytes", "1000"]),
            patch.object(cp, "list_caches", return_value=entries),
            patch.object(cp, "delete_cache") as delete,
            contextlib.redirect_stdout(io.StringIO()),
        ):
            self.assertEqual(cp.main(), 0)
        delete.assert_called_once_with("owner/repo", entries[0]["id"])

    def test_retire_mode_only_retires_archives_older_than_this_commits_saves(self):
        now = datetime.now(timezone.utc)
        linux = cp.COMPILER_CACHE_PREFIX + "Linux-X64-"
        macos = cp.COMPILER_CACHE_PREFIX + "macOS-ARM64-"
        old_linux = cache_entry(linux + "a" * 40, stamp(now - timedelta(days=1)))
        old_macos = cache_entry(macos + "a" * 40, stamp(now - timedelta(days=1)))
        saved_linux = cache_entry(linux + "b" * 40, stamp(now))
        saved_macos = cache_entry(macos + "b" * 40, stamp(now))
        expired = cache_entry("ci-linux-11111111", stamp(now - timedelta(days=30)),
                              size_in_bytes=cp.MAX_CACHE_BYTES)
        foreign = dict(old_linux, id=next(_IDS), ref="refs/pull/44/merge")
        everything = [old_linux, old_macos, saved_linux, saved_macos, expired, foreign]
        for entries, expected in [
            (everything, {old_linux["id"], old_macos["id"]}),
            ([old_linux, old_macos, saved_linux, expired], {old_linux["id"]}),
            ([old_linux, old_macos, expired, foreign], set()),
            ([], set()),
        ]:
            with (
                self.subTest(entries=[entry["key"] for entry in entries]),
                patch("sys.argv", ["cache_prune.py", "--repo", "owner/repo",
                                   "--retire-compiler-caches", "b" * 40, "--ref", "refs/heads/main"]),
                patch.object(cp, "list_caches", return_value=entries),
                patch.object(cp, "delete_cache") as delete,
                contextlib.redirect_stdout(io.StringIO()),
            ):
                self.assertEqual(cp.main(), 0)
                self.assertEqual({call.args[1] for call in delete.call_args_list}, expected)

    def test_retire_mode_requires_a_commit_sha_and_explicit_ref(self):
        for arguments in [["--retire-compiler-caches", "b" * 39, "--ref", "refs/heads/main"],
                          ["--retire-compiler-caches", "B" * 40, "--ref", "refs/heads/main"],
                          ["--retire-compiler-caches", "b" * 40]]:
            with (
                self.subTest(arguments=arguments),
                patch("sys.argv", ["cache_prune.py", "--repo", "owner/repo", *arguments]),
                patch.object(cp, "gh_api") as gh,
                contextlib.redirect_stderr(io.StringIO()),
            ):
                self.assertEqual(cp.main(), 1)
                gh.assert_not_called()

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

    def test_nonpositive_byte_budget_is_refused(self):
        for limit in ["0", "-1"]:
            with (
                self.subTest(limit=limit),
                patch("sys.argv", ["cache_prune.py", "--repo", "owner/repo", "--max-bytes", limit]),
                patch.object(cp, "gh_api") as gh,
                contextlib.redirect_stderr(io.StringIO()),
            ):
                self.assertEqual(cp.main(), 1)
                gh.assert_not_called()


class CompilerRetirement(unittest.TestCase):
    def test_replacement_only_retires_older_archives_in_the_same_namespace_and_ref(self):
        prefix = cp.COMPILER_CACHE_PREFIX + "Linux-X64-"
        old = cache_entry(prefix + "a" * 40, stamp(NOW - timedelta(days=1)))
        saved = cache_entry(prefix + "b" * 40, stamp(NOW))
        newer = cache_entry(prefix + "c" * 40, stamp(NOW))
        foreign = dict(old, id=next(_IDS), ref="refs/pull/44/merge")
        macos = cache_entry(f"{cp.COMPILER_CACHE_PREFIX}macOS-ARM64-{'a' * 40}", old["created_at"])
        release = cache_entry("plugin-release-linux-12345678", old["created_at"])
        entries = [newer, foreign, macos, release, saved, old]
        self.assertEqual(cp.replaced_compiler_caches(entries, saved["key"], saved["ref"]), [old])
        self.assertEqual(cp.replaced_compiler_caches(entries, newer["key"], newer["ref"]), [saved, old])

    def test_missing_replacement_never_retires_existing_archives(self):
        key = f"{cp.COMPILER_CACHE_PREFIX}Linux-X64-{'a' * 40}"
        foreign = cache_entry(key, stamp(NOW))
        foreign["ref"] = "refs/pull/44/merge"
        for entries in [[], [foreign]]:
            with self.subTest(entries=entries), self.assertRaises(LookupError):
                cp.replaced_compiler_caches(entries, key, "refs/heads/main")

    def test_upload_bursts_keep_one_archive_per_platform_and_current_releases(self):
        release = cache_entry("plugin-release-linux-12345678", stamp(NOW),
                              size_in_bytes=cp.MAX_CACHE_BYTES - 2 * cp.COMPILER_CACHE_BYTES)
        entries = [release]
        for push in range(20):
            now = NOW + timedelta(minutes=push)
            doomed = {c["id"] for c in cp.plan_prune(entries, 2, 14, now)}
            entries = [c for c in entries if c["id"] not in doomed]
            saves = [
                cache_entry(f"{cp.COMPILER_CACHE_PREFIX}{platform}-{push:040x}", stamp(now),
                            size_in_bytes=cp.COMPILER_CACHE_BYTES)
                for platform in ["Linux-X64", "macOS-ARM64"]
            ]
            entries.extend(saves)
            for saved in reversed(saves):
                retired = {c["id"] for c in cp.replaced_compiler_caches(entries, saved["key"], saved["ref"])}
                entries = [c for c in entries if c["id"] not in retired]
                doomed = {c["id"] for c in cp.plan_prune(entries, 2, 14, now)}
                entries = [c for c in entries if c["id"] not in doomed]
            self.assertIn(release, entries)
            self.assertEqual(len(entries), 3)
            self.assertLessEqual(sum(c["size_in_bytes"] for c in entries), cp.MAX_CACHE_BYTES)


if __name__ == "__main__":
    unittest.main()
