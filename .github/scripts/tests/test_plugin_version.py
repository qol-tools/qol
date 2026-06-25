import importlib.util
import sys
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "plugin_version", Path(__file__).resolve().parents[1] / "plugin_version.py"
)
pv = importlib.util.module_from_spec(_SPEC)
sys.modules["plugin_version"] = pv
_SPEC.loader.exec_module(pv)


def write_plugin(plugins_dir: Path, dir_name: str, plugin_id: str, version: str, package: str):
    crate = plugins_dir / dir_name
    crate.mkdir(parents=True)
    (crate / "plugin.toml").write_text(
        f'[plugin]\nid = "{plugin_id}"\nname = "X"\ndescription = "x"\n'
        f'version = "{version}"\nplatforms = ["linux"]\n'
    )
    (crate / "Cargo.toml").write_text(
        f'[package]\nname = "{package}"\nversion = "{version}"\n'
    )
    return crate


def package_map(*names: str) -> dict:
    return {
        name: pv.Package(
            name=name,
            directory=Path("/x"),
            rel_dir="x",
            manifest=Path("/x/Cargo.toml"),
            deps=set(),
            external_deps=set(),
        )
        for name in names
    }


class DiscoverPluginsTests(unittest.TestCase):
    def setUp(self):
        import tempfile

        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "plugins").mkdir()

    def tearDown(self):
        self.tmp.cleanup()

    def test_discovers_prefixless_plugin_by_manifest_presence(self):
        write_plugin(self.root / "plugins", "qol-shot", "qol-shot", "1.7.1", "qol-shot")
        write_plugin(self.root / "plugins", "plugin-alt-tab", "plugin-alt-tab", "0.1.0", "alt-tab")
        plugins = pv.discover_plugins(self.root, package_map("qol-shot", "alt-tab"), None)
        ids = sorted(p.id for p in plugins)
        self.assertEqual(ids, ["plugin-alt-tab", "qol-shot"], "prefix-less dir must be discovered")

    def test_ignores_dirs_without_plugin_toml(self):
        write_plugin(self.root / "plugins", "qol-shot", "qol-shot", "1.7.1", "qol-shot")
        (self.root / "plugins" / "not-a-plugin").mkdir()
        (self.root / "plugins" / "not-a-plugin" / "README.md").write_text("hi")
        plugins = pv.discover_plugins(self.root, package_map("qol-shot"), None)
        self.assertEqual([p.id for p in plugins], ["qol-shot"])

    def test_excludes_template(self):
        write_plugin(self.root / "plugins", "plugin-template", "plugin-template", "0.1.0", "tmpl")
        write_plugin(self.root / "plugins", "qol-shot", "qol-shot", "1.7.1", "qol-shot")
        plugins = pv.discover_plugins(self.root, package_map("tmpl", "qol-shot"), None)
        self.assertEqual([p.id for p in plugins], ["qol-shot"])

    def test_rejects_dir_id_mismatch(self):
        write_plugin(self.root / "plugins", "shot", "qol-shot", "1.7.1", "qol-shot")
        with self.assertRaises(RuntimeError):
            pv.discover_plugins(self.root, package_map("qol-shot"), None)


class InitialReleasePlanTests(unittest.TestCase):
    def _plugin(self, plugin_id: str, version: str) -> "pv.ReleaseUnit":
        return pv.ReleaseUnit(
            id=plugin_id,
            directory=Path("/x") / plugin_id,
            package_name=plugin_id,
            cargo_manifest=Path("/x/Cargo.toml"),
            cargo_version=version,
            plugin_manifest=Path("/x/plugin.toml"),
            plugin_version=version,
        )

    def test_bootstraps_manifest_version_with_id_prefixed_tag(self):
        plan = pv.initial_release_plan(self._plugin("qol-shot", "1.7.1"))
        self.assertEqual(plan.new_version, "1.7.1")
        self.assertEqual(plan.old_version, "1.7.1")
        self.assertEqual(plan.tag, "qol-shot-v1.7.1", "tag prefix is the manifest id, no plugin- assumption")
        self.assertEqual(plan.bump, "initial")

    def test_host_unit_tag_uses_id_prefix_without_plugin_manifest(self):
        host = pv.ReleaseUnit(
            id="qol-tray",
            directory=Path("/x/apps/qol-tray"),
            package_name="qol-tray",
            cargo_manifest=Path("/x/apps/qol-tray/Cargo.toml"),
            cargo_version="3.16.0",
        )
        self.assertIsNone(host.plugin_manifest, "host app has no plugin.toml")
        plan = pv.initial_release_plan(host)
        self.assertEqual(plan.tag, "qol-tray-v3.16.0")


