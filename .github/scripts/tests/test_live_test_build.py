import json
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))

import live_test  # noqa: E402
import live_test_build as build  # noqa: E402

LISTED = live_test.patterns(live_test.PATHS_FILE.read_text(encoding="utf-8"))


def graph(**deps: list[str]) -> dict:
    dirs = {
        "qol-gpui": "libs/gpui",
        "qol-sessions-engine": "libs/sessions-engine",
        "qol-tray": "apps/tray",
        "qol-cli-sessions": "plugins/cli-sessions",
        "qol-shot": "plugins/shot",
        "qol-bluetooth": "plugins/bluetooth",
        "qol": "tools/cli",
    }
    return {
        name: {"dir": directory, "deps": set(deps.get(name, [])), "doctest": False}
        for name, directory in dirs.items()
    }


GRAPH = graph(
    **{
        "qol-tray": ["qol-gpui"],
        "qol-cli-sessions": ["qol-sessions-engine", "qol-gpui"],
        "qol-shot": ["qol-gpui"],
        "qol": ["qol-sessions-engine"],
    }
)
UNITS = {
    "qol-cli-sessions": {"id": "qol-cli-sessions", "version": "0.127.1", "linux": True},
    "qol-shot": {"id": "qol-shot", "version": "1.4.0", "linux": True},
    "qol-bluetooth": {"id": "qol-bluetooth", "version": "2.0.0", "linux": False},
}


class PlanTests(unittest.TestCase):
    def test_a_change_off_the_list_builds_nothing(self):
        result = build.plan(["libs/sessions-engine/src/switchboard/mod.rs"], LISTED, GRAPH, UNITS, "3.0.0")
        self.assertEqual(result, {"found": [], "orphans": [], "plugins": [], "units": [], "tray": ""})

    def test_the_agent_rows_build_the_plugin_that_draws_them(self):
        result = build.plan(
            ["libs/sessions-engine/src/switchboard/rows.rs", "libs/sessions-engine/src/switchboard/mod.rs"],
            LISTED,
            GRAPH,
            UNITS,
            "3.0.0",
        )
        self.assertEqual(result["plugins"], ["qol-cli-sessions-v0.127.1"])
        self.assertEqual(result["tray"], "")

    def test_a_shared_ui_lib_builds_its_linux_plugins_and_the_tray(self):
        result = build.plan(["libs/gpui/src/kit.rs"], LISTED, GRAPH, UNITS, "3.0.0")
        self.assertEqual(result["plugins"], ["qol-cli-sessions-v0.127.1", "qol-shot-v1.4.0"])
        self.assertEqual(result["tray"], "qol-tray-v3.0.0")

    def test_a_plugin_without_linux_is_left_out_and_the_job_says_so(self):
        result = build.plan(["plugins/bluetooth/src/ui/panel.rs"], LISTED, GRAPH, UNITS, "3.0.0")
        self.assertEqual(result["plugins"], [])
        self.assertEqual(len(result["found"]), 1)
        code, message = build.verdict(result)
        self.assertEqual(code, 0)
        self.assertTrue(message.startswith("::warning::"), message)

    def test_a_patched_vendor_crate_builds_every_crate_that_uses_it(self):
        patches = {"vendor/gpui": "gpui"}
        users = {"gpui": {"qol-gpui", "qol-shot"}}
        result = build.plan(["vendor/gpui/src/text.rs"], LISTED, GRAPH, UNITS, "3.0.0", patches, users)
        self.assertEqual(result["orphans"], [])
        self.assertEqual(result["plugins"], ["qol-cli-sessions-v0.127.1", "qol-shot-v1.4.0"])
        self.assertEqual(result["tray"], "qol-tray-v3.0.0")

    def test_a_listed_file_no_crate_builds_fails_the_job(self):
        result = build.plan(["vendor/gpui/src/text.rs"], LISTED, GRAPH, UNITS, "3.0.0")
        self.assertEqual(result["orphans"], ["vendor/gpui/src/text.rs"])
        code, message = build.verdict(result)
        self.assertEqual(code, 1)
        self.assertIn("vendor/gpui/src/text.rs", message)

    def test_the_repository_patches_name_the_vendored_gpui(self):
        self.assertEqual(build.patched_paths(build.ROOT).get("vendor/gpui"), "gpui")
        declared = build.declared_dependencies(
            {"dependencies": {"a": "1"}, "target": {"cfg(unix)": {"dependencies": {"b": "1"}}}}
        )
        self.assertEqual(declared, {"a", "b"})

    def test_outputs_say_whether_anything_builds(self):
        with tempfile.TemporaryDirectory() as folder:
            out = Path(folder) / "out"
            build.emit(str(out), {"plugins": ["a-v1.0.0", "b-v1.0.0"], "tray": ""})
            build.emit(str(out), {"plugins": [], "tray": ""})
            self.assertEqual(
                out.read_text().splitlines(),
                [
                    "plugin_tags=a-v1.0.0 b-v1.0.0",
                    "tray_tag=",
                    "build=true",
                    "plugin_tags=",
                    "tray_tag=",
                    "build=false",
                ],
            )

    def test_the_manifest_names_the_pull_request_and_its_commit(self):
        files = [{"id": "a", "version": "1.0.0", "tag": "a-v1.0.0", "file": "a-linux-x86_64", "bin_name": "a"}]
        body = build.manifest({"plugins": ["a-v1.0.0"], "tray": "qol-tray-v3.0.0"}, "412", "abc", "77", files)
        self.assertEqual(body["pull_request"], 412)
        self.assertEqual(body["plugins"], files)
        self.assertEqual(body["run_id"], 77)
        self.assertEqual(body["head_sha"], "abc")
        self.assertEqual(json.loads(json.dumps(body))["tray"], "qol-tray-v3.0.0")

    def test_the_manifest_names_the_file_and_the_installed_binary(self):
        units = build.plugin_units(build.ROOT)
        entry = build.installed_files(build.ROOT, units["qol-cli-sessions"])
        self.assertEqual(entry["bin_name"], "qol-cli-sessions")
        self.assertEqual(entry["tag"], f"qol-cli-sessions-v{units['qol-cli-sessions']['version']}")
        self.assertTrue(entry["file"].startswith("qol-cli-sessions"), entry)

    def test_every_repository_plugin_maps_to_an_id_and_its_version(self):
        units = build.plugin_units(build.ROOT)
        self.assertEqual(units["qol-cli-sessions"]["id"], "qol-cli-sessions")
        for name, unit in units.items():
            with self.subTest(package=name):
                self.assertRegex(f"{unit['id']}-v{unit['version']}", r"^[a-z0-9][a-z0-9-]*-v[0-9]+\.[0-9]+\.[0-9]+$")


if __name__ == "__main__":
    unittest.main()
