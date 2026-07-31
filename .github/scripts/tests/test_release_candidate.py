import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location(
    "release_candidate", SCRIPTS / "release_candidate.py"
)
rc = importlib.util.module_from_spec(_SPEC)
sys.modules["release_candidate"] = rc
_SPEC.loader.exec_module(rc)


def write_plugin(
    root: Path,
    directory: str,
    plugin_id: str,
    version: str,
    platforms: list[str],
):
    crate = root / "plugins" / directory
    crate.mkdir(parents=True)
    platform_values = ", ".join(f'"{platform}"' for platform in platforms)
    (crate / "plugin.toml").write_text(
        f'[plugin]\nid = "{plugin_id}"\nname = "Fixture"\ndescription = ""\n'
        f'version = "{version}"\nplatforms = [{platform_values}]\n'
    )
    (crate / "Cargo.toml").write_text(
        f'[package]\nname = "{directory}-package"\nversion = "{version}"\n'
    )


def write_host(root: Path, version: str):
    crate = root / "apps/qol-tray"
    crate.mkdir(parents=True)
    (crate / "Cargo.toml").write_text(
        f'[package]\nname = "qol-tray"\nversion = "{version}"\n'
    )


class ReleaseTagTests(unittest.TestCase):
    def test_parses_host_and_plugin_tags(self):
        tags = rc.parse_release_tags(
            "plugin-alt-tab-v1.2.3 qol-tray-v3.41.1"
        )

        self.assertEqual(
            [(tag.unit_id, tag.version) for tag in tags],
            [("plugin-alt-tab", "1.2.3"), ("qol-tray", "3.41.1")],
        )

    def test_rejects_invalid_and_duplicate_tags(self):
        cases = [
            "plugin-alt-tab-v1.2",
            "plugin-alt-tab-v1.2.3;touch-pwned",
            "plugin-alt-tab-v1.2.3 plugin-alt-tab-v1.2.3",
        ]
        for value in cases:
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    rc.parse_release_tags(value)


class CandidatePlanTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def test_derives_plugin_targets_and_host_from_candidate_tree(self):
        write_plugin(
            self.root,
            "fixture",
            "plugin-fixture",
            "1.2.3",
            ["linux", "macos"],
        )
        write_host(self.root, "3.41.1")
        tags = rc.parse_release_tags(
            "plugin-fixture-v1.2.3 qol-tray-v3.41.1"
        )

        plan = rc.candidate_plan(self.root, tags)

        self.assertTrue(plan["has_plugins"])
        self.assertTrue(plan["has_qol_tray"])
        self.assertEqual(plan["qol_tray_tag"], "qol-tray-v3.41.1")
        matrix = plan["plugin_matrix"]["include"]
        self.assertEqual(
            {(entry["os"], entry["target"]) for entry in matrix},
            {
                ("ubuntu-latest", "x86_64-unknown-linux-gnu"),
                ("macos-latest", "aarch64-apple-darwin"),
                ("macos-latest", "x86_64-apple-darwin"),
            },
        )
        self.assertEqual(
            {entry["package"] for entry in matrix}, {"fixture-package"}
        )
        self.assertEqual(
            {entry["tag"] for entry in matrix}, {"plugin-fixture-v1.2.3"}
        )

    def test_rejects_host_manifest_version_mismatch(self):
        write_host(self.root, "3.41.0")
        tags = rc.parse_release_tags("qol-tray-v3.41.1")

        with self.assertRaisesRegex(ValueError, "tag expects version"):
            rc.candidate_plan(self.root, tags)


