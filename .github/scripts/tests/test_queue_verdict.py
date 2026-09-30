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


class ReusableTests(unittest.TestCase):
    PASSED = [{"name": "tests", "conclusion": "success"}]

    def test_identical_tree_with_a_passed_run_is_reused(self):
        self.assertTrue(qv.reusable("t1", "t1", self.PASSED, "tests"))

    def test_a_moved_main_changes_the_tree(self):
        self.assertFalse(qv.reusable("t2", "t1", self.PASSED, "tests"))

    def test_no_passed_run_means_no_reuse(self):
        self.assertFalse(qv.reusable("t1", "t1", [], "tests"))
        failed = [{"name": "tests", "conclusion": "failure"}]
        self.assertFalse(qv.reusable("t1", "t1", failed, "tests"))

    def test_another_workflow_passing_does_not_count(self):
        other = [{"name": "Versioning", "conclusion": "success"}]
        self.assertFalse(qv.reusable("t1", "t1", other, "tests"))


if __name__ == "__main__":
    unittest.main()
