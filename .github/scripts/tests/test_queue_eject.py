import importlib.util
import sys
import unittest
from pathlib import Path
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location("queue_eject", SCRIPTS / "queue_eject.py")
qe = importlib.util.module_from_spec(_SPEC)
sys.modules["queue_eject"] = qe
_SPEC.loader.exec_module(qe)
qb = qe.qb

MAIN = "0" * 40


def sha(n: int) -> str:
    return f"{n:x}" * 40


def chain(count: int) -> list[dict]:
    entries, base = [], MAIN
    for n in range(1, count + 1):
        entries.append({"position": n, "state": "AWAITING_CHECKS", "oid": sha(n), "base": base, "number": 100 + n})
        base = sha(n)
    return entries


class FakeGitHub:
    def __init__(self, entries: list[dict], checks: dict[int, str]):
        self.entries = entries
        self.checks = {}
        for n, state in checks.items():
            if state == "failed":
                self.checks[(sha(n), qb.GATE)] = "failure"
            elif state == "passed":
                self.checks[(sha(n), qb.CHECKED)] = "success"
        self.dequeued: list[str] = []

    def __call__(self, args: list[str]) -> dict:
        path = next(arg for arg in args if arg.startswith("repos/") or arg == "graphql")
        if path == "graphql":
            self.dequeued.append(next(arg for arg in args if arg.startswith("id=")))
            return {}
        if "/pulls/" in path:
            return {"node_id": f"PR_{path.rsplit('/', 1)[1]}"}
        commit = path.split("/commits/")[1].split("/")[0]
        name = next(arg for arg in args if arg.startswith("check_name=")).split("=", 1)[1]
        found = self.checks.get((commit, name))
        return {"check_runs": [{"conclusion": found}] if found else []}


def patched(fake: FakeGitHub):
    return mock.patch.multiple(qb, api=fake, live_entries=lambda repo: fake.entries)


class JudgeTests(unittest.TestCase):
    def test_blames_the_entry_only_once_the_entry_ahead_passed(self):
        cases = [
            (1, {}, ("own", None)),
            (2, {1: "passed"}, ("own", 101)),
            (2, {1: "failed"}, ("inherited", 101)),
            (2, {}, ("pending", 101)),
            (9, {}, ("gone", None)),
        ]
        for own, checks, (cause, ahead) in cases:
            with self.subTest(own=own, checks=checks), patched(FakeGitHub(chain(3), checks)):
                found, entry = qe.judge("o/r", chain(3), sha(own))
                self.assertEqual((found, entry and entry["number"]), (cause, ahead))

    def test_an_entry_ahead_with_no_queue_commit_leaves_main_as_the_base(self):
        entries = chain(2)
        entries[0]["oid"] = None
        with patched(FakeGitHub(entries, {})):
            self.assertEqual(qe.judge("o/r", entries, sha(2)), ("own", None))


class SweepTests(unittest.TestCase):
    def test_dequeues_the_frontmost_failure_whose_entry_ahead_passed(self):
        fake = FakeGitHub(chain(4), {1: "pending", 2: "passed", 3: "failed", 4: "failed"})
        with patched(fake):
            self.assertEqual(qe.sweep("o/r"), "dequeued #103: its own change failed, so the entries behind it rebuild now")
        self.assertEqual(fake.dequeued, ["id=PR_103"])

    def test_leaves_a_failure_queued_while_the_entry_ahead_runs(self):
        fake = FakeGitHub(chain(3), {2: "failed", 3: "failed"})
        with patched(fake):
            self.assertEqual(qe.sweep("o/r"), "#102 failed, but #101 ahead of it is still running: left queued")
        self.assertEqual(fake.dequeued, [])

    def test_a_queue_without_failures_dequeues_nothing(self):
        fake = FakeGitHub(chain(2), {1: "passed"})
        with patched(fake):
            self.assertEqual(qe.sweep("o/r"), "no failed entry")
        self.assertEqual(fake.dequeued, [])


if __name__ == "__main__":
    unittest.main()
