import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CARGO_GATE = re.compile(r"\bcargo (?:build|test|clippy|check|nextest run)\b")


def named_step(document, name, indent):
    start = document.index(f"{indent}- name: {name}\n")
    end = document.find(f"\n{indent}- ", start + 1)
    return document[start:end] if end != -1 else document[start:]


class ReleaseWorkflowContractTests(unittest.TestCase):
    def test_setup_reports_whether_the_build_cache_will_be_saved(self):
        action = (ROOT / ".github/actions/rust-setup/action.yml").read_text()
        cache = named_step(action, "Restore or save build cache", "    ")
        self.assertIn("id: build-cache", cache)
        self.assertIn("uses: Swatinem/rust-cache@", cache)
        self.assertIn("save-if: ${{ github.ref == 'refs/heads/main' }}", cache)
        outputs = action.split("outputs:\n", 1)[1].split("runs:\n", 1)[0]
        self.assertIn("build-cache-hit:", outputs)
        self.assertIn("value: ${{ steps.build-cache.outputs.cache-hit }}", outputs)

    def test_main_run_that_saves_the_build_cache_also_checks_the_release_profile(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        self.assertIn("id: rust_setup", check)
        step = named_step(check, "Release profile check", "      ")
        for contract in [
            "matrix.skip != 'true'",
            "github.event_name == 'pull_request' ||",
            "github.ref == 'refs/heads/main'",
            "steps.rust_setup.outputs.build-cache-hit != 'true'",
            "cargo check --release --locked $BUILD_ARGS",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, step)
        self.assertNotIn("cargo build --release", check)

    def test_build_jobs_never_hold_a_cache_deletion_token(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        self.assertNotIn("actions: write", check)
        self.assertNotIn("github.token", check)
        self.assertNotIn("cache_prune.py", check)

    def test_release_build_runs_beside_lint_and_test(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        self.assertIn("if: ${{ !cancelled() && needs.plan.result == 'success' && needs.plan.outputs.reused != 'true' }}", check)
        build = workflow.split("  release-build:\n", 1)[1].split("  registry-tool:\n", 1)[0]
        for contract in [
            "name: release build (${{ matrix.os }})",
            "if: ${{ !cancelled() && needs.plan.result == 'success' && github.event_name == 'merge_group' }}",
            "RUSTFLAGS: -D warnings",
            "cache-key: ci-${{ matrix.os }}",
            "cargo build --release --locked $BUILD_ARGS",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, build)
        self.assertNotIn("actions: write", build)
        self.assertNotIn("github.token", build)

    def test_main_warms_the_build_cache_outside_the_tests_workflow(self):
        warm = (ROOT / ".github/workflows/build-cache.yml").read_text()
        tests = (ROOT / ".github/workflows/ci.yml").read_text()
        rustflags = (
            "RUSTFLAGS: -D warnings ${{ matrix.os == 'ubuntu-latest' && "
            "'-C link-arg=-fuse-ld=lld' || matrix.os == 'windows-latest' && "
            "'-C linker=rust-lld -C target-feature=+crt-static' || '' }}"
        )
        # The check job, its release check step and the release build share
        # the cache key the warm job saves, so each restates its RUSTFLAGS.
        self.assertEqual(tests.count(rustflags), 3)
        for contract in [
            "branches: [main]",
            "cancel-in-progress: false",
            "persist-credentials: false",
            "cache-key: ci-${{ matrix.os }}",
            rustflags,
            "steps.rust_setup.outputs.build-cache-hit != 'true'",
            "cargo clippy --locked --keep-going $CLIPPY_ARGS -- -D warnings",
            "cargo check --release --locked $BUILD_ARGS",
            "cargo build --release --locked $BUILD_ARGS",
            "cargo nextest run --locked $TEST_ARGS --no-run",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, warm)
        self.assertNotIn("actions: write", warm)
        self.assertNotIn("github.token", warm)
        versioning = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        self.assertIn("workflows: [tests]", versioning)
        self.assertNotIn("linker=rust-lld", versioning)

    def test_one_gate_job_carries_the_merge_verdict(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        gate = workflow.split("  gate:\n", 1)[1].split("  queue-report:\n", 1)[0]
        for contract in [
            "name: merge gate",
            "needs: [plan, check, release-build, process-windows, registry-tool]",
            "if: ${{ always() && github.event_name != 'push' }}",
            '[ "$PLAN" = success ] || exit 1',
            '[ "$BUILD" = success ] || exit 1',
            '[ "$REUSED" = true ] || [ "$CHECK" = success ] || exit 1',
            '*" failure "*|*" cancelled "*) exit 1 ;;',
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, gate)
        report = workflow.split("  queue-report:\n", 1)[1]
        self.assertIn("needs: [plan, check, release-build, process-windows, registry-tool]", report)

    def test_pull_request_runs_start_only_after_the_review_passes(self):
        for name, first in [("ci.yml", "  plan:\n"), ("codeql.yml", "  analyze:\n")]:
            with self.subTest(workflow=name):
                workflow = (ROOT / ".github/workflows" / name).read_text()
                wait = workflow.split("  review:\n", 1)[1].split(first, 1)[0]
                self.assertIn("if: github.event_name == 'pull_request'", wait)
                self.assertIn("run: python3 .github/scripts/wait_for_review.py", wait)
                gated = workflow.split(first, 1)[1].split("    runs-on:", 1)[0]
                self.assertIn("needs: review", gated)
                self.assertIn("!cancelled() && needs.review.result != 'failure'", gated)
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        stop = workflow.split("  stop-on-failure:\n", 1)[1].split("    runs-on:", 1)[0]
        self.assertIn("needs: review", stop)
        for job in ["check", "process-windows", "release-build", "registry-tool"]:
            with self.subTest(job=job):
                gated = workflow.split(f"  {job}:\n", 1)[1].split("    runs-on:", 1)[0]
                self.assertIn("!cancelled() && needs.plan.result == 'success' && ", gated)

    def test_only_a_pass_verdict_satisfies_the_review_check(self):
        workflow = (ROOT / ".github/workflows/claude-review.yml").read_text()
        self.assertIn("  merge_group:\n", workflow.split("jobs:\n", 1)[0])
        verdict = workflow.split("  verdict:\n", 1)[1]
        for contract in [
            "name: claude review",
            "needs: [review, publish]",
            "if: ${{ always() && (github.event_name == 'pull_request' || github.event_name == 'merge_group') }}",
            '[ "$EVENT" = merge_group ] || [ "$BOT" = true ] || [ "$VERDICT" = pass ]',
            "github.event.pull_request.user.login == 'dependabot[bot]'",
            "github.event.pull_request.head.repo.full_name == github.repository",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, verdict)
        self.assertIn("verdict: ${{ steps.verdict.outputs.verdict }}", workflow)

    def test_no_workflow_wraps_the_compiler(self):
        sources = [ROOT / ".github/actions/rust-setup/action.yml"]
        sources.extend((ROOT / ".github/workflows").glob("*.yml"))
        for path in sources:
            with self.subTest(path=path.name):
                self.assertNotIn("RUSTC_WRAPPER", path.read_text())

    def test_versioning_waits_for_main_ci(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()

        self.assertIn("workflow_run:", workflow)
        self.assertIn("conclusion == 'success'", workflow)
        self.assertIn(
            "SOURCE_CI_RUN_ID: ${{ github.event.workflow_run.id }}", workflow
        )
        self.assertIn('args+=(--run-id "${SOURCE_CI_RUN_ID}")', workflow)
        self.assertNotIn("push:\n    branches: [main]", workflow)

    def test_only_green_main_pushes_share_the_versioning_queue(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        trigger = workflow.split("  workflow_run:\n", 1)[1].split("permissions:", 1)[0]
        self.assertIn("branches: [main]", trigger)
        group = workflow.split("concurrency:\n", 1)[1].split("jobs:\n", 1)[0]
        prepare = workflow.split("  prepare:\n", 1)[1].split("    runs-on:", 1)[0]
        condition = prepare.split("${{", 1)[1].split("}}", 1)[0].strip()
        self.assertIn(" ".join(condition.split()), " ".join(group.split()))
        self.assertIn("&& 'plugin-version-main'", group)
        self.assertIn("|| format('plugin-version-idle-{0}', github.run_id)", group)
        self.assertIn("cancel-in-progress: false", group)

    def test_candidates_are_attested_before_tags_are_created(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()

        attest = workflow.index("release_candidate.py attest")
        tag = workflow.index('git tag "${tag}"')
        self.assertLess(attest, tag)

    def test_candidate_release_set_remains_atomic(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()

        required = [
            "needs.plugin_candidate.result == 'success'",
            "needs.qol_tray_linux_candidate.result == 'success'",
            "needs.qol_tray_macos_candidate.result == 'success'",
            "needs.qol_tray_windows_candidate.result == 'success'",
            'git push --atomic origin "${new_tags[@]}"',
        ]
        for contract in required:
            self.assertIn(contract, workflow)
        attest = named_step(workflow, "Attest exact release candidate", "      ")
        self.assertIn('--plan "${RUNNER_TEMP}/candidate-plan/release-candidate-report.json"', attest)
        self.assertIn('--build-reports "${RUNNER_TEMP}/candidate-builds"', attest)
        reports = named_step(workflow, "Download candidate build reports", "      ")
        self.assertIn("pattern: candidate-*", reports)

    def test_releases_publish_the_attested_candidate_files(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        self.assertNotIn("probe", workflow)
        self.assertFalse((ROOT / ".github/scripts/release_probe.py").exists())
        plugin = workflow.split("  plugin_candidate:\n", 1)[1].split("  qol_tray_linux_candidate:\n", 1)[0]
        upload = named_step(plugin, "Upload release files", "      ")
        for contract in [
            "name: dist-${{ matrix.tag }}-${{ matrix.target }}",
            "retention-days: 90",
            "if-no-files-found: error",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, upload)
        for platform in ["linux", "macos", "windows"]:
            with self.subTest(platform=platform):
                job = workflow.split(f"  qol_tray_{platform}_candidate:\n", 1)[1].split("\n  qol_tray_", 1)[0].split("\n  tag_and_dispatch:", 1)[0]
                upload = named_step(job, "Upload release files", "      ")
                self.assertIn(f"name: dist-${{{{ needs.prepare.outputs.qol_tray_tag }}}}-{platform}", upload)
                self.assertIn("retention-days: 90", upload)
        for name, needs in [("release.yml", "setup"), ("qol-tray-release.yml", "gate")]:
            with self.subTest(workflow=name):
                release = (ROOT / ".github/workflows" / name).read_text()
                self.assertNotIn("release_candidate.py build", release)
                self.assertNotIn("cache-key: plugin-release-", release)
                self.assertNotIn("cache-key: qol-tray-", release)
                download = named_step(release, "Download candidate files", "      ")
                self.assertIn(f"run-id: ${{{{ needs.{needs}.outputs.candidate_run }}}}", download)
                self.assertIn("github-token: ${{ github.token }}", download)
                self.assertIn("pattern: dist-${{ env.RELEASE_TAG }}-*", download)
                self.assertIn("candidate_run: ${{ steps.verify.outputs.run_id }}", release)
                self.assertIn("actions: read", release)
                self.assertNotIn("actions: write", release.split("  index:\n", 1)[0])

    def test_release_workflows_verify_exact_candidate(self):
        for name in ["release.yml", "qol-tray-release.yml"]:
            with self.subTest(workflow=name):
                workflow = (ROOT / ".github/workflows" / name).read_text()
                verify = workflow.index("release_candidate.py verify")
                download = workflow.index("- name: Download candidate files")
                self.assertLess(verify, download)

    def test_ci_runs_release_profile_builds(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()

        self.assertIn("merge_group:", workflow)
        self.assertIn("cargo check --release --locked $BUILD_ARGS", workflow)
        self.assertIn("cargo build --release --locked $BUILD_ARGS", workflow)
        self.assertIn("if: ${{ !cancelled() && needs.plan.result == 'success' && github.event_name == 'merge_group' }}", workflow)
        self.assertIn("RUSTFLAGS: -D warnings", workflow)
        self.assertNotIn("debug-assertions", workflow)
        self.assertIn("timeout-minutes:", workflow)

    def test_ci_matches_the_release_pipeline_lockfile_strictness(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()

        lenient = [
            line.strip()
            for line in workflow.splitlines()
            if CARGO_GATE.search(line)
            and not line.strip().startswith("#")
            and "--locked" not in line
        ]
        self.assertEqual(
            lenient,
            [],
            "every CI cargo gate must pass --locked so a stale Cargo.lock cannot "
            "reach the --locked release pipeline",
        )

    def test_ci_asserts_lockfile_freshness_with_a_full_resolve(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()

        self.assertIn("cargo metadata --locked --format-version 1", workflow)
        self.assertNotIn(
            "cargo metadata --locked --no-deps",
            workflow,
            "--no-deps skips resolution and exits 0 against a stale lockfile",
        )

    def test_nextest_preserves_affected_scope_and_documentation_on_each_platform(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check_job = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        test_steps = re.findall(r"      - name: Test\n(.*?)(?=\n      - name:|\Z)", check_job, re.S)
        self.assertEqual(len(test_steps), 2)
        self.assertRegex(check_job, r"tool: cargo-nextest@\d+\.\d+\.\d+")
        for step in test_steps:
            with self.subTest(step=step.splitlines()[0]):
                self.assertIn("TEST_ARGS: ${{ matrix.test_args }}", step)
                self.assertIn("DOCTEST: ${{ matrix.doctest }}", step)
                self.assertIn('if [ "$DOCTEST" = "true" ]; then', step)
                self.assertIn(
                    "cargo nextest run --locked $TEST_ARGS --no-fail-fast --retries 0 --no-tests=pass --ignore-default-filter",
                    step,
                )
                self.assertIn("cargo test --locked $TEST_ARGS --doc", step)
                self.assertLess(step.index("cargo nextest run"), step.index("cargo test"))

    def test_windows_dev_build_tests_follow_the_affected_plan(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()

        for contract in [
            "windows_dev_build: ${{ steps.affected.outputs.windows_dev_build }}",
            "fromJSON(needs.plan.outputs.windows_dev_build || 'false')",
            "if: ${{ fromJSON(needs.plan.outputs.windows_dev_build) }}",
            "run: cargo test --locked -p qol-dev-build --all-targets",
        ]:
            self.assertIn(contract, workflow)

    def test_windows_lints_and_tests_the_affected_plan_like_the_other_runners(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        check = workflow.split("  check:\n", 1)[1].split("  process-windows:\n", 1)[0]
        for key in ["clippy", "build", "test", "doctest", "skip"]:
            with self.subTest(key=key):
                self.assertIn(
                    f"windows_{key}: ${{{{ steps.affected.outputs.windows_{key} }}}}",
                    workflow,
                )
        for contract in [
            "          - os: windows-latest\n",
            "clippy_args: ${{ needs.plan.outputs.windows_clippy }}",
            "build_args: ${{ needs.plan.outputs.windows_build }}",
            "test_args: ${{ needs.plan.outputs.windows_test }}",
            "doctest: ${{ needs.plan.outputs.windows_doctest }}",
            "skip: ${{ needs.plan.outputs.windows_skip }}",
            "    defaults:\n      run:\n        shell: bash\n",
            "if: runner.os != 'Linux' && matrix.skip != 'true' && github.event_name != 'pull_request'",
            "cargo clippy --locked --keep-going $CLIPPY_ARGS -- -D warnings",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, check)

    def test_main_warms_the_release_build_cache_on_every_os(self):
        warm = (ROOT / ".github/workflows/build-cache.yml").read_text()
        self.assertNotIn("release_build", warm)
        for contract in [
            "          - os: windows-latest\n            plan: windows\n",
            "          - os: ubuntu-latest\n            plan: ubuntu\n",
            "          - os: macos-latest\n            plan: macos\n",
            "    defaults:\n      run:\n        shell: bash\n",
            "          cargo build --release --locked $BUILD_ARGS\n",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, warm)

    def test_tray_release_ships_the_windows_executable_and_installer_bundle(self):
        versioning = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        workflow = (ROOT / ".github/workflows/qol-tray-release.yml").read_text()
        job = versioning.split("  qol_tray_windows_candidate:\n", 1)[1].split("  tag_and_dispatch:\n", 1)[0]
        for contract in [
            "runs-on: windows-latest",
            "RUSTFLAGS: -D warnings -C link-arg=/Brepro -C target-feature=+crt-static",
            "cache-key: qol-tray-windows",
            "--kind qol-tray-windows",
            'cp "${BUILT}/qol-tray.exe" "dist/${BUNDLE}.exe"',
            'cp "${BUILT}/qol-tray.exe" "${BUILT}/qol-tray-install.exe" "${BUNDLE}/"',
            '7z a -tzip "dist/${BUNDLE}.zip" "${BUNDLE}"',
            "BUNDLE=qol-tray-windows-x86_64",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, job)
        release = workflow.split("  release:\n", 1)[1]
        self.assertIn("needs: gate", release)
        self.assertIn("for platform in linux macos windows; do", named_step(release, "Require every platform", "      "))
        updater = (ROOT / "apps/tray/src/updates/platform/windows.rs").read_text()
        self.assertIn('format!("qol-tray-windows-{}.exe", std::env::consts::ARCH)', updater)

    def test_merge_queue_builds_windows_release_binaries(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        sandbox = workflow.split("  process-windows:\n", 1)[1].split("  release-build:\n", 1)[0]
        build = workflow.split("  release-build:\n", 1)[1].split("  registry-tool:\n", 1)[0]

        self.assertNotIn("windows_apps", workflow)
        self.assertNotIn("cargo build", sandbox)
        self.assertNotIn("upload-artifact", sandbox)
        self.assertIn(
            "RUSTFLAGS: -D warnings -C linker=rust-lld -C target-feature=+crt-static", sandbox
        )
        for contract in [
            "          - os: windows-latest\n"
            "            build_args: ${{ needs.plan.outputs.windows_build }}\n"
            "            skip: ${{ needs.plan.outputs.windows_skip }}\n",
            "timeout-minutes: ${{ matrix.os == 'windows-latest' && 75 || 45 }}",
            "    defaults:\n      run:\n        shell: bash\n",
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, build)

    def test_windows_builds_link_the_c_runtime_statically(self):
        for name in [
            "ci.yml",
            "build-cache.yml",
            "release.yml",
            "plugin-version.yml",
            "qol-tray-release.yml",
        ]:
            workflow = (ROOT / ".github/workflows" / name).read_text()
            for line in workflow.splitlines():
                if "RUSTFLAGS:" in line and ("windows" in line or "/Brepro" in line):
                    with self.subTest(workflow=name, line=line.strip()):
                        self.assertIn("-C target-feature=+crt-static", line)
                        self.assertIn("-D warnings", line)

    def test_windows_release_jobs_get_a_longer_timeout(self):
        conditional = "timeout-minutes: ${{ matrix.os == 'windows-latest' && 75 || 45 }}"
        cases = [
            ("plugin-version.yml", "  plugin_candidate:\n", "  qol_tray_linux_candidate:\n", conditional),
            ("plugin-version.yml", "  qol_tray_windows_candidate:\n", "    steps:\n", "timeout-minutes: 75"),
        ]
        for name, start, end, timeout in cases:
            with self.subTest(workflow=name, job=start.strip()):
                workflow = (ROOT / ".github/workflows" / name).read_text()
                job = workflow.split(start, 1)[1].split(end, 1)[0]
                self.assertIn(timeout, job)

    def test_each_tray_build_publishes_to_the_index_without_waiting_for_the_others(self):
        versioning = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        publish = (ROOT / ".github/workflows/qol-tray-host-publish.yml").read_text()
        for platform in ["linux", "macos", "windows"]:
            with self.subTest(platform=platform):
                job = versioning.split(f"  qol_tray_{platform}_publish:\n", 1)[1].split("\n  qol_tray_", 1)[0]
                needs = job.split("    needs:\n", 1)[1].split("    if:", 1)[0]
                self.assertEqual(needs.split(), ["-", "prepare", "-", f"qol_tray_{platform}_candidate"])
                self.assertIn("uses: ./.github/workflows/qol-tray-host-publish.yml", job)
                self.assertIn(f"os: {platform}", job)
        attest = publish.split("  attest:\n", 1)[1].split("\n  registry:\n", 1)[0]
        self.assertIn("actions/attest-build-provenance@", attest)
        self.assertNotIn("cargo", attest)
        registry = publish.split("  registry:\n", 1)[1].split("\n  index:\n", 1)[0]
        self.assertIn("needs: attest", registry)
        self.assertIn("persist-credentials: false", registry)
        self.assertNotIn("id-token", registry)
        build = named_step(registry, "Build registry tool", "      ")
        self.assertNotIn("token", build)
        index = publish.split("  index:\n", 1)[1]
        self.assertIn("needs: registry", index)
        self.assertIn("gh workflow run plugin-index.yml --ref main", index)

    def test_tray_release_builds_drop_checkout_credentials(self):
        workflow = (ROOT / ".github/workflows/plugin-version.yml").read_text()
        for job, end in [
            ("  qol_tray_linux_candidate:\n", "  qol_tray_macos_candidate:\n"),
            ("  qol_tray_macos_candidate:\n", "  qol_tray_windows_candidate:\n"),
            ("  qol_tray_windows_candidate:\n", "  tag_and_dispatch:\n"),
        ]:
            with self.subTest(job=job.strip()):
                body = workflow.split(job, 1)[1].split(end, 1)[0]
                self.assertIn("persist-credentials: false", body)

    def test_plugin_release_builds_run_bash_on_windows(self):
        cases = [
            (".github/workflows/plugin-version.yml", "  plugin_candidate:\n", "  qol_tray_linux_candidate:\n"),
            (".github/workflows/plugin-version.yml", "  qol_tray_windows_candidate:\n", "  tag_and_dispatch:\n"),
        ]
        for path, start, end in cases:
            with self.subTest(path=path, job=start.strip()):
                workflow = (ROOT / path).read_text()
                job = workflow.split(start, 1)[1].split(end, 1)[0]
                self.assertIn("    defaults:\n      run:\n        shell: bash\n", job)

    def test_plugins_publish_no_github_release(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()

        self.assertNotIn("gh release", workflow)
        self.assertNotIn("contents: write", workflow)

    def test_plugin_registry_push_waits_for_every_target_file(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        registry = workflow.split("  registry:\n", 1)[1].split("  index:\n", 1)[0]
        self.assertIn("needs: setup", registry)
        self.assertLess(
            registry.index("release_candidate.py require-release-files"),
            registry.index("- name: Push registry artifact"),
        )
        self.assertIn("packages: write", registry)
        self.assertIn("ref: ${{ env.RELEASE_REF }}", registry)
        push = named_step(registry, "Push registry artifact", "      ")
        for contract in [
            "set -euo pipefail",
            '--registry "ghcr.io/${GITHUB_REPOSITORY_OWNER}/plugins"',
            '--tag "${RELEASE_TAG}" --files release_files',
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, push)

    def test_registry_tool_compiles_in_a_step_that_holds_no_token(self):
        jobs = {
            "release.yml": ("  registry:\n", "  index:\n"),
            "plugin-index.yml": ("  build:\n", "  sign:\n"),
            "plugin-registry-backfill.yml": ("  backfill:\n", "  index:\n"),
            "ci.yml": ("  registry-tool:\n", "  gate:\n"),
        }
        for name, (begin, finish) in jobs.items():
            with self.subTest(workflow=name):
                workflow = (ROOT / ".github/workflows" / name).read_text()
                job = workflow.split(begin, 1)[1].split(finish, 1)[0]
                self.assertIn("persist-credentials: false", job)
                self.assertIn("cache-key: plugin-registry", job)
                self.assertIn('system-dependencies: "false"', job)
                self.assertIn("RUSTFLAGS: -D warnings -C link-arg=-fuse-ld=lld", job)
                self.assertNotIn("actions: write", job)
                build = named_step(job, "Build registry tool", "      ")
                self.assertIn("cargo build --locked -p qol-plugin-registry", build)
                self.assertNotIn("github.token", build)
                self.assertNotIn("secrets.", build)
                self.assertEqual(job.count("cargo "), 1)

    def test_plugin_release_refreshes_the_signed_index(self):
        release = (ROOT / ".github/workflows/release.yml").read_text()
        dispatch = release.split("  index:\n", 1)[1]
        self.assertIn("needs: registry", dispatch)
        self.assertIn("actions: write", dispatch)
        self.assertIn("gh workflow run plugin-index.yml --ref main", dispatch)

        workflow = (ROOT / ".github/workflows/plugin-index.yml").read_text()
        build = workflow.split("  build:\n", 1)[1].split("  sign:\n", 1)[0]
        sign = workflow.split("  sign:\n", 1)[1].split("  deploy:\n", 1)[0]
        deploy = workflow.split("  deploy:\n", 1)[1]
        self.assertIn("cancel-in-progress: false", workflow)
        self.assertIn("if: github.ref == 'refs/heads/main'", build)
        self.assertNotIn("environment:", build)
        self.assertNotIn("secrets.", build)
        self.assertIn("needs: build", sign)
        self.assertIn("if: needs.build.outputs.changed == 'true'", sign)
        self.assertIn("name: plugin-index\n      deployment: false", sign)
        compare = named_step(build, "Compare with the deployed index", "      ")
        self.assertIn("jq -S 'del(.serial)' index.json", compare)
        self.assertIn("secrets.PLUGIN_INDEX_MINISIGN_KEY", sign)
        self.assertIn("-x site/plugins/index.json.minisig", sign)
        for job in (build, sign):
            self.assertNotIn("pages: write", job)
        self.assertIn("needs: sign", deploy)
        self.assertIn("pages: write", deploy)
        self.assertNotIn("secrets.", deploy)

    def test_plugin_index_signs_in_a_job_that_compiles_nothing(self):
        workflow = (ROOT / ".github/workflows/plugin-index.yml").read_text()
        build = workflow.split("  build:\n", 1)[1].split("  sign:\n", 1)[0]
        sign = workflow.split("  sign:\n", 1)[1].split("  deploy:\n", 1)[0]
        public_key = re.search(r"^  PUBLIC_KEY: (\S+)$", workflow, re.MULTILINE).group(1)
        self.assertTrue((ROOT / public_key).is_file(), public_key)
        self.assertNotIn("apt-get", workflow)

        index = named_step(build, "Build index", "      ")
        for contract in [
            "set -euo pipefail",
            '--public-key "${PUBLIC_KEY}"',
            '--previous-url "${INDEX_URL}"',
        ]:
            with self.subTest(contract=contract):
                self.assertIn(contract, index)

        for compiles in ["cargo", "rust-setup", "github.token"]:
            with self.subTest(compiles=compiles):
                self.assertNotIn(compiles, sign)
        self.assertIn(str(Path(public_key).parent), sign)
        install = named_step(sign, "Install minisign", "      ")
        self.assertIn("sha256sum --check --strict", install)
        signing = named_step(sign, "Sign index", "      ")
        self.assertIn("trap 'rm -f \"${key}\"' EXIT", signing)
        self.assertIn("< /dev/null", signing)
        verify = named_step(sign, "Verify with the key the tray ships", "      ")
        self.assertIn('minisign -V -p "${PUBLIC_KEY}" -m site/plugins/index.json', verify)
        bundle = named_step(sign, "Bundle the signature with the index", "      ")
        self.assertIn("> site/plugins/index.signed.json", bundle)
        self.assertIn("jq -j .index site/plugins/index.signed.json | cmp - site/plugins/index.json", bundle)
        self.assertIn(
            "jq -j .signature site/plugins/index.signed.json | cmp - site/plugins/index.json.minisig",
            bundle,
        )
        self.assertRegex(build, r"INDEX_URL: [^\n]+/plugins/index\.signed\.json\n")

        secret_steps = [step for step in workflow.split("\n      - ") if "secrets." in step]
        self.assertEqual(len(secret_steps), 1)
        self.assertIn("name: Sign index", secret_steps[0])
        order = [
            "- name: Install minisign",
            "actions/download-artifact",
            "- name: Sign index",
            "- name: Verify with the key the tray ships",
            "- name: Bundle the signature with the index",
            "actions/upload-pages-artifact",
        ]
        positions = [sign.index(marker) for marker in order]
        self.assertEqual(positions, sorted(positions))

    def test_registry_backfill_pushes_only_from_main(self):
        workflow = (ROOT / ".github/workflows/plugin-registry-backfill.yml").read_text()
        backfill = workflow.split("  backfill:\n", 1)[1].split("  index:\n", 1)[0]
        dispatch = workflow.split("  index:\n", 1)[1]
        self.assertIn("if: github.ref == 'refs/heads/main'", backfill)
        self.assertIn("packages: write", backfill)
        self.assertIn("fetch-depth: 0", backfill)
        self.assertIn("needs: backfill", dispatch)
        self.assertIn("actions: write", dispatch)
        self.assertNotIn("cargo", dispatch)

    def test_tray_publish_claims_latest(self):
        workflow = (ROOT / ".github/workflows/qol-tray-release.yml").read_text()

        self.assertIn(
            'gh release edit "${RELEASE_TAG}" --draft=false --latest=true',
            workflow,
        )
        self.assertNotIn("--latest=false", workflow)

    def test_release_creation_steps_carry_an_explicit_latest_policy(self):
        for workflow_path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            with self.subTest(workflow=workflow_path.name):
                workflow = workflow_path.read_text()
                lines = workflow.splitlines()
                for index, line in enumerate(lines):
                    if "gh release create" in line:
                        self.assertIn(
                            "--latest",
                            line,
                            f"release creation must carry an explicit latest flag: "
                            f"{workflow_path.name}:{index + 1}",
                        )
                    if "action-gh-release" in line:
                        block = lines[index + 1 : index + 12]
                        self.assertTrue(
                            any("make_latest:" in b for b in block),
                            f"action-gh-release step must declare make_latest: "
                            f"{workflow_path.name}:{index + 1}",
                        )


if __name__ == "__main__":
    unittest.main()