def write_host(root: Path, rel_dir: str, package: str, version: str) -> Path:
    crate = root / rel_dir
    crate.mkdir(parents=True)
    (crate / "Cargo.toml").write_text(
        f'[package]\nname = "{package}"\nversion = "{version}"\n'
    )
    return crate


class DiscoverHostUnitsTests(unittest.TestCase):
    def setUp(self):
        import tempfile

        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def test_discovers_host_app_without_plugin_toml(self):
        write_host(self.root, "apps/qol-tray", "qol-tray", "3.16.0")
        units = pv.discover_host_units(self.root, package_map("qol-tray"), None)
        self.assertEqual([u.id for u in units], ["qol-tray"])
        self.assertIsNone(units[0].plugin_manifest)
        self.assertEqual(units[0].cargo_version, "3.16.0")

    def test_selection_picks_host_unit(self):
        write_host(self.root, "apps/qol-tray", "qol-tray", "3.16.0")
        units = pv.discover_host_units(self.root, package_map("qol-tray"), "qol-tray")
        self.assertEqual([u.id for u in units], ["qol-tray"])

    def test_selection_excludes_unmatched_host(self):
        write_host(self.root, "apps/qol-tray", "qol-tray", "3.16.0")
        self.assertEqual(pv.discover_host_units(self.root, package_map("qol-tray"), "qol-shot"), [])

    def test_unknown_selection_raises_across_all_units(self):
        (self.root / "plugins").mkdir()
        write_host(self.root, "apps/qol-tray", "qol-tray", "3.16.0")
        with self.assertRaises(RuntimeError):
            pv.discover_release_units(self.root, package_map("qol-tray"), "does-not-exist")


class ApplyHostPlanTests(unittest.TestCase):
    def setUp(self):
        import tempfile

        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def test_bumps_cargo_and_lock_without_touching_plugin_manifest(self):
        crate = write_host(self.root, "apps/qol-tray", "qol-tray", "3.16.0")
        (self.root / "Cargo.lock").write_text(
            '[[package]]\nname = "qol-tray"\nversion = "3.16.0"\n'
        )
        unit = pv.ReleaseUnit(
            id="qol-tray",
            directory=crate,
            package_name="qol-tray",
            cargo_manifest=crate / "Cargo.toml",
            cargo_version="3.16.0",
        )
        plan = pv.ReleasePlan(
            unit=unit,
            old_version="3.16.0",
            new_version="3.17.0",
            tag="qol-tray-v3.17.0",
            bump="minor",
            commit_count=4,
        )
        changed = pv.apply_plans(self.root, [plan])
        self.assertTrue(changed)
        self.assertEqual(pv.package_version(crate / "Cargo.toml"), "3.17.0")
        self.assertIn('version = "3.17.0"', (self.root / "Cargo.lock").read_text())
        self.assertFalse((crate / "plugin.toml").exists(), "host release must not create a plugin.toml")


class HighestVersionTagTests(unittest.TestCase):
    def test_picks_highest_semver_for_prefix(self):
        cases = [
            (["qol-tray-v3.16.0"], "qol-tray-v", "qol-tray-v3.16.0"),
            (["qol-tray-v3.9.0", "qol-tray-v3.16.0", "qol-tray-v3.10.1"], "qol-tray-v", "qol-tray-v3.16.0"),
            ([], "qol-tray-v", None),
            (["plugin-alt-tab-v1.0.0"], "qol-tray-v", None),
            (["qol-tray-v3.16.0", "qol-tray-vbogus"], "qol-tray-v", "qol-tray-v3.16.0"),
        ]
        for tags, prefix, expected in cases:
            self.assertEqual(pv.highest_version_tag(tags, prefix), expected, f"tags={tags}")


if __name__ == "__main__":
    unittest.main()
