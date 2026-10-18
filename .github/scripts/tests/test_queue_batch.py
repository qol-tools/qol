import importlib
import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
ROOT = SCRIPTS.parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location("queue_batch", SCRIPTS / "queue_batch.py")
qb = importlib.util.module_from_spec(_SPEC)
sys.modules["queue_batch"] = qb
_SPEC.loader.exec_module(qb)
plugin_version = importlib.import_module("plugin_version")

MAIN = "0" * 40
RUN_URL = "https://github.com/o/r/actions/runs/9"


def sha(n: int) -> str:
    return f"{n:x}" * 40


def entry(n: int, position: int, base: str, state: str = "AWAITING_CHECKS", title: str | None = None) -> dict:
    return {
        "position": position,
        "state": state,
        "oid": sha(n),
        "base": base,
        "number": 100 + n,
        "title": title or f"fix(x): change {n}",
        "branch": f"branch-{n}",
    }


def chain(*states: str) -> list[dict]:
    entries, base = [], MAIN
    for position, state in enumerate(states, start=1):
        entries.append(entry(position, position, base, state))
        base = sha(position)
    return entries


class FakeGitHub:
    def __init__(self, entries: list[dict]):
        self.entries = entries
        self.checks: dict[tuple[str, str], str] = {}
        self.posted: list[tuple[str, str, str]] = []
        self.calls: list[list[str]] = []
        self.refs: dict[str, str] = {}
        self.pulls: list[dict] = []
        self.runs: dict[str, dict] = {}
        self.jobs: list[dict] = []
        self.compare = "diverged"

    def __call__(self, args: list[str]) -> object:
        self.calls.append(args)
        fields = dict(arg.split("=", 1) for arg in args if "=" in arg and not arg.startswith("repos/"))
        path = next(arg for arg in args if arg.startswith("repos/") or arg == "graphql")
        method = args[args.index("--method") + 1] if "--method" in args else "GET"
        if path == "graphql":
            return self.queue()
        if "/compare/" in path:
            return {"status": self.compare}
        if path.endswith("/check-runs") and method == "POST":
            self.checks[(fields["head_sha"], fields["name"])] = fields["conclusion"]
            self.posted.append((fields["head_sha"], fields["name"], fields["conclusion"]))
            return {}
        if "/commits/" in path and path.endswith("/check-runs"):
            commit = path.split("/commits/")[1].split("/")[0]
            found = self.checks.get((commit, fields["check_name"]))
            return {"check_runs": [{"conclusion": found}] if found else []}
        if path.endswith("/git/refs"):
            branch = fields["ref"].removeprefix("refs/heads/")
            if branch in self.refs:
                raise RuntimeError("Reference already exists")
            self.refs[branch] = fields["sha"]
            return {}
        if "/git/ref/heads/" in path:
            return {"object": {"sha": self.refs[path.split("/git/ref/heads/")[1]]}}
        if path.endswith("/pulls") and method == "GET":
            return [pull for pull in self.pulls if pull["head"] == fields["head"].split(":")[1]]
        if path.endswith("/pulls") and method == "POST":
            pull = {"number": 900, "head": fields["head"], "title": fields["title"], "body": fields["body"]}
            self.pulls.append(pull)
            return pull
        if path.endswith("/actions/workflows/ci.yml/runs"):
            run = self.runs.get(fields["head_sha"])
            return {"workflow_runs": [run] if run else []}
        if "/actions/runs/" in path and "/jobs" in path:
            return {"jobs": self.jobs}
        if "/annotations" in path:
            return []
        return {}

    def queue(self) -> dict:
        nodes = [
            {
                "position": e["position"],
                "state": e["state"],
                "headCommit": {"oid": e["oid"]},
                "baseCommit": {"oid": e["base"]},
                "pullRequest": {"number": e["number"], "title": e["title"], "headRefName": e["branch"]},
            }
            for e in self.entries
        ]
        return {"data": {"repository": {"mergeQueue": {"entries": {"nodes": nodes}}}}}

    def writes(self, fragment: str) -> list[list[str]]:
        return [call for call in self.calls if "--method" in call and any(fragment in arg for arg in call)]


def patched(fake: FakeGitHub):
    return mock.patch.object(qb, "api", fake)