class SourceCiTests(unittest.TestCase):
    def test_requires_successful_exact_sha_run_and_platform_jobs(self):
        sha = "a" * 40
        runs = [
            {
                "id": 12,
                "head_sha": "b" * 40,
                "conclusion": "success",
                "html_url": "https://example.invalid/12",
                "run_started_at": "2026-07-30T10:00:00Z",
            },
            {
                "id": 13,
                "head_sha": sha,
                "conclusion": "success",
                "html_url": "https://example.invalid/13",
                "run_started_at": "2026-07-30T11:00:00Z",
            },
        ]
        jobs = [
            {"name": "Plan affected crates", "conclusion": "success"},
            {"name": "lint + test (ubuntu-latest)", "conclusion": "success"},
            {"name": "lint + test (macos-latest)", "conclusion": "success"},
            {"name": "sandbox lifecycle (windows)", "conclusion": "skipped"},
        ]

        evidence = rc.require_source_ci(sha, runs, jobs)

        self.assertEqual(evidence["id"], 13)
        self.assertEqual(evidence["html_url"], "https://example.invalid/13")

    def test_rejects_missing_or_incomplete_ci(self):
        sha = "a" * 40
        cases = [
            (
                [{"id": 1, "head_sha": sha, "conclusion": "failure"}],
                [],
            ),
            (
                [{"id": 2, "head_sha": sha, "conclusion": "success"}],
                [
                    {"name": "Plan affected crates", "conclusion": "success"},
                    {
                        "name": "lint + test (ubuntu-latest)",
                        "conclusion": "success",
                    },
                ],
            ),
        ]
        for runs, jobs in cases:
            with self.subTest(runs=runs, jobs=jobs):
                with self.assertRaises(RuntimeError):
                    rc.require_source_ci(sha, runs, jobs)

    @patch.object(rc.plugin_version, "verified_version_bump_parent")
    @patch.object(rc, "gh_json")
    def test_bot_bump_uses_its_verified_parent_ci(
        self,
        gh_json,
        verified_parent,
    ):
        source_sha = "a" * 40
        parent_sha = "b" * 40
        verified_parent.return_value = parent_sha
        run = {
            "id": 42,
            "head_sha": parent_sha,
            "conclusion": "success",
            "html_url": "https://example.invalid/42",
            "run_started_at": "2026-07-30T11:00:00Z",
        }
        jobs = [
            {"name": name, "conclusion": "success"}
            for name in rc.REQUIRED_CI_JOBS
        ]
        gh_json.side_effect = [
            {"workflow_runs": []},
            {"workflow_runs": [run]},
            {"jobs": jobs},
        ]

        evidence = rc.source_ci_evidence(
            "qol-tools/qol",
            source_sha,
            Path("/repo"),
        )

        self.assertEqual(evidence["source_sha"], source_sha)
        self.assertEqual(evidence["ci_sha"], parent_sha)
        self.assertEqual(evidence["id"], 42)
        verified_parent.assert_called_once_with(Path("/repo"), source_sha)

    @patch.object(rc.plugin_version, "verified_version_bump_parent")
    @patch.object(rc, "gh_json")
    def test_incomplete_exact_ci_never_falls_back(
        self,
        gh_json,
        verified_parent,
    ):
        source_sha = "a" * 40
        run = {
            "id": 42,
            "head_sha": source_sha,
            "conclusion": "success",
            "run_started_at": "2026-07-30T11:00:00Z",
        }
        gh_json.side_effect = [
            {"workflow_runs": [run]},
            {"jobs": []},
        ]

        with self.assertRaisesRegex(RuntimeError, "lacks successful jobs"):
            rc.source_ci_evidence(
                "qol-tools/qol",
                source_sha,
                Path("/repo"),
            )

        verified_parent.assert_not_called()

    @patch.object(rc.plugin_version, "verified_version_bump_parent")
    @patch.object(rc, "gh_json")
    def test_failed_exact_ci_never_falls_back(
        self,
        gh_json,
        verified_parent,
    ):
        source_sha = "a" * 40
        run = {
            "id": 42,
            "head_sha": source_sha,
            "conclusion": "failure",
            "run_started_at": "2026-07-30T11:00:00Z",
        }
        gh_json.return_value = {"workflow_runs": [run]}

        with self.assertRaisesRegex(RuntimeError, "CI has not succeeded"):
            rc.source_ci_evidence(
                "qol-tools/qol",
                source_sha,
                Path("/repo"),
            )

        verified_parent.assert_not_called()


