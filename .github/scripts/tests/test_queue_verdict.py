import importlib.util
import sys
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location("queue_verdict", SCRIPTS / "queue_verdict.py")
qv = importlib.util.module_from_spec(_SPEC)
sys.modules["queue_verdict"] = qv
_SPEC.loader.exec_module(qv)

SHA = "d40f472d6" + "0" * 31


class QueuedPullTests(unittest.TestCase):
    def test_queue_branch_names_its_pull_request(self):
        self.assertEqual(qv.queued_pull(f"refs/heads/gh-readonly-queue/main/pr-40-{SHA}"), 40)

    def test_base_branch_with_slashes_still_resolves(self):
        self.assertEqual(
            qv.queued_pull(f"refs/heads/gh-readonly-queue/release/v3/pr-7-{SHA}"), 7
        )

    def test_other_branches_have_no_pull_request(self):
        self.assertIsNone(qv.queued_pull("refs/heads/main"))
        self.assertIsNone(qv.queued_pull("refs/heads/gh-readonly-queue/main/pr-40-short"))


class PassedTests(unittest.TestCase):
    def test_a_passed_queue_run_of_this_workflow_counts(self):
        self.assertTrue(qv.passed([{"name": "tests", "conclusion": "success"}], "tests"))

    def test_other_workflows_and_failures_do_not_count(self):
        runs = [
            {"name": "Versioning", "conclusion": "success"},
            {"name": "tests", "conclusion": "failure"},
        ]
        self.assertFalse(qv.passed(runs, "tests"))
        self.assertFalse(qv.passed([], "tests"))


if __name__ == "__main__":
    unittest.main()