class RoleTests(unittest.TestCase):
    def test_tail_is_the_last_entry_github_is_building(self):
        cases = [
            (("AWAITING_CHECKS",), 1, "tail"),
            (("AWAITING_CHECKS", "AWAITING_CHECKS"), 1, "ahead"),
            (("AWAITING_CHECKS", "AWAITING_CHECKS"), 2, "tail"),
            (("AWAITING_CHECKS", "MERGEABLE"), 1, "ahead"),
            (("AWAITING_CHECKS", "QUEUED"), 1, "tail"),
            (("AWAITING_CHECKS", "UNMERGEABLE"), 1, "tail"),
            (("AWAITING_CHECKS",), 5, "gone"),
        ]
        for states, own, expected in cases:
            with self.subTest(states=states, own=own):
                self.assertEqual(qb.role(chain(*states), sha(own)), expected)

    def test_parses_the_queue_and_tolerates_an_unbuilt_entry(self):
        data = FakeGitHub(chain("AWAITING_CHECKS", "QUEUED")).queue()
        data["data"]["repository"]["mergeQueue"]["entries"]["nodes"][1]["headCommit"] = None
        self.assertEqual([e["oid"] for e in qb.queue_entries(data)], [sha(1), None])
        self.assertEqual(qb.queue_entries({"data": {"repository": {"mergeQueue": None}}}), [])

    def test_a_missing_entry_is_looked_up_again_before_it_counts_as_gone(self):
        naps = []
        with patched(FakeGitHub([])):
            self.assertEqual(qb.decide("o/r", sha(1), sleep=naps.append), "gone")
        self.assertEqual(len(naps), qb.MAX_ATTEMPTS - 1)

    def test_an_unreadable_queue_builds_the_release_here(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "out"
            env = {
                "GITHUB_REPOSITORY": "o/r",
                "GITHUB_SHA": sha(1),
                "GITHUB_RUN_ID": "9",
                "GITHUB_SERVER_URL": "https://github.com",
                "GITHUB_OUTPUT": str(out),
            }
            with (
                mock.patch.dict(os.environ, env),
                mock.patch.object(sys, "argv", ["queue_batch.py", "role"]),
                mock.patch.object(qb, "live_entries", side_effect=RuntimeError("HTTP 502")),
                mock.patch("builtins.print"),
            ):
                self.assertEqual(qb.main(), 0)
            self.assertEqual(out.read_text(), "role=tail\n")


class BatchTests(unittest.TestCase):
    def test_follows_queue_commit_parents_back_to_main(self):
        entries = chain("AWAITING_CHECKS", "AWAITING_CHECKS", "AWAITING_CHECKS")
        self.assertEqual([e["number"] for e in qb.batch(entries, entries[2])], [101, 102, 103])
        self.assertEqual([e["number"] for e in qb.batch(entries, entries[0])], [101])

    def test_an_entry_the_commit_does_not_hold_stays_out(self):
        entries = chain("AWAITING_CHECKS", "AWAITING_CHECKS")
        rebuilt = entry(3, 3, sha(1))
        self.assertEqual([e["number"] for e in qb.batch(entries + [rebuilt], rebuilt)], [101, 103])


class TitleTests(unittest.TestCase):
    def titled(self, *titles: str) -> list[dict]:
        return [entry(n, n, MAIN, title=title) for n, title in enumerate(titles, start=1)]

    def test_keeps_the_type_versioning_releases_highest(self):
        cases = [
            (("fix(a): x", "feat(b): y", "perf: z"), "feat: land #101, #102 and #103 together"),
            (("perf(a): x", "fix(b): y"), "fix: land #101 and #102 together"),
            (("perf(a): x", "docs: y"), "perf: land #101 and #102 together"),
            (("ci: x", "docs: y"), "ci: land #101 and #102 together"),
            (("chore(a)!: x", "fix: y"), "fix!: land #101 and #102 together"),
            (("not conventional", "neither"), "chore: land #101 and #102 together"),
        ]
        for titles, expected in cases:
            with self.subTest(titles=titles):
                self.assertEqual(qb.combined_title(self.titled(*titles)), expected)

    def test_the_commit_msg_gate_accepts_the_longest_title(self):
        entries = [entry(n, n, MAIN, title="feat(a)!: x") for n in range(1, 6)]
        for e, number in zip(entries, range(9995, 10000)):
            e["number"] = number
        title = qb.combined_title(entries)
        with tempfile.TemporaryDirectory() as tmp:
            message = Path(tmp) / "msg"
            message.write_text(title + "\n")
            result = subprocess.run(
                [str(ROOT / ".githooks/commit-msg"), str(message)], capture_output=True, text=True, cwd=ROOT
            )
        self.assertEqual(result.returncode, 0, title + result.stdout + result.stderr)

    def test_the_body_lists_each_original_subject_for_versioning(self):
        body = qb.combined_body(self.titled("feat(a): add x", "fix(b): mend y"), "failure details")
        commit = plugin_version.Commit(sha=MAIN, subject="feat: land #101 and #102 together", body=body)
        self.assertEqual(
            plugin_version.commit_subjects(commit),
            ["feat: land #101 and #102 together", "feat(a): add x (#101)", "fix(b): mend y (#102)"],
        )
        self.assertIn("failure details", body)


class GateTests(unittest.TestCase):
    def gate(self, fake: FakeGitHub, own: int, own_passed: bool, role: str) -> str:
        with patched(fake):
            return qb.gate("o/r", sha(own), own_passed, role, RUN_URL)

    def test_a_stale_or_merged_run_gates_nothing(self):
        for compare, expected in [("diverged", "stale"), ("behind", "merged"), ("identical", "merged")]:
            with self.subTest(compare=compare):
                fake = FakeGitHub(chain("AWAITING_CHECKS"))
                fake.compare = compare
                self.assertTrue(self.gate(fake, 7, False, "tail").startswith(expected))
                self.assertEqual(fake.posted, [])

    def test_failed_own_jobs_fail_only_this_entry(self):
        fake = FakeGitHub(chain("AWAITING_CHECKS", "AWAITING_CHECKS"))
        self.gate(fake, 2, False, "tail")
        self.assertEqual(fake.posted, [(sha(2), qb.GATE, "failure")])

    def test_the_tail_marks_its_batch_built_and_gates_each_checked_entry(self):
        fake = FakeGitHub(chain("AWAITING_CHECKS", "AWAITING_CHECKS", "AWAITING_CHECKS"))
        fake.checks[(sha(1), qb.CHECKED)] = "success"
        message = self.gate(fake, 3, True, "tail")
        self.assertEqual(
            fake.posted,
            [
                (sha(3), qb.CHECKED, "success"),
                (sha(3), qb.BUILT, "success"),
                (sha(2), qb.BUILT, "success"),
                (sha(1), qb.BUILT, "success"),
                (sha(3), qb.GATE, "success"),
                (sha(1), qb.GATE, "success"),
            ],
        )
        self.assertEqual(message, "built #101, #102 and #103 and passed the merge gate of #101 and #103")

    def test_an_entry_ahead_passes_when_the_batch_already_built_it(self):
        fake = FakeGitHub(chain("AWAITING_CHECKS", "AWAITING_CHECKS"))
        fake.checks[(sha(1), qb.BUILT)] = "success"
        self.gate(fake, 1, True, "ahead")
        self.assertEqual(fake.posted, [(sha(1), qb.CHECKED, "success"), (sha(1), qb.GATE, "success")])

    def test_an_entry_ahead_waits_without_a_runner(self):
        fake = FakeGitHub(chain("AWAITING_CHECKS", "AWAITING_CHECKS"))
        self.assertIn("waits", self.gate(fake, 1, True, "ahead"))
        self.assertEqual(fake.posted, [(sha(1), qb.CHECKED, "success")])


class CombineTests(unittest.TestCase):
    def combine(self, fake: FakeGitHub, own: int) -> int | None:
        with patched(fake), mock.patch("builtins.print"):
            return qb.combine("o/r", sha(own), "9", RUN_URL)

    def test_one_pull_request_holds_the_batch_and_the_originals_close_back_to_front(self):
        fake = FakeGitHub(chain("AWAITING_CHECKS", "AWAITING_CHECKS", "AWAITING_CHECKS"))
        self.assertEqual(self.combine(fake, 3), 900)
        self.assertEqual(fake.refs, {f"queue-batch/{sha(3)[:7]}": sha(3)})
        self.assertEqual(fake.pulls[0]["title"], "fix: land #101, #102 and #103 together")
        closes = fake.writes("state=closed")
        self.assertEqual(
            [call[2] for call in closes], ["repos/o/r/pulls/103", "repos/o/r/pulls/102", "repos/o/r/pulls/101"]
        )
        notes = fake.writes("/comments")
        self.assertEqual(len(notes), 3)
        self.assertIn("Batched into #900", notes[0][-1])
        self.assertLess(fake.calls.index(fake.writes("/git/refs")[0]), fake.calls.index(closes[0]))

    def test_a_rerun_reuses_the_branch_and_the_pull_request(self):
        fake = FakeGitHub(chain("AWAITING_CHECKS", "AWAITING_CHECKS"))
        branch = f"queue-batch/{sha(2)[:7]}"
        fake.refs[branch] = sha(2)
        fake.pulls.append({"number": 901, "head": branch})
        self.assertEqual(self.combine(fake, 2), 901)

    def test_a_branch_holding_other_work_is_never_overwritten(self):
        fake = FakeGitHub(chain("AWAITING_CHECKS", "AWAITING_CHECKS"))
        fake.refs[f"queue-batch/{sha(2)[:7]}"] = sha(9)
        with self.assertRaises(RuntimeError):
            self.combine(fake, 2)
        self.assertEqual(fake.writes("state=closed"), [])

    def test_a_lone_stale_or_combined_tail_goes_back_on_its_own(self):
        combined = chain("AWAITING_CHECKS", "AWAITING_CHECKS")
        combined[1]["branch"] = "queue-batch/abc1234"
        cases = [(chain("AWAITING_CHECKS"), 1), (chain("AWAITING_CHECKS", "AWAITING_CHECKS"), 7), (combined, 2)]
        for entries, own in cases:
            with self.subTest(own=own, branch=entries[-1]["branch"]):
                fake = FakeGitHub(entries)
                self.assertIsNone(self.combine(fake, own))
                self.assertEqual(fake.writes("POST"), [])


class ReconcileTests(unittest.TestCase):
    def reconcile(self, fake: FakeGitHub) -> str:
        with patched(fake):
            return qb.reconcile("o/r", RUN_URL)

    def unbuilt_tail(self) -> FakeGitHub:
        fake = FakeGitHub(chain("AWAITING_CHECKS", "AWAITING_CHECKS", "UNMERGEABLE"))
        fake.checks[(sha(2), qb.CHECKED)] = "success"
        fake.runs[sha(2)] = {"id": 55, "status": "completed", "run_attempt": 1}
        fake.jobs = [
            {"id": 7, "name": qb.BATCH_JOB, "conclusion": "success"},
            {"id": 8, "name": "release build (ubuntu-latest)", "conclusion": "skipped"},
        ]
        return fake

    def test_reruns_the_batch_role_of_a_tail_that_never_built(self):
        fake = self.unbuilt_tail()
        self.assertIn("#102 is the tail now: reran its batch role", self.reconcile(fake))
        self.assertEqual(fake.writes("/rerun")[0][2], "repos/o/r/actions/jobs/7/rerun")

    def test_leaves_a_tail_that_needs_nothing(self):
        cases = [
            ("is built", lambda f: f.checks.__setitem__((sha(2), qb.BUILT), "success")),
            ("is still running", lambda f: f.runs[sha(2)].__setitem__("status", "in_progress")),
            ("left to the check timeout", lambda f: f.runs[sha(2)].__setitem__("run_attempt", qb.MAX_ATTEMPTS)),
            ("already ran its release build", lambda f: f.jobs[1].__setitem__("conclusion", "failure")),
            ("did not pass its own jobs", lambda f: f.checks.pop((sha(2), qb.CHECKED))),
        ]
        for expected, change in cases:
            with self.subTest(expected=expected):
                fake = self.unbuilt_tail()
                change(fake)
                self.assertIn(expected, self.reconcile(fake))
                self.assertEqual(fake.writes("/rerun"), [])

    def test_a_tail_whose_gate_failed_hands_over_to_the_entry_ahead(self):
        fake = self.unbuilt_tail()
        fake.entries[2]["state"] = "AWAITING_CHECKS"
        fake.checks[(sha(3), qb.GATE)] = "failure"
        self.assertIn("#102 is the tail now", self.reconcile(fake))

    def test_gates_an_entry_whose_jobs_and_build_passed_unseen(self):
        fake = self.unbuilt_tail()
        fake.checks[(sha(1), qb.CHECKED)] = "success"
        fake.checks[(sha(1), qb.BUILT)] = "success"
        self.assertIn("passed the merge gate of #101", self.reconcile(fake))
        self.assertIn((sha(1), qb.GATE, "success"), fake.posted)


if __name__ == "__main__":
    unittest.main()
