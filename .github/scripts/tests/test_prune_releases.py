import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

_SPEC = importlib.util.spec_from_file_location(
    "prune_releases", Path(__file__).resolve().parents[1] / "prune_releases.py"
)
pr = importlib.util.module_from_spec(_SPEC)
sys.modules["prune_releases"] = pr
_SPEC.loader.exec_module(pr)


class ParseTag(unittest.TestCase):
    def test_component_and_version_extraction(self):
        cases = [
            ("plugin-alt-tab-v0.29.1", ("plugin-alt-tab", (0, 29, 1))),
            ("qol-tray-v3.18.2", ("qol-tray", (3, 18, 2))),
            ("qol-shot-v1.15.0", ("qol-shot", (1, 15, 0))),
            ("v1.2.3", None),
            ("plugin-alt-tab-v0.29", None),
            ("plugin-alt-tab-v0.29.1-rc1", None),
            ("random-tag", None),
        ]
        for tag, expected in cases:
            self.assertEqual(pr.parse_tag(tag), expected, f"tag: {tag}")


class PlanPrune(unittest.TestCase):
    def test_keeps_newest_per_component_and_ignores_foreign_tags(self):
        tags = [
            "plugin-a-v1.0.0",
            "plugin-a-v1.2.0",
            "plugin-a-v1.10.0",
            "plugin-a-v1.9.0",
            "plugin-b-v0.1.0",
            "plugin-b-v0.2.0",
            "baseline-marker",
        ]
        doomed = pr.plan_prune(tags, keep=2)
        self.assertEqual(doomed, ["plugin-a-v1.0.0", "plugin-a-v1.2.0"])

    def test_semver_ordering_beats_lexicographic(self):
        tags = ["p-v0.9.0", "p-v0.10.0", "p-v0.2.0"]
        doomed = pr.plan_prune(tags, keep=2)
        self.assertEqual(doomed, ["p-v0.2.0"])

    def test_keep_larger_than_count_prunes_nothing(self):
        self.assertEqual(pr.plan_prune(["p-v1.0.0", "p-v1.1.0"], keep=3), [])


class ReleaseIdForTag(unittest.TestCase):
    def test_returns_id_when_release_exists(self):
        with patch.object(pr, "gh_api", return_value='{"id": 42}') as mocked:
            self.assertEqual(pr.release_id_for_tag("p-v1.0.0"), 42)
        mocked.assert_called_once()

    def test_returns_none_on_missing_release(self):
        error = subprocess.CalledProcessError(1, [], stderr="gh: Not Found (HTTP 404)")
        with patch.object(pr, "gh_api", side_effect=error) as mocked:
            self.assertIsNone(pr.release_id_for_tag("p-v1.0.0"))
        mocked.assert_called_once()

    def test_retries_transient_errors_then_succeeds(self):
        error = subprocess.CalledProcessError(1, [], stderr="gh: Internal Server Error (HTTP 500)")
        with (
            patch.object(pr, "gh_api", side_effect=[error, '{"id": 7}']),
            patch("time.sleep"),
        ):
            self.assertEqual(pr.release_id_for_tag("p-v1.0.0"), 7)

    def test_raises_after_retries_on_persistent_error(self):
        error = subprocess.CalledProcessError(1, [], stderr="gh: Internal Server Error (HTTP 500)")
        with patch.object(pr, "gh_api", side_effect=error), patch("time.sleep"), self.assertRaises(subprocess.CalledProcessError):
            pr.release_id_for_tag("p-v1.0.0")


