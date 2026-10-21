import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))


def load(name):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


ac = load("affected_crates")
qt = load("queue_trust")

REPO = "qol-tools/qol"
RUN = subprocess.run
PR_HEAD = "b" * 40
TESTED_BASE = "c" * 40
QUEUE_BASE = "d" * 40
QUEUE_HEAD = "e" * 40
HEAD_REF = f"refs/heads/gh-readonly-queue/main/pr-7-{'f' * 40}"
GRAPH = {
    "lib-a": {"dir": "libs/a", "deps": set(), "doctest": False},
    "app-a": {"dir": "apps/a", "deps": {"lib-a"}, "doctest": True},
    "lib-b": {"dir": "libs/b", "deps": set(), "doctest": False},
    "app-b": {"dir": "apps/b", "deps": {"lib-b"}, "doctest": False},
    "app-ab": {"dir": "apps/ab", "deps": {"lib-a", "lib-b"}, "doctest": False},
}
PASSED_RUN = {"id": 30}
GATE_PASSED = [{"name": "merge gate", "conclusion": "success"}]


def pr_plan(crates):
    return ac.affected_outputs(set(crates), GRAPH)


class QueueTrustTests(unittest.TestCase):
    def setUp(self):
        self.runs = [PASSED_RUN]
        self.jobs = GATE_PASSED
        self.record = {"head": PR_HEAD, "base": TESTED_BASE, "plan": pr_plan({"app-a"})}
        self.downloaded = True
        self.graph = GRAPH
        self.diffs = {
            (TESTED_BASE, QUEUE_BASE): ["libs/b/src/lib.rs"],
            (QUEUE_BASE, QUEUE_HEAD): ["apps/a/src/main.rs"],
        }
        self.downloads = []
        patches = [
            patch.object(qt, "gh_json", side_effect=self.fake_gh_json),
            patch.object(qt.subprocess, "run", side_effect=self.fake_run),
            patch.object(ac, "workspace_graph", side_effect=lambda: self.graph),
            patch.object(ac, "changed_files", side_effect=lambda base, head: self.diffs[(base, head)]),
        ]
        for patcher in patches:
            patcher.start()
            self.addCleanup(patcher.stop)

    def fake_gh_json(self, args):
        path = args[2] if args[0] == "--method" else args[0]
        if path == f"repos/{REPO}/pulls/7":
            return {"head": {"sha": PR_HEAD}}
        if path == f"repos/{REPO}/actions/workflows/ci.yml/runs":
            self.assertIn(f"head_sha={PR_HEAD}", args)
            self.assertIn("event=pull_request", args)
            return {"workflow_runs": self.runs}
        if path.startswith(f"repos/{REPO}/actions/runs/") and "/jobs" in path:
            return {"jobs": self.jobs}
        raise AssertionError(f"unexpected gh api call: {args}")

    def fake_run(self, cmd, **kwargs):
        if cmd[0] == "gh":
            self.downloads.append(cmd)
            if not self.downloaded:
                return subprocess.CompletedProcess(cmd, 1, "", "no artifact")
            directory = Path(cmd[cmd.index("--dir") + 1])
            (directory / "tested-base.json").write_text(json.dumps(self.record))
            return subprocess.CompletedProcess(cmd, 0, "", "")
        if cmd[0] == "git":
            return RUN(cmd, **kwargs)
        with patch.dict(os.environ, kwargs["env"], clear=True):
            ac.main()
        return subprocess.CompletedProcess(cmd, 0, "", "")

    def decide(self):
        return qt.decide(REPO, HEAD_REF, QUEUE_BASE, QUEUE_HEAD)

    def assert_runs_the_queue_plan(self, plan):
        self.assertIs(plan["trusted"], False)
        self.assertEqual(plan["crates"], "app-a")
        self.assertIn("-p app-a", plan["ubuntu_test"])

    def test_no_passed_pull_request_run_runs_the_queue_plan(self):
        cases = [
            ([], GATE_PASSED),
            ([PASSED_RUN], []),
            ([PASSED_RUN], [{"name": "merge gate", "conclusion": "failure"}]),
            ([PASSED_RUN], [{"name": "queue gate", "conclusion": "success"}]),
        ]
        for runs, jobs in cases:
            with self.subTest(runs=runs, jobs=jobs):
                self.runs, self.jobs, self.downloads = runs, jobs, []
                self.assert_runs_the_queue_plan(self.decide())
                self.assertEqual(self.downloads, [])

    def test_missing_or_foreign_record_runs_the_queue_plan(self):
        cases = [
            (False, self.record),
            (True, {**self.record, "head": "a" * 40}),
            (True, {"head": PR_HEAD, "plan": self.record["plan"]}),
            (True, {"head": PR_HEAD, "base": TESTED_BASE}),
            (True, {**self.record, "base": "--output=/tmp/diff"}),
            (True, {**self.record, "base": "HEAD"}),
        ]
        for base in ["--output=/tmp/diff", "HEAD"]:
            self.diffs[(base, QUEUE_BASE)] = ["libs/b/src/lib.rs"]
        for downloaded, record in cases:
            with self.subTest(downloaded=downloaded, record=record):
                self.downloaded, self.record = downloaded, record
                self.assert_runs_the_queue_plan(self.decide())

    def test_full_pull_request_plan_runs_the_queue_plan(self):
        for plan in ({**pr_plan(set()), "full": True}, {"full": False}):
            with self.subTest(plan=plan):
                self.record = {**self.record, "plan": plan}
                self.assert_runs_the_queue_plan(self.decide())

    def test_landed_change_reaching_the_full_workspace_runs_the_queue_plan(self):
        for landed in (
            [".github/workflows/ci.yml"],
            ["libs/b/src/lib.rs", "Cargo.toml"],
        ):
            with self.subTest(landed=landed):
                self.diffs[(TESTED_BASE, QUEUE_BASE)] = landed
                self.assert_runs_the_queue_plan(self.decide())

    def test_unreadable_github_api_runs_the_queue_plan(self):
        with patch.object(qt, "gh_json", side_effect=RuntimeError("HTTP 502")):
            self.assert_runs_the_queue_plan(self.decide())

    def test_queue_branch_without_a_pull_request_runs_the_queue_plan(self):
        plan = qt.decide(REPO, "refs/heads/main", QUEUE_BASE, QUEUE_HEAD)
        self.assert_runs_the_queue_plan(plan)

    def test_disjoint_closures_trust_the_pull_request_run(self):
        for landed in (["libs/b/src/lib.rs"], [], ["README.md"]):
            with self.subTest(landed=landed):
                self.diffs[(TESTED_BASE, QUEUE_BASE)] = landed
                plan = self.decide()
                self.assertIs(plan["trusted"], True)
                self.assertEqual(plan["crates"], "")
                for system in ("ubuntu", "macos", "windows"):
                    self.assertIs(plan[f"{system}_skip"], True)
                self.assertIs(plan["windows_process"], False)
                self.assertIs(plan["windows_dev_build"], False)

    def test_overlapping_closures_check_only_the_intersection(self):
        self.record = {**self.record, "plan": pr_plan({"lib-a", "app-a", "app-ab"})}
        plan = self.decide()
        self.assertIs(plan["trusted"], False)
        self.assertEqual(plan["crates"], "app-ab")
        for system in ("ubuntu", "macos", "windows"):
            self.assertEqual(plan[f"{system}_clippy"], "-p app-ab --all-targets")
            self.assertEqual(plan[f"{system}_test"], "-p app-ab")
            self.assertEqual(plan[f"{system}_build"], "-p app-ab")
            self.assertIs(plan[f"{system}_skip"], False)
            self.assertIs(plan[f"{system}_doctest"], False)

    def test_closures_follow_the_queue_tree_dependency_graph(self):
        self.record = {**self.record, "plan": pr_plan({"lib-b", "app-b", "app-ab"})}
        self.diffs[(TESTED_BASE, QUEUE_BASE)] = ["apps/a/src/main.rs"]
        self.diffs[(QUEUE_BASE, QUEUE_HEAD)] = ["libs/b/src/lib.rs"]
        cases = [
            (GRAPH, True, ""),
            ({**GRAPH, "app-a": {**GRAPH["app-a"], "deps": {"lib-a", "lib-b"}}}, False, "app-a"),
        ]
        for graph, trusted, crates in cases:
            with self.subTest(trusted=trusted):
                self.graph = graph
                plan = self.decide()
                self.assertIs(plan["trusted"], trusted)
                self.assertEqual(plan["crates"], crates)
                self.assertIs(plan["ubuntu_doctest"], not trusted)

    def test_main_emits_the_trusted_plan(self):
        with tempfile.TemporaryDirectory() as scratch:
            output = Path(scratch) / "output"
            env = {
                "GITHUB_REPOSITORY": REPO,
                "GITHUB_OUTPUT": str(output),
                "HEAD_REF": HEAD_REF,
                "BASE_SHA": QUEUE_BASE,
                "HEAD_SHA": QUEUE_HEAD,
            }
            with patch.dict(os.environ, env, clear=True), patch.object(sys, "argv", ["queue_trust.py"]):
                self.assertEqual(qt.main(), 0)
            lines = output.read_text().splitlines()
        self.assertIn("trusted=true", lines)
        self.assertIn("ubuntu_skip=true", lines)
        self.assertIn("windows_process=false", lines)


