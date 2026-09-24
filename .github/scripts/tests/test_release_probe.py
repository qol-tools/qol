import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
_SPEC = importlib.util.spec_from_file_location("release_probe", SCRIPTS / "release_probe.py")
rp = importlib.util.module_from_spec(_SPEC)
sys.modules["release_probe"] = rp
_SPEC.loader.exec_module(rp)


def call_site_table(*sites: tuple[int, int, int, int]) -> bytes:
    table = bytes(value for site in sites for value in site)
    return bytes([0x01, len(table)]) + table


class LsdaEndTests(unittest.TestCase):
    def test_cleanup_only_lsda_ends_after_its_call_sites(self):
        lsda = bytes([0xFF, 0xFF]) + call_site_table((0, 4, 8, 0), (4, 2, 0, 0))
        self.assertEqual(rp.lsda_end(lsda + b"\xAA" * 8, 0), len(lsda))

    def test_action_chain_extends_the_lsda(self):
        sites = call_site_table((0, 4, 8, 1))
        actions = bytes([0x00, 0x00])
        lsda = bytes([0xFF, 0xFF]) + sites + actions
        self.assertEqual(rp.lsda_end(lsda + b"\xAA" * 8, 0), len(lsda))

    def test_type_table_end_bounds_the_lsda(self):
        body = call_site_table((0, 4, 8, 1)) + bytes([0x01, 0x00]) + b"\x00" * 4
        lsda = bytes([0xFF, 0x9B, len(body)]) + body
        self.assertEqual(rp.lsda_end(lsda + b"\xAA" * 8, 0), len(lsda))

    def test_unsupported_landing_pad_base_is_rejected(self):
        with self.assertRaises(ValueError):
            rp.lsda_end(bytes([0x00, 0xFF, 0x01, 0x00]), 0)


class BlankUnreferencedTests(unittest.TestCase):
    def test_only_bytes_outside_live_ranges_are_blanked(self):
        data = bytearray(b"HEADtablegarbage")
        rp.blank_unreferenced(data, 4, 12, [(4, 9)])
        self.assertEqual(bytes(data), b"HEADtable" + bytes(7))


class CompareTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def write(self, name: str, content: bytes) -> Path:
        path = self.root / name
        path.write_bytes(content)
        return path

    def report(self) -> dict:
        return {"identical": False, "reason": ""}

    def test_identical_raw_binaries_match(self):
        current = self.write("current", b"MZ same bytes")
        previous = self.write("previous", b"MZ same bytes")
        self.assertTrue(rp.compare(current, previous, self.report())["identical"])

    def test_different_raw_binaries_do_not_match(self):
        current = self.write("current", b"MZ new bytes")
        previous = self.write("previous", b"MZ old bytes")
        report = rp.compare(current, previous, self.report())
        self.assertFalse(report["identical"])
        self.assertEqual(report["reason"], "binary changed")

    def test_missing_previous_asset_counts_as_changed(self):
        current = self.write("current", b"MZ bytes")
        report = rp.compare(current, None, self.report())
        self.assertFalse(report["identical"])
        self.assertNotIn("current", report)


if __name__ == "__main__":
    unittest.main()