class DeleteTag(unittest.TestCase):
    def test_deletes_release_then_tag(self):
        calls = []
        def fake_gh_api(args):
            calls.append(args)
            if "releases/tags/" in args[0] and "-X" not in args:
                return '{"id": 5}'
            return ""
        with (
            patch.object(pr, "gh_api", side_effect=fake_gh_api),
            patch("time.sleep"),
        ):
            pr.delete_tag("p-v1.0.0")
        deletes = [" ".join(a) for a in calls if "-X" in a]
        self.assertEqual(len(deletes), 2)
        self.assertIn("releases/5", deletes[0])
        self.assertIn("git/refs/tags/p-v1.0.0", deletes[1])

    def test_self_heals_when_tag_delete_refused_by_surviving_release(self):
        calls = []
        def fake_gh_api(args):
            calls.append(args)
            joined = " ".join(args)
            if "releases/tags/" in joined and "-X" not in joined:
                return '{"id": 5}'
            if "git/refs/tags/" in joined:
                if sum(1 for a in calls if "git/refs/tags/" in " ".join(a)) == 1:
                    raise subprocess.CalledProcessError(
                        1,
                        [],
                        stderr="Repository rule violations found\nCannot delete this tag\n (HTTP 422)",
                    )
                return ""
            return ""
        with (
            patch.object(pr, "gh_api", side_effect=fake_gh_api),
            patch("time.sleep"),
        ):
            pr.delete_tag("p-v1.0.0")
        deletes = [" ".join(a) for a in calls if "-X" in a]
        self.assertEqual(len(deletes), 4)
        self.assertIn("releases/5", deletes[0])
        self.assertIn("git/refs/tags/p-v1.0.0", deletes[1])
        self.assertIn("releases/5", deletes[2])
        self.assertIn("git/refs/tags/p-v1.0.0", deletes[3])

    def test_accepts_already_gone_release_and_tag(self):
        error = subprocess.CalledProcessError(1, [], stderr="gh: Not Found (HTTP 404)")
        def fake_gh_api(args):
            if "-X" in args:
                raise error
            return '{"id": 5}'
        with patch.object(pr, "gh_api", side_effect=fake_gh_api), patch("time.sleep"):
            pr.delete_tag("p-v1.0.0")

    def test_persistent_refusal_raises_after_retries(self):
        calls = []

        def fake_gh_api(args):
            calls.append(args)
            joined = " ".join(args)
            if "releases/tags/" in joined and "-X" not in joined:
                return '{"id": 5}'
            if "git/refs/tags/" in joined:
                raise subprocess.CalledProcessError(
                    1, [], stderr="Cannot delete this tag (HTTP 422)"
                )
            return ""

        with (
            patch.object(pr, "gh_api", side_effect=fake_gh_api),
            patch("time.sleep"),
            self.assertRaises(subprocess.CalledProcessError),
        ):
            pr.delete_tag("p-v1.0.0")
        tag_deletes = [a for a in calls if "git/refs/tags/" in " ".join(a)]
        self.assertEqual(len(tag_deletes), 3)

    def test_rule_refusal_without_release_raises_immediately(self):
        calls = []

        def fake_gh_api(args):
            calls.append(args)
            joined = " ".join(args)
            if "releases/tags/" in joined and "-X" not in joined:
                if sum(1 for a in calls if "releases/tags/" in " ".join(a)) == 1:
                    return '{"id": 5}'
                raise subprocess.CalledProcessError(
                    1, [], stderr="gh: Not Found (HTTP 404)"
                )
            if "git/refs/tags/" in joined:
                raise subprocess.CalledProcessError(
                    1,
                    [],
                    stderr="Repository rule violations found\nCannot delete this tag\n (HTTP 422)",
                )
            return ""

        with (
            patch.object(pr, "gh_api", side_effect=fake_gh_api),
            patch("time.sleep"),
            self.assertRaisesRegex(pr.RuleRefusedError, "repository rule"),
        ):
            pr.delete_tag("p-v1.0.0")
        tag_deletes = [a for a in calls if "git/refs/tags/" in " ".join(a)]
        self.assertEqual(len(tag_deletes), 1)

    def test_tag_paths_are_percent_encoded(self):
        calls = []

        def fake_gh_api(args):
            calls.append(args)
            joined = " ".join(args)
            if "releases/tags/" in joined and "-X" not in joined:
                return '{"id": 5}'
            return ""

        with patch.object(pr, "gh_api", side_effect=fake_gh_api), patch("time.sleep"):
            pr.delete_tag("p-v1.0.0#x-v1.2.3")
        joined_all = " ".join(" ".join(a) for a in calls)
        self.assertIn("releases/tags/p-v1.0.0%23x-v1.2.3", joined_all)
        self.assertIn("git/refs/tags/p-v1.0.0%23x-v1.2.3", joined_all)
        self.assertNotIn("#x-v1.2.3", joined_all)


class CurrentLatestTag(unittest.TestCase):
    def test_returns_tag_when_release_exists(self):
        with patch.object(pr, "gh_api", return_value="qol-tray-v3.55.4\n"):
            self.assertEqual(pr.current_latest_tag(), "qol-tray-v3.55.4")

    def test_returns_none_when_no_latest(self):
        error = subprocess.CalledProcessError(1, [], stderr="gh: Not Found (HTTP 404)")
        with patch.object(pr, "gh_api", side_effect=error):
            self.assertIsNone(pr.current_latest_tag())


class Main(unittest.TestCase):
    def test_continues_after_failure_and_reports_it(self):
        tags = ["p-a-v4.0.0", "p-a-v3.0.0", "p-a-v2.0.0", "p-a-v1.0.0"]
        with patch("sys.argv", ["prune", "--keep", "1"]):
            with patch.object(pr, "list_remote_tags", return_value=tags):
                with patch.object(pr, "current_latest_tag", return_value=None):
                    with patch.object(
                        pr, "delete_tag", side_effect=[None, RuntimeError("boom"), None]
                    ) as delete_mock:
                        code = pr.main()
        self.assertEqual(code, 1)
        self.assertEqual(delete_mock.call_count, 3)

    def test_success_returns_zero(self):
        tags = ["p-a-v4.0.0", "p-a-v3.0.0", "p-a-v2.0.0", "p-a-v1.0.0"]
        with (
            patch("sys.argv", ["prune", "--keep", "1"]),
            patch.object(pr, "list_remote_tags", return_value=tags),
            patch.object(pr, "current_latest_tag", return_value=None),
            patch.object(pr, "delete_tag"),
        ):
            self.assertEqual(pr.main(), 0)

    def test_latest_holder_is_never_pruned(self):
        tags = ["p-a-v4.0.0", "p-a-v3.0.0", "p-a-v2.0.0", "p-a-v1.0.0"]
        with patch("sys.argv", ["prune", "--keep", "1"]):
            with patch.object(pr, "list_remote_tags", return_value=tags):
                with patch.object(pr, "current_latest_tag", return_value="p-a-v2.0.0"):
                    with patch.object(pr, "delete_tag") as delete_mock:
                        code = pr.main()
        self.assertEqual(code, 0)
        deleted = [call.args[0] for call in delete_mock.call_args_list]
        self.assertEqual(deleted, ["p-a-v1.0.0", "p-a-v3.0.0"])

    def test_rule_refused_tags_are_reported_not_failed(self):
        tags = ["p-a-v4.0.0", "p-a-v3.0.0", "p-a-v2.0.0", "p-a-v1.0.0"]
        with patch("sys.argv", ["prune", "--keep", "1"]):
            with patch.object(pr, "list_remote_tags", return_value=tags):
                with patch.object(pr, "current_latest_tag", return_value=None):
                    with patch.object(
                        pr,
                        "delete_tag",
                        side_effect=[pr.RuleRefusedError("rule"), None, None],
                    ) as delete_mock:
                        code = pr.main()
        self.assertEqual(code, 0)
        self.assertEqual(delete_mock.call_count, 3)
