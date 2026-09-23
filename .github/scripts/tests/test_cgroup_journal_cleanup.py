import importlib.util
from pathlib import Path
import tempfile
import unittest


SPEC = importlib.util.spec_from_file_location(
    "cleanup_cgroup_journal", Path(__file__).parents[1] / "cleanup_cgroup_journal.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class CgroupJournalCleanupTests(unittest.TestCase):
    def test_removes_abandoned_records_only_after_the_cgroup_tree_is_gone(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            journals = root / "journals"
            journals.mkdir()
            for name in ["registry.guard", "abandoned.lock", "old.lock.quarantine-1"]:
                (journals / name).write_text("record")
            MODULE.cleanup(journals, root / "removed-cgroup")
            self.assertFalse(journals.exists())

    def test_keeps_evidence_when_the_cgroup_tree_still_exists(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            journals = root / "journals"
            journals.mkdir()
            record = journals / "live.lock"
            record.write_text("record")
            cgroup = root / "cgroup"
            cgroup.mkdir()
            with self.assertRaisesRegex(RuntimeError, "cgroup tree"):
                MODULE.cleanup(journals, cgroup)
            self.assertEqual(record.read_text(), "record")

    def test_rejects_unexpected_entries_without_partial_deletion(self):
        for kind in ["directory", "symlink"]:
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                journals = root / "journals"
                journals.mkdir()
                record = journals / "record.lock"
                record.write_text("record")
                unexpected = journals / "unexpected"
                if kind == "directory":
                    unexpected.mkdir()
                else:
                    unexpected.symlink_to(record)
                with self.assertRaisesRegex(RuntimeError, "unexpected entry"):
                    MODULE.cleanup(journals, root / "removed-cgroup")
                self.assertEqual(record.read_text(), "record")
