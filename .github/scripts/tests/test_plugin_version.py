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
        ids = sorted(p.plugin_id for p in plugins)
        self.assertEqual(ids, ["plugin-alt-tab", "qol-shot"], "prefix-less dir must be discovered")

    def test_ignores_dirs_without_plugin_toml(self):
        write_plugin(self.root / "plugins", "qol-shot", "qol-shot", "1.7.1", "qol-shot")
        (self.root / "plugins" / "not-a-plugin").mkdir()
        (self.root / "plugins" / "not-a-plugin" / "README.md").write_text("hi")
        plugins = pv.discover_plugins(self.root, package_map("qol-shot"), None)
        self.assertEqual([p.plugin_id for p in plugins], ["qol-shot"])

    def test_excludes_template(self):
        write_plugin(self.root / "plugins", "plugin-template", "plugin-template", "0.1.0", "tmpl")
        write_plugin(self.root / "plugins", "qol-shot", "qol-shot", "1.7.1", "qol-shot")
        plugins = pv.discover_plugins(self.root, package_map("tmpl", "qol-shot"), None)
        self.assertEqual([p.plugin_id for p in plugins], ["qol-shot"])

    def test_rejects_dir_id_mismatch(self):
        write_plugin(self.root / "plugins", "shot", "qol-shot", "1.7.1", "qol-shot")
        with self.assertRaises(RuntimeError):
            pv.discover_plugins(self.root, package_map("qol-shot"), None)


class InitialReleasePlanTests(unittest.TestCase):
    def _plugin(self, plugin_id: str, version: str) -> "pv.Plugin":
        return pv.Plugin(
            plugin_id=plugin_id,
            directory=Path("/x") / plugin_id,
            package_name=plugin_id,
            cargo_manifest=Path("/x/Cargo.toml"),
            plugin_manifest=Path("/x/plugin.toml"),
            cargo_version=version,
            plugin_version=version,
        )

    def test_bootstraps_manifest_version_with_id_prefixed_tag(self):
        plan = pv.initial_release_plan(self._plugin("qol-shot", "1.7.1"))
        self.assertEqual(plan.new_version, "1.7.1")
        self.assertEqual(plan.old_version, "1.7.1")
        self.assertEqual(plan.tag, "qol-shot-v1.7.1", "tag prefix is the manifest id, no plugin- assumption")
        self.assertEqual(plan.bump, "initial")


if __name__ == "__main__":
    unittest.main()