class RecordTests(unittest.TestCase):
    def test_record_holds_the_head_the_tested_base_and_the_plan(self):
        with tempfile.TemporaryDirectory() as scratch:
            plan_path = Path(scratch) / "affected.json"
            plan_path.write_text(json.dumps(pr_plan({"app-a"})))
            out = Path(scratch) / "tested-base" / "tested-base.json"
            argv = ["queue_trust.py", "record", PR_HEAD, TESTED_BASE, str(plan_path), str(out)]
            with patch.object(sys, "argv", argv):
                self.assertEqual(qt.main(), 0)
            recorded = json.loads(out.read_text())
        self.assertEqual(recorded, {"head": PR_HEAD, "base": TESTED_BASE, "plan": pr_plan({"app-a"})})
        self.assertEqual(qt.planned_crates(recorded["plan"]), {"app-a"})

    def test_planned_crates_refuses_a_full_or_unknown_plan(self):
        cases = [
            ({"full": True, "crates": ""}, None),
            ({"full": False}, None),
            ({"crates": "app-a"}, None),
            ({"full": False, "crates": ""}, set()),
            ({"full": False, "crates": "app-a lib-a"}, {"app-a", "lib-a"}),
        ]
        for plan, expected in cases:
            with self.subTest(plan=plan):
                self.assertEqual(qt.planned_crates(plan), expected)


if __name__ == "__main__":
    unittest.main()
