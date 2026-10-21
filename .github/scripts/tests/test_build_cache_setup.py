import itertools
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


def field(block, name):
    return re.search(rf"^\s*{re.escape(name)}: (.+)$", block, re.M)[1]


def condition(expression, values):
    expression = expression.removeprefix("${{ ").removesuffix(" }}")
    for key, value in values.items():
        expression = expression.replace(key, repr(value))
    return eval(expression.replace("&&", "and").replace("||", "or"), {"__builtins__": {}})


class BuildCacheSetupTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.action = (ROOT / ".github/actions/rust-setup/action.yml").read_text()
        cls.steps = cls.action.split("runs:\n", 1)[1].split("\n    - ")[1:]
        cls.cache = next(step for step in cls.steps if "id: build-cache" in step)
        cls.warm = (ROOT / ".github/workflows/build-cache.yml").read_text()
        cls.warm_steps = cls.warm.split("    steps:\n", 1)[1].split("\n      - ")[1:]

    def test_probing_and_disabling_saves_are_opt_in(self):
        inputs = self.action.split("inputs:\n", 1)[1].split("outputs:\n", 1)[0]
        for name, default in [("lookup-only", '"false"'), ("save-cache", '"true"')]:
            with self.subTest(input=name):
                block = re.split(r"\n  (?=\S)", inputs.split(f"  {name}:\n", 1)[1], 1)[0]
                self.assertEqual(field(block, "default"), default)
        self.assertEqual(field(self.cache, "lookup-only"), "${{ inputs.lookup-only }}")

    def test_only_main_builds_with_save_permission_can_write(self):
        expression = field(self.cache, "save-if")
        for ref, lookup, save in itertools.product(
            ["refs/heads/main", "refs/pull/1/merge", "refs/heads/gh-readonly-queue/main/pr-1"],
            ["true", "false"],
            ["true", "false"],
        ):
            with self.subTest(ref=ref, lookup=lookup, save=save):
                actual = condition(expression, {
                    "github.ref": ref,
                    "inputs.lookup-only": lookup,
                    "inputs.save-cache": save,
                })
                self.assertEqual(actual, ref == "refs/heads/main" and lookup == "false" and save == "true")

    def test_probes_never_clean_disk_or_install_system_dependencies(self):
        room = next(step for step in self.steps if step.startswith("name: Make room"))
        dependencies = next(step for step in self.steps if step.startswith("name: Install Linux"))
        for runner, lookup, install in itertools.product(
            ["Linux", "macOS", "Windows"], ["true", "false"], ["true", "false"]
        ):
            values = {
                "runner.os": runner,
                "inputs.lookup-only": lookup,
                "inputs.system-dependencies": install,
            }
            with self.subTest(**values):
                self.assertEqual(condition(field(room, "if"), values), runner == "Linux" and lookup == "false")
                self.assertEqual(condition(field(dependencies, "if"), values), runner == "Linux" and lookup == "false" and install == "true")

    def test_probe_and_restore_share_the_exact_setup_and_namespace(self):
        setup = [step for step in self.warm_steps if "uses: ./.github/actions/rust-setup\n" in step]
        self.assertEqual(len(setup), 2)
        probe, restore = setup
        self.assertEqual(field(probe, "id"), "rust_setup")
        self.assertEqual(field(probe, "lookup-only"), '"true"')
        self.assertNotIn("lookup-only:", restore)
        for name in ["cache-key", "components"]:
            self.assertEqual(field(probe, name), field(restore, name))
        self.assertEqual(field(probe, "cache-key"), "ci-${{ matrix.os }}")

    def test_python_setup_leaves_the_compile_environment_alone(self):
        python = next(step for step in self.warm_steps if "actions/setup-python@" in step)
        self.assertEqual(field(python, "update-environment"), "false")
        self.assertEqual(field(python, "id"), "python")
        workspace = next(step for step in self.warm_steps if step.startswith("name: Workspace arguments"))
        self.assertIn("${{ steps.python.outputs.python-path }}", field(workspace, "run"))

    def test_only_exact_hits_skip_restore_and_every_warming_step(self):
        probe_index = next(i for i, step in enumerate(self.warm_steps) if "id: rust_setup\n" in step)
        following = self.warm_steps[probe_index + 1:]
        self.assertGreaterEqual(len(following), 5)
        for step, hit in itertools.product(following, ["true", "false", ""]):
            with self.subTest(step=step.splitlines()[0], hit=hit):
                self.assertEqual(
                    condition(field(step, "if"), {"steps.rust_setup.outputs.build-cache-hit": hit}),
                    hit != "true",
                )


if __name__ == "__main__":
    unittest.main()
