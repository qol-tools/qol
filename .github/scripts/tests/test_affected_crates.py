import importlib.util
import sys
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "affected_crates", Path(__file__).resolve().parents[1] / "affected_crates.py"
)
ac = importlib.util.module_from_spec(_SPEC)
sys.modules["affected_crates"] = ac
_SPEC.loader.exec_module(ac)


class PlatformExcludeDerivation(unittest.TestCase):
    def test_excludes_derived_from_plugin_platforms(self):
        ubuntu, macos = ac.platform_excludes()
        cases = [
            ("keyremap", ubuntu, True),
            ("plugin-removeapp", ubuntu, True),
            ("plugin-os-themes", ubuntu, False),
            ("plugin-os-themes", macos, True),
            ("keyremap", macos, False),
            ("alt-tab", ubuntu, False),
            ("alt-tab", macos, False),
        ]
        for package, excluded, expected in cases:
            self.assertEqual(
                package in excluded, expected, f"{package} in {sorted(excluded)}"
            )

    def test_exclude_flags_sorted_and_spaced(self):
        self.assertEqual(ac.exclude_flags(set()), "")
        self.assertEqual(
            ac.exclude_flags({"b", "a"}), " --exclude a --exclude b"
        )


if __name__ == "__main__":
    unittest.main()
