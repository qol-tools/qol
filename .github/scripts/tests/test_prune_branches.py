import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

_SPEC = importlib.util.spec_from_file_location(
    "prune_branches", Path(__file__).resolve().parents[1] / "prune_branches.py"
)
pb = importlib.util.module_from_spec(_SPEC)
sys.modules["prune_branches"] = pb
_SPEC.loader.exec_module(pb)

TIP = "a" * 40
MERGE = "m" * 40


def pull(number=1, state="MERGED", head="feature", oid=TIP, base="main", cross=False, merge=MERGE):
    return {
        "number": number,
        "state": state,
        "headRefName": head,
        "headRefOid": oid,
        "baseRefName": base,
        "isCrossRepository": cross,
        "mergeCommit": {"oid": merge} if merge else None,
    }


def in_main(sha):
    return sha == MERGE


class PlanPrune(unittest.TestCase):
    def test_only_an_exact_tip_merged_into_the_default_branch_goes(self):
        cases = [
            ("exact tip merged into main", [pull()], True),
            ("commits pushed after the merge", [pull(oid="b" * 40)], False),
            ("merged into a side branch", [pull(base="side")], False),
            ("merged from a fork with the same name", [pull(cross=True)], False),
            ("merge commit not in main", [pull(merge="x" * 40)], False),
            ("merge commit missing", [pull(merge=None)], False),
            ("closed without merging", [pull(state="CLOSED")], False),
            ("no pull request", [], False),
            ("open pull request from the branch", [pull(), pull(2, "OPEN", merge=None)], False),
            ("open pull request into the branch", [pull(), pull(2, "OPEN", head="child", base="feature", merge=None)], False),
            ("older pull request differs, newer matches", [pull(oid="c" * 40), pull(2)], True),
        ]
        for name, pulls, deleted in cases:
            with self.subTest(case=name):
                plan = pb.plan_prune({"feature": TIP}, pulls, "main", in_main)
                self.assertEqual(plan != [], deleted)

    def test_never_plans_the_default_branch(self):
        plan = pb.plan_prune({"main": TIP}, [pull(head="main")], "main", in_main)
        self.assertEqual(plan, [])

    def test_plan_names_the_branch_tip_and_pull_request(self):
        plan = pb.plan_prune({"b": TIP, "a": TIP}, [pull(7, head="a"), pull(8, head="b")], "main", in_main)
        self.assertEqual(plan, [("a", TIP, 7), ("b", TIP, 8)])


class Delete(unittest.TestCase):
    def test_pushes_the_delete_with_a_lease_on_the_planned_tip(self):
        with patch.object(pb.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, "", "")) as run:
            self.assertTrue(pb.delete("feature", TIP))
        self.assertEqual(
            run.call_args.args[0],
            ["git", "push", f"--force-with-lease=refs/heads/feature:{TIP}", "origin", ":refs/heads/feature"],
        )

    def test_a_refused_push_keeps_the_branch(self):
        with patch.object(pb.subprocess, "run", return_value=subprocess.CompletedProcess([], 1, "", "stale info")):
            self.assertFalse(pb.delete("feature", TIP))


if __name__ == "__main__":
    unittest.main()
