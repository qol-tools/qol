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
    crate = root / "apps/tray"
    crate.mkdir(parents=True)
    (crate / "Cargo.toml").write_text(
        f'[package]\nname = "qol-tray"\nversion = "{version}"\n'
    )


class ReleaseTagTests(unittest.TestCase):
    def test_parses_host_and_plugin_tags(self):
        tags = rc.parse_release_tags(
            "qol-alt-tab-v1.2.3 qol-tray-v3.41.1"
        )

        self.assertEqual(
            [(tag.unit_id, tag.version) for tag in tags],
            [("qol-alt-tab", "1.2.3"), ("qol-tray", "3.41.1")],
        )

    def test_rejects_invalid_and_duplicate_tags(self):
        cases = [
            "qol-alt-tab-v1.2",
            "qol-alt-tab-v1.2.3;touch-pwned",
            "qol-alt-tab-v1.2.3 qol-alt-tab-v1.2.3",
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
        self.assertEqual(
            {entry["bin_name"] for entry in matrix}, {"fixture-package"}
        )

    def test_rejects_host_manifest_version_mismatch(self):
        write_host(self.root, "3.41.0")
        tags = rc.parse_release_tags("qol-tray-v3.41.1")

        with self.assertRaisesRegex(ValueError, "tag expects version"):
            rc.candidate_plan(self.root, tags)


def write_build_report(directory: Path, kind, package, target, status="pass"):
    directory.mkdir(parents=True)
    (directory / "release-build-report.json").write_text(
        json.dumps(
            {
                "name": "release-build",
                "status": status,
                "inputs": {"kind": kind, "package": package, "target": target},
            }
        )
    )


class CandidateBuildGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        write_plugin(self.root, "fixture", "plugin-fixture", "1.2.3", ["linux", "macos"])
        write_host(self.root, "3.41.1")
        self.tags = rc.parse_release_tags("plugin-fixture-v1.2.3 qol-tray-v3.41.1")
        self.plan = rc.candidate_plan(self.root, self.tags)
        self.reports = self.root / "candidates"

    def tearDown(self):
        self.temp.cleanup()

    def write_every_build(self, skip=None, failed=None):
        builds = [
            ("plugin", "fixture-package", target)
            for target in (
                "x86_64-unknown-linux-gnu",
                "aarch64-apple-darwin",
                "x86_64-apple-darwin",
            )
        ] + [(kind, None, None) for kind in rc.HOST_BUILD_KINDS]
        for index, build in enumerate(builds):
            if build == skip:
                continue
            status = "failed" if build == failed else "pass"
            write_build_report(self.reports / f"candidate-{index}", *build, status)

    def test_every_planned_target_with_a_passing_build_attests(self):
        self.write_every_build()

        rc.require_candidate_builds(self.plan, self.reports)

    def test_a_skipped_or_failed_candidate_refuses_the_attestation(self):
        cases = [
            {"skip": ("plugin", "fixture-package", "x86_64-apple-darwin")},
            {"skip": ("qol-tray-windows", None, None)},
            {"failed": ("plugin", "fixture-package", "aarch64-apple-darwin")},
        ]
        for case in cases:
            with self.subTest(case=case):
                with tempfile.TemporaryDirectory() as reports:
                    self.reports = Path(reports) / "candidates"
                    self.write_every_build(**case)
                    with self.assertRaisesRegex(RuntimeError, "no passing candidate build"):
                        rc.require_candidate_builds(self.plan, self.reports)

    def test_no_reports_at_all_refuses_the_attestation(self):
        with self.assertRaisesRegex(RuntimeError, "plugin/fixture-package/x86_64-unknown-linux-gnu"):
            rc.require_candidate_builds(self.plan, self.root / "missing")

    @patch.object(rc, "gh_json")
    def test_attest_posts_nothing_without_every_candidate_build(self, gh_json):
        plan_report = self.root / "plan.json"
        plan_report.write_text(json.dumps({"artifacts": {"plan": self.plan}}))
        self.write_every_build(skip=("qol-tray-linux", None, None))
        args = rc.build_parser().parse_args(
            [
                "attest",
                "--repo", "qol-tools/qol",
                "--sha", "a" * 40,
                "--tags", "plugin-fixture-v1.2.3 qol-tray-v3.41.1",
                "--target-url", "https://github.com/qol-tools/qol/actions/runs/42",
                "--plan", str(plan_report),
                "--build-reports", str(self.reports),
            ]
        )

        with self.assertRaisesRegex(RuntimeError, "qol-tray-linux"):
            args.handler(args)
        gh_json.assert_not_called()

    @patch.object(rc, "gh_json")
    def test_attest_refuses_a_plan_for_other_tags(self, gh_json):
        plan_report = self.root / "plan.json"
        plan_report.write_text(json.dumps({"artifacts": {"plan": self.plan}}))
        self.write_every_build()
        args = rc.build_parser().parse_args(
            [
                "attest",
                "--repo", "qol-tools/qol",
                "--sha", "a" * 40,
                "--tags", "plugin-fixture-v1.2.3",
                "--target-url", "https://github.com/qol-tools/qol/actions/runs/42",
                "--plan", str(plan_report),
                "--build-reports", str(self.reports),
            ]
        )

        with self.assertRaisesRegex(RuntimeError, "candidate plan covers"):
            args.handler(args)
        gh_json.assert_not_called()


class ReleaseFilesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        write_plugin(self.root, "fixture", "plugin-fixture", "1.2.3", ["linux", "windows"])
        self.tag = rc.parse_release_tags("plugin-fixture-v1.2.3")[0]
        self.files = self.root / "release_files"
        self.files.mkdir()

    def tearDown(self):
        self.temp.cleanup()

    def test_every_target_binary_must_come_from_the_candidate_run(self):
        (self.files / "plugin-fixture-linux-x86_64").write_text("elf")

        with self.assertRaisesRegex(RuntimeError, "plugin-fixture-windows-x86_64.exe"):
            rc.require_release_files(self.root, self.tag, self.files)

        (self.files / "plugin-fixture-windows-x86_64.exe").write_text("pe")
        self.assertEqual(
            rc.require_release_files(self.root, self.tag, self.files),
            ["plugin-fixture-linux-x86_64", "plugin-fixture-windows-x86_64.exe"],
        )


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
            {"name": "lint + test (windows-latest)", "conclusion": "success"},
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

    @patch.object(rc, "gh_json")
    def test_trigger_uses_exact_run_without_workflow_list_lookup(self, gh_json):
        sha = "a" * 40
        run = {
            "id": 42,
            "path": ".github/workflows/ci.yml",
            "event": "push",
            "status": "completed",
            "head_sha": sha,
            "conclusion": "success",
            "html_url": "https://example.invalid/42",
            "run_started_at": "2026-07-30T11:00:00Z",
        }
        jobs = [
            {"name": name, "conclusion": "success"}
            for name in rc.REQUIRED_CI_JOBS
        ]
        gh_json.side_effect = [run, {"jobs": jobs}]

        evidence = rc.source_ci_evidence(
            "qol-tools/qol", sha, Path("/repo"), run_id=42
        )

        self.assertEqual(evidence["id"], 42)
        self.assertEqual(evidence["ci_sha"], sha)
        self.assertEqual(
            gh_json.call_args_list[0].args[0],
            [
                "--method",
                "GET",
                "/repos/qol-tools/qol/actions/runs/42",
            ],
        )

    @patch.object(rc, "gh_json")
    def test_landed_push_uses_the_merge_queue_run_it_reused(self, gh_json):
        sha = "a" * 40
        landed = {
            "id": 42,
            "path": ".github/workflows/ci.yml",
            "event": "push",
            "status": "completed",
            "head_sha": sha,
            "conclusion": "success",
            "html_url": "https://example.invalid/42",
        }
        queued = {
            "id": 41,
            "head_sha": sha,
            "conclusion": "success",
            "html_url": "https://example.invalid/41",
        }
        landed_jobs = [
            {"name": "Plan affected crates", "conclusion": "success"},
            {"name": "lint + test (${{ matrix.os }})", "conclusion": "skipped"},
        ]
        queued_jobs = [
            {"name": name, "conclusion": "success"}
            for name in rc.REQUIRED_CI_JOBS
        ]
        gh_json.side_effect = [
            landed,
            {"jobs": landed_jobs},
            {"workflow_runs": [queued]},
            {"jobs": queued_jobs},
        ]

        evidence = rc.source_ci_evidence(
            "qol-tools/qol", sha, Path("/repo"), run_id=42
        )

        self.assertEqual(evidence["id"], 41)
        self.assertEqual(evidence["ci_sha"], sha)
        self.assertIn("event=merge_group", gh_json.call_args_list[2].args[0])
        self.assertIn(f"head_sha={sha}", gh_json.call_args_list[2].args[0])

    @patch.object(rc, "gh_json")
    def test_queue_run_that_reused_the_pull_request_needs_its_release_builds(
        self, gh_json
    ):
        sha = "a" * 40
        landed = {
            "id": 42,
            "path": ".github/workflows/ci.yml",
            "event": "push",
            "status": "completed",
            "head_sha": sha,
            "conclusion": "success",
        }
        queued = {"id": 41, "head_sha": sha, "conclusion": "success"}
        plan = {"name": "Plan affected crates", "conclusion": "success"}
        skipped = {"name": "lint + test (${{ matrix.os }})", "conclusion": "skipped"}
        ubuntu = {"name": "release build (ubuntu-latest)", "conclusion": "success"}
        macos = {"name": "release build (macos-latest)", "conclusion": "success"}
        windows = {"name": "release build (windows-latest)", "conclusion": "success"}
        cases = [
            ([plan, skipped, ubuntu, macos, windows], None),
            ([plan, skipped, ubuntu, windows], "release build \\(macos-latest\\)"),
            ([plan, skipped, ubuntu, macos], "release build \\(windows-latest\\)"),
            (
                [plan, skipped, ubuntu, {**macos, "conclusion": "failure"}, windows],
                "release build \\(macos-latest\\)",
            ),
            ([plan, skipped], "lacks successful jobs"),
        ]
        for queue_jobs, message in cases:
            with self.subTest(queue_jobs=queue_jobs):
                gh_json.side_effect = [
                    landed,
                    {"jobs": [plan, skipped]},
                    {"workflow_runs": [queued]},
                    {"jobs": queue_jobs},
                ]
                if message is None:
                    evidence = rc.source_ci_evidence(
                        "qol-tools/qol", sha, Path("/repo"), run_id=42
                    )
                    self.assertEqual(evidence["id"], 41)
                    continue
                with self.assertRaisesRegex(RuntimeError, message):
                    rc.source_ci_evidence(
                        "qol-tools/qol", sha, Path("/repo"), run_id=42
                    )

    @patch.object(rc, "gh_json")
    def test_landed_push_rejects_a_missing_or_incomplete_queue_run(self, gh_json):
        sha = "a" * 40
        landed = {
            "id": 42,
            "path": ".github/workflows/ci.yml",
            "event": "push",
            "status": "completed",
            "head_sha": sha,
            "conclusion": "success",
        }
        landed_jobs = [
            {"name": "Plan affected crates", "conclusion": "success"},
            {"name": "lint + test (${{ matrix.os }})", "conclusion": "skipped"},
        ]
        queued = {"id": 41, "head_sha": sha, "conclusion": "success"}
        cases = [
            ([], [], "reused a merge queue verdict"),
            (
                [{**queued, "conclusion": "failure"}],
                [],
                "reused a merge queue verdict",
            ),
            (
                [queued],
                [{"name": "Plan affected crates", "conclusion": "success"}],
                "lacks successful jobs",
            ),
        ]
        for queue_runs, queue_jobs, message in cases:
            with self.subTest(queue_runs=queue_runs, queue_jobs=queue_jobs):
                gh_json.side_effect = [
                    landed,
                    {"jobs": landed_jobs},
                    {"workflow_runs": queue_runs},
                    {"jobs": queue_jobs},
                ]
                with self.assertRaisesRegex(RuntimeError, message):
                    rc.source_ci_evidence(
                        "qol-tools/qol", sha, Path("/repo"), run_id=42
                    )

    @patch.object(rc, "gh_json")
    def test_batched_queue_run_needs_the_batch_release_build_check(self, gh_json):
        sha = "a" * 40
        landed = {
            "id": 42,
            "path": ".github/workflows/ci.yml",
            "event": "push",
            "status": "completed",
            "head_sha": sha,
            "conclusion": "success",
        }
        landed_jobs = [
            {"name": "Plan affected crates", "conclusion": "success"},
            {"name": "lint + test (${{ matrix.os }})", "conclusion": "skipped"},
        ]
        queued = {"id": 41, "head_sha": sha, "conclusion": "success"}
        queue_jobs = [
            {"name": "Plan affected crates", "conclusion": "success"},
            {"name": "lint + test (ubuntu-latest)", "conclusion": "success"},
            {"name": "lint + test (macos-latest)", "conclusion": "success"},
            {"name": "lint + test (windows-latest)", "conclusion": "success"},
            {"name": "batch role", "conclusion": "success"},
            {"name": "release build (${{ matrix.os }})", "conclusion": "skipped"},
        ]
        trusted_plan = {
            "name": "Plan affected crates",
            "conclusion": "success",
            "steps": [{"name": rc.TRUSTED_PLAN_STEP, "conclusion": "success"}],
        }
        reused_jobs = [
            trusted_plan,
            {"name": "lint + test (${{ matrix.os }})", "conclusion": "skipped"},
            {"name": "batch role", "conclusion": "success"},
            {"name": "release build (${{ matrix.os }})", "conclusion": "skipped"},
        ]
        untrusted_jobs = [
            {"name": "Plan affected crates", "conclusion": "success"},
            *reused_jobs[1:],
        ]
        skipped_trust_jobs = [
            {
                **trusted_plan,
                "steps": [{"name": rc.TRUSTED_PLAN_STEP, "conclusion": "skipped"}],
            },
            *reused_jobs[1:],
        ]
        cases = [
            (queue_jobs, [{"conclusion": "success"}], None),
            (reused_jobs, [{"conclusion": "success"}], None),
            (queue_jobs, [{"conclusion": "failure"}], "no batch release build covered"),
            (queue_jobs, [], "no batch release build covered"),
            (reused_jobs, [], "no batch release build covered"),
            (untrusted_jobs, [{"conclusion": "success"}], "without trusting a pull request run"),
            (skipped_trust_jobs, [{"conclusion": "success"}], "without trusting a pull request run"),
        ]
        for jobs, checks, message in cases:
            with self.subTest(jobs=len(jobs), checks=checks):
                gh_json.reset_mock()
                gh_json.side_effect = [
                    landed,
                    {"jobs": landed_jobs},
                    {"workflow_runs": [queued]},
                    {"jobs": jobs},
                    {"check_runs": checks},
                ]
                if message is None:
                    evidence = rc.source_ci_evidence(
                        "qol-tools/qol", sha, Path("/repo"), run_id=42
                    )
                    self.assertEqual(evidence["id"], 41)
                    self.assertIn(
                        "check_name=batch release build",
                        gh_json.call_args_list[4].args[0],
                    )
                    continue
                with self.assertRaisesRegex(RuntimeError, message):
                    rc.source_ci_evidence(
                        "qol-tools/qol", sha, Path("/repo"), run_id=42
                    )

    def test_skipped_checks_count_as_reuse_only_beside_a_passed_plan(self):
        plan = {"name": "Plan affected crates", "conclusion": "success"}
        skipped = {"name": "lint + test (${{ matrix.os }})", "conclusion": "skipped"}
        cases = [
            ([plan, skipped], True),
            ([skipped], False),
            ([plan], False),
            ([{**plan, "conclusion": "failure"}, skipped], False),
            (
                [
                    plan,
                    skipped,
                    {"name": "lint + test (macos-latest)", "conclusion": "failure"},
                ],
                False,
            ),
        ]
        for jobs, expected in cases:
            with self.subTest(jobs=jobs):
                self.assertEqual(rc.reused_queue_verdict(jobs), expected)

    def test_trigger_rejects_wrong_run_provenance(self):
        sha = "a" * 40
        fields = {
            "id": 42,
            "path": ".github/workflows/ci.yml",
            "event": "push",
            "status": "completed",
            "head_sha": sha,
            "conclusion": "success",
        }
        cases = [
            ("id", 41),
            ("path", ".github/workflows/release.yml"),
            ("event", "workflow_dispatch"),
            ("status", "in_progress"),
            ("head_sha", "b" * 40),
            ("conclusion", "failure"),
        ]
        for name, value in cases:
            with self.subTest(name=name):
                run = {**fields, name: value}
                with self.assertRaisesRegex(RuntimeError, "invalid provenance"):
                    rc.require_triggered_provenance(run, 42, sha)

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
                "creator": {"login": "github-actions[bot]"},
            },
        ]

        status = rc.require_attestation(tag, statuses, "qol-tools/qol")

        self.assertEqual(status["state"], "success")

    def test_attestation_names_the_run_that_holds_the_candidate_files(self):
        repo = "qol-tools/qol"
        cases = [
            ("https://github.com/qol-tools/qol/actions/runs/42", 42),
            ("https://github.com/qol-tools/qol/actions/runs/42/attempts/2", 42),
            ("https://github.com/other/qol/actions/runs/42", None),
            ("https://github.com/qol-tools/qol/actions/runs/", None),
        ]
        for url, expected in cases:
            with self.subTest(url=url):
                self.assertEqual(rc.attestation_run_id(url, repo), expected)

    def candidate_run(self, **changes):
        repository = {"full_name": "qol-tools/qol"}
        run = {
            "id": 77,
            "path": ".github/workflows/plugin-version.yml",
            "event": "workflow_run",
            "head_branch": "main",
            "repository": repository,
            "head_repository": repository,
        }
        return {**run, **changes}

    def test_candidate_files_come_only_from_a_versioning_run_on_main(self):
        rc.require_candidate_run(self.candidate_run(), 77, "qol-tools/qol")
        rc.require_candidate_run(
            self.candidate_run(event="workflow_dispatch"), 77, "qol-tools/qol"
        )
        cases = [
            ({"id": 78}, "id"),
            ({"path": ".github/workflows/ci.yml"}, "path"),
            ({"event": "pull_request"}, "event"),
            ({"head_branch": "feature"}, "head_branch"),
            ({"head_repository": {"full_name": "fork/qol"}}, "head_repository"),
        ]
        for changes, failed in cases:
            with self.subTest(failed=failed):
                with self.assertRaisesRegex(RuntimeError, failed):
                    rc.require_candidate_run(
                        self.candidate_run(**changes), 77, "qol-tools/qol"
                    )

    @patch.object(rc, "exact_ci_run")
    @patch.object(rc, "commit_statuses")
    def test_verify_hands_the_candidate_run_to_the_publish_job(self, statuses, exact_run):
        tag = rc.parse_release_tags("qol-tray-v3.41.1")[0]
        statuses.return_value = [
            {
                "context": rc.attestation_context(tag),
                "state": "success",
                "created_at": "2026-07-30T11:00:00Z",
                "target_url": "https://github.com/qol-tools/qol/actions/runs/77",
                "creator": {"login": "github-actions[bot]"},
            }
        ]
        exact_run.return_value = self.candidate_run()
        with tempfile.TemporaryDirectory() as temp:
            output = Path(temp) / "github-output"
            args = rc.build_parser().parse_args(
                [
                    "verify",
                    "--repo", "qol-tools/qol",
                    "--sha", "a" * 40,
                    "--tag", tag.value,
                    "--github-output", str(output),
                ]
            )
            args.handler(args)

            self.assertEqual(output.read_text(), "run_id=77\n")

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
                    "creator": {"login": "github-actions[bot]"},
                }
            ],
            [
                {
                    "context": rc.attestation_context(tag),
                    "state": "success",
                    "created_at": "2026-07-30T11:00:00Z",
                    "target_url": "https://github.com/qol-tools/qol/actions/runs/1",
                    "creator": {"login": "someone"},
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
            "creator": {"login": "github-actions[bot]"},
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
        verify = [
            "cargo",
            "run",
            "--quiet",
            "--locked",
            "-p",
            "qol-build-identity",
            "--bin",
            "qol-build-identity",
            "--",
            "verify",
            "production",
        ]
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
                        "--locked",
                        "-p",
                        "fixture-package",
                        "--target",
                        "x86_64-unknown-linux-gnu",
                    ],
                    verify,
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
                        "--locked",
                        "-p",
                        "qol-voice",
                        "--target",
                        "x86_64-unknown-linux-gnu",
                        "--features",
                        "local-stt",
                        "--features",
                        "sherpa-stt",
                    ],
                    verify,
                ],
            ),
            (
                "qol-tray-linux",
                None,
                None,
                [
                    ["cargo", "deb", "-p", "qol-tray", "--locked"],
                    verify,
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
                        "--locked",
                        "--target",
                        "aarch64-apple-darwin",
                        "--bin",
                        "qol-tray",
                    ],
                    [
                        "cargo",
                        "build",
                        "--release",
                        "--locked",
                        "--target",
                        "x86_64-apple-darwin",
                        "--bin",
                        "qol-tray",
                    ],
                    verify,
                ],
            ),
            (
                "qol-tray-windows",
                None,
                None,
                [
                    [
                        "cargo",
                        "build",
                        "--release",
                        "--locked",
                        "--target",
                        "x86_64-pc-windows-msvc",
                        "--bin",
                        "qol-tray",
                        "--bin",
                        "qol-tray-install",
                    ],
                    verify,
                ],
            ),
        ]
        root = SCRIPTS.parents[1]
        for kind, package, target, expected in cases:
            with self.subTest(kind=kind, package=package):
                self.assertEqual(
                    rc.build_commands(root, kind, package, target), expected
                )


if __name__ == "__main__":
    unittest.main()
