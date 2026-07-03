import importlib.util
import sys
import unittest
from pathlib import Path

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
