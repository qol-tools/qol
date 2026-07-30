import importlib.util
import os
import subprocess
import sys
import tempfile
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


def git(root: Path, *args: str, env: dict[str, str] | None = None) -> str:
    result = subprocess.run(
        ["git", "-C", str(root), *args],
        capture_output=True,
        text=True,
        check=True,
        env=env,
    )
    return result.stdout.strip()


class DiscoverPluginsTests(unittest.TestCase):
    def setUp(self):
        import tempfile

        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "plugins").mkdir()

    def tearDown(self):
        self.tmp.cleanup()

    def test_discovers_plugins_by_manifest_identity(self):
        write_plugin(self.root / "plugins", "qol-shot", "qol-shot", "1.7.1", "qol-shot")
        write_plugin(self.root / "plugins", "alt-tab", "plugin-alt-tab", "0.1.0", "alt-tab")
        plugins = pv.discover_plugins(self.root, package_map("qol-shot", "alt-tab"), None)
        ids = sorted(p.id for p in plugins)
        self.assertEqual(ids, ["plugin-alt-tab", "qol-shot"])

    def test_ignores_dirs_without_plugin_toml(self):
        write_plugin(self.root / "plugins", "qol-shot", "qol-shot", "1.7.1", "qol-shot")
        (self.root / "plugins" / "not-a-plugin").mkdir()
        (self.root / "plugins" / "not-a-plugin" / "README.md").write_text("hi")
        plugins = pv.discover_plugins(self.root, package_map("qol-shot"), None)
        self.assertEqual([p.id for p in plugins], ["qol-shot"])

    def test_excludes_template(self):
        write_plugin(self.root / "plugins", "template", "plugin-template", "0.1.0", "tmpl")
        write_plugin(self.root / "plugins", "qol-shot", "qol-shot", "1.7.1", "qol-shot")
        plugins = pv.discover_plugins(self.root, package_map("tmpl", "qol-shot"), None)
        self.assertEqual([p.id for p in plugins], ["qol-shot"])

    def test_directory_name_is_independent_from_plugin_id(self):
        write_plugin(self.root / "plugins", "shot", "qol-shot", "1.7.1", "qol-shot")
        plugins = pv.discover_plugins(self.root, package_map("qol-shot"), None)
        self.assertEqual(plugins[0].id, "qol-shot")
        self.assertEqual(plugins[0].directory.name, "shot")

    def test_rejects_duplicate_declared_ids(self):
        write_plugin(self.root / "plugins", "first", "qol-shot", "1.7.1", "first")
        write_plugin(self.root / "plugins", "second", "qol-shot", "1.7.1", "second")
        with self.assertRaises(RuntimeError):
            pv.discover_plugins(self.root, package_map("first", "second"), None)


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


class VersionBumpCommitTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        git(self.root, "init", "--quiet")
        git(self.root, "config", "user.name", "Fixture")
        git(self.root, "config", "user.email", "fixture@example.invalid")
        write_host(self.root, "apps/qol-tray", "qol-tray", "3.40.5")
        write_plugin(
            self.root / "plugins",
            "fixture",
            "plugin-fixture",
            "1.2.3",
            "fixture-package",
        )
        (self.root / "Cargo.lock").write_text(
            'version = 4\n\n'
            '[[package]]\nname = "fixture-package"\nversion = "1.2.3"\n\n'
            '[[package]]\nname = "qol-tray"\nversion = "3.40.5"\n'
        )
        git(self.root, "add", ".")
        git(self.root, "commit", "--quiet", "-m", "feat: baseline")
        self.parent = git(self.root, "rev-parse", "HEAD")

    def tearDown(self):
        self.tmp.cleanup()

    def replace(self, path: str, old: str, new: str) -> None:
        target = self.root / path
        target.write_text(target.read_text().replace(old, new))

    def commit(self, bot: bool = True) -> str:
        git(self.root, "add", ".")
        if not bot:
            git(self.root, "commit", "--quiet", "-m", pv.VERSION_BUMP_SUBJECT)
            return git(self.root, "rev-parse", "HEAD")
        env = dict(os.environ)
        env.update(
            {
                "GIT_AUTHOR_NAME": pv.VERSION_BUMP_BOT_NAME,
                "GIT_AUTHOR_EMAIL": pv.VERSION_BUMP_BOT_EMAIL,
                "GIT_COMMITTER_NAME": pv.VERSION_BUMP_BOT_NAME,
                "GIT_COMMITTER_EMAIL": pv.VERSION_BUMP_BOT_EMAIL,
            }
        )
        git(
            self.root,
            "commit",
            "--quiet",
            "-m",
            pv.VERSION_BUMP_SUBJECT,
            env=env,
        )
        return git(self.root, "rev-parse", "HEAD")

    def test_accepts_matching_release_unit_and_lock_versions(self):
        changes = [
            ("apps/qol-tray/Cargo.toml", "3.40.5", "3.40.6"),
            ("plugins/fixture/Cargo.toml", "1.2.3", "1.3.0"),
            ("plugins/fixture/plugin.toml", "1.2.3", "1.3.0"),
            ("Cargo.lock", "3.40.5", "3.40.6"),
            ("Cargo.lock", "1.2.3", "1.3.0"),
        ]
        for path, old, new in changes:
            self.replace(path, old, new)
        sha = self.commit()

        self.assertEqual(
            pv.verified_version_bump_parent(self.root, sha),
            self.parent,
        )

    def test_rejects_manifest_changes_beyond_versions(self):
        self.replace("apps/qol-tray/Cargo.toml", "3.40.5", "3.40.6")
        self.replace("Cargo.lock", "3.40.5", "3.40.6")
        with (self.root / "apps/qol-tray/Cargo.toml").open("a") as handle:
            handle.write('description = "changed"\n')
        sha = self.commit()

        with self.assertRaisesRegex(RuntimeError, "changed more than"):
            pv.verified_version_bump_parent(self.root, sha)

    def test_rejects_mismatched_plugin_versions(self):
        self.replace("plugins/fixture/Cargo.toml", "1.2.3", "1.2.4")
        self.replace("plugins/fixture/plugin.toml", "1.2.3", "1.3.0")
        self.replace("Cargo.lock", "1.2.3", "1.2.4")
        sha = self.commit()

        with self.assertRaisesRegex(RuntimeError, "versions differ"):
            pv.verified_version_bump_parent(self.root, sha)

    def test_rejects_non_bot_commits(self):
        self.replace("apps/qol-tray/Cargo.toml", "3.40.5", "3.40.6")
        self.replace("Cargo.lock", "3.40.5", "3.40.6")
        sha = self.commit(bot=False)

        with self.assertRaisesRegex(RuntimeError, "not an exact"):
            pv.verified_version_bump_parent(self.root, sha)


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


class ReservedPluginIdsTests(unittest.TestCase):
    def test_auto_excluded_derives_from_qol_conventions(self):
        self.assertEqual(pv.AUTO_EXCLUDED_PLUGIN_IDS, {"plugin-template"})


if __name__ == "__main__":
    unittest.main()