class AttestationTests(unittest.TestCase):
    def test_legacy_tags_without_attestations_remain_forward_only(self):
        tag = rc.parse_release_tags("qol-tray-v3.40.6")[0]

        with self.assertRaisesRegex(
            RuntimeError,
            "has no release-candidate attestation",
        ):
            rc.require_attestation(tag, [], "qol-tools/qol")

    def test_requires_latest_success_for_exact_tag_context(self):
        tag = rc.parse_release_tags("qol-tray-v3.41.1")[0]
        statuses = [
            {
                "context": rc.attestation_context(tag),
                "state": "failure",
                "created_at": "2026-07-30T10:00:00Z",
            },
            {
                "context": rc.attestation_context(tag),
                "state": "success",
                "created_at": "2026-07-30T11:00:00Z",
                "target_url": "https://github.com/qol-tools/qol/actions/runs/1",
            },
        ]

        status = rc.require_attestation(tag, statuses, "qol-tools/qol")

        self.assertEqual(status["state"], "success")

    def test_rejects_missing_failed_and_untrusted_attestations(self):
        tag = rc.parse_release_tags("qol-tray-v3.41.1")[0]
        cases = [
            [],
            [
                {
                    "context": rc.attestation_context(tag),
                    "state": "failure",
                    "created_at": "2026-07-30T11:00:00Z",
                }
            ],
            [
                {
                    "context": rc.attestation_context(tag),
                    "state": "success",
                    "created_at": "2026-07-30T11:00:00Z",
                    "target_url": "https://example.invalid/run/1",
                }
            ],
        ]
        for statuses in cases:
            with self.subTest(statuses=statuses):
                with self.assertRaises(RuntimeError):
                    rc.require_attestation(tag, statuses, "qol-tools/qol")

    @patch.object(rc, "gh_json")
    def test_attests_each_tag_on_the_exact_commit(self, gh_json):
        gh_json.side_effect = [{"id": 1}, {"id": 2}]
        sha = "a" * 40
        tags = rc.parse_release_tags(
            "plugin-fixture-v1.2.3 qol-tray-v3.41.1"
        )
        target_url = "https://github.com/qol-tools/qol/actions/runs/42"

        statuses = rc.attest("qol-tools/qol", sha, tags, target_url)

        self.assertEqual(statuses, [{"id": 1}, {"id": 2}])
        for call, tag in zip(gh_json.call_args_list, tags, strict=True):
            self.assertIn(
                f"context={rc.attestation_context(tag)}", call.args[0]
            )
            self.assertIn(
                f"/repos/qol-tools/qol/statuses/{sha}", call.args[0]
            )

    @patch.object(rc.subprocess, "run")
    def test_commit_statuses_reads_every_page(self, run):
        tag = rc.parse_release_tags("qol-tray-v3.41.1")[0]
        first_page = [{"context": f"other/{index}"} for index in range(100)]
        attestation = {
            "context": rc.attestation_context(tag),
            "state": "success",
            "created_at": "2026-07-30T11:00:00Z",
            "target_url": "https://github.com/qol-tools/qol/actions/runs/1",
        }
        run.return_value = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=f"{json.dumps(first_page)}\n{json.dumps([attestation])}\n",
            stderr="",
        )

        statuses = rc.commit_statuses("qol-tools/qol", "a" * 40)
        status = rc.require_attestation(tag, statuses, "qol-tools/qol")

        self.assertEqual(len(statuses), 101)
        self.assertEqual(status, attestation)
        command = run.call_args.args[0]
        self.assertIn("--paginate", command)
        self.assertNotIn("--slurp", command)


class BuildNodeTests(unittest.TestCase):
    def test_build_commands_preserve_release_profile_parity(self):
        cases = [
            (
                "plugin",
                "fixture-package",
                "x86_64-unknown-linux-gnu",
                [
                    [
                        "cargo",
                        "build",
                        "--release",
                        "-p",
                        "fixture-package",
                        "--target",
                        "x86_64-unknown-linux-gnu",
                    ]
                ],
            ),
            (
                "plugin",
                "qol-voice",
                "x86_64-unknown-linux-gnu",
                [
                    [
                        "cargo",
                        "build",
                        "--release",
                        "-p",
                        "qol-voice",
                        "--target",
                        "x86_64-unknown-linux-gnu",
                        "--features",
                        "local-stt",
                    ]
                ],
            ),
            (
                "qol-tray-linux",
                None,
                None,
                [
                    ["cargo", "deb", "-p", "qol-tray"],
                    [
                        "cargo",
                        "run",
                        "--quiet",
                        "-p",
                        "qol-build-identity",
                        "--bin",
                        "qol-build-identity",
                        "--",
                        "verify",
                        "production",
                    ],
                ],
            ),
            (
                "qol-tray-macos",
                None,
                None,
                [
                    [
                        "cargo",
                        "build",
                        "--release",
                        "--target",
                        "aarch64-apple-darwin",
                        "--bin",
                        "qol-tray",
                    ],
                    [
                        "cargo",
                        "build",
                        "--release",
                        "--target",
                        "x86_64-apple-darwin",
                        "--bin",
                        "qol-tray",
                    ],
                    [
                        "cargo",
                        "run",
                        "--quiet",
                        "-p",
                        "qol-build-identity",
                        "--bin",
                        "qol-build-identity",
                        "--",
                        "verify",
                        "production",
                    ],
                ],
            ),
        ]
        for kind, package, target, expected in cases:
            with self.subTest(kind=kind):
                self.assertEqual(
                    rc.build_commands(kind, package, target), expected
                )


if __name__ == "__main__":
    unittest.main()
