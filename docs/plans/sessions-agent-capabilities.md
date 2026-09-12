# Sessions agent capability admission

## User outcome

Sessions must represent what a particular configured agent can do and what the user trusts it to do, separately from terminal read/focus/input permissions. Assignment requirements must be checked before work is dispatched, including reuse, resume, fork, grouped launch and subsequent submit. This is assignment policy, not a sandbox or proof of model competence.

The user's visual preference is OpenAI first, Anthropic an allowed alternative, and DeepSeek/GLM not trusted as final visual judges. Keep vendor preferences in user-owned configuration, never hardcode model/provider rankings or infer capabilities from name substrings. Keep the existing spending allowlist independent: no automatic model upgrade, fallback launch, or widening allowed_models.

## Scope and ownership

One implementation lane owns these exact paths recursively:
- tools/qol-cli/src/commands/sessions/
- tools/qol-cli/src/cli/contract.rs (only sessions help/contract references if needed)
- docs/notes/sessions-agent-capabilities.md (new maintained documentation)

Root owns this plan. No other source edits. In particular no plugin UI, library capability enum changes, manifests, lockfile, host configuration, installed binaries, generated skill-plugin clients or cloud operations. Source MCP schemas and their pi exporter belong to the sessions owner and must describe the new contract. The cross-repository generated client refresh will be handled after source acceptance; report the exact existing export command. Read applicable repository/skill instructions before edits. No source comments.

Discovery established that SessionCapabilities is terminal I/O, while orchestration configuration and admission belong to the CLI sessions module. Extend that owner with a dedicated agent_policy.rs module rather than add agent policy to terminal transport. No new dependency, daemon, score engine, receipt signatures or image relay implementation.

## Data contract

Use these names consistently in TOML, Rust serde, CLI and MCP:
- agent_profiles: map of named profiles. Each profile declares tool, model, optional provider (informational), roles (closed enum scout/implement/architect/review/debug), image_input (none/native/unknown), visual_review (deny/allow), and optional preference integer (lower sorts first). Defaults for technical/trust fields are unknown and deny. Reject unknown role/image/trust values and profile keys; reject empty identities, duplicate roles where practical. Provider never grants permission.
- default_agent_profile: optional name, must resolve.
- enforce_agent_profiles: boolean. Missing defaults to false only when no profiles are configured; configuring any profiles enables enforcement unless explicitly disabled. Document the opt-out as unconstrained compatibility, never as checked admission.
- On spawn/fork/submit: agent_profile optional name, task_role optional closed enum, requires optional list of closed enum image_input/visual_review. Lane set entries support these fields independently; single/top-level fields must have clear documented default inheritance and reject ambiguous conflicting fields.
- Constrained mode is active if enforcement is enabled OR any new assignment field is present. In constrained mode a resolvable explicit/default profile and explicit task_role are required, even if requires is empty. Role must be allowed by profile. image_input requires native; visual_review requires native plus allow. Unknown fails closed. Nonvisual implementers can implement written specifications with requires empty, even on visually oriented projects.
- Unconfigured legacy calls remain supported, with explicit unknown/unconstrained agent status in output. Do not advertise them as eligible for declared visual requirements.

Profile tool/model must match the actual selected launch identity and spending policy. With a selected profile, its model is the default when no explicit model is supplied; a conflicting explicit tool/model is an error. No automatic preference-based launch. Discovery may show profiles sorted by preference so architect can choose within budget.

Represent the immutable resolved assignment in one reusable struct containing profile name, declared tool/model/provider, permitted roles, image/trust declarations, task_role, requires, and evidence_basis=configuration_declared. Serialize as agent_assignment on outcomes and receipts; expose profile eligibility through sessions capability and agent_assignment/status on list. Keep existing terminal capabilities untouched.

## Enforcement and persistence

Read policy fresh for each dispatch operation; do not silently keep permissions granted by an old MCP server startup snapshot. Group preflight resolves every lane against one consistent snapshot before launching the first lane; policy incompatibility must create no terminal or pending round. Runtime launch failures can still produce partial sets and must be reported honestly.

Keep admission in shared orchestration functions reached by CLI and MCP, not only schema or prompt validation. Close the observed MCP fork spending-allowlist bypass in that common boundary. Human terminal diagnostics are outside this admission boundary; do not claim to constrain arbitrary shell execution.

Persist resolved assignment with spawn/fork identity and checkpoint BEFORE the relevant work is dispatched, and propagate checkpoint assignment into the durable round receipt. Backward fields use serde defaults, old receipts remain readable. Preserve existing session/marker/report identity checks. Receipt means policy was checked for assignment, not that the model passed its work or produced trusted visual evidence.

For live reuse, consult recorded identity associated with the live session token, not caller-supplied model or a stale key-only record. Refuse constrained reuse without an identity-bound assignment; never relabel an unmanaged live session with a new trusted profile. Reject conflicting model/profile requests rather than reporting a model switch that did not occur. Correct the existing reused-outcome model reporting even in legacy mode when a bound record exists.

For fresh resume, a prior incompatible/missing assignment cannot be promoted through resume; fail constrained resume with an actionable instruction to use resume=false for a fresh session. A compatible recorded profile still must satisfy CURRENT policy; revocation must block reuse/submit/resume. Bind the fresh launch record to its returned session token. Runtime model changes performed inside a harness cannot be attested by this feature; label identities configuration-declared and document that limitation. Do not attempt screen parsing or guessed model detection.

Submit inherits the recorded profile when omitted; task_role may inherit the last constrained assignment if omitted, but explicit new requirements must be checked. Current policy revalidation is mandatory. All fresh task-carrying bridge paths must use the same admission check; collection-only bridge calls remain unchanged. Server prompt includes the resolved role/requirements and visual-review permission as a directive, but prompts are not the enforcement mechanism.

## Tests and delivery evidence

Implementation lane writes meaningful tests but runs no build/test/lint/format/git/browser commands. Cover:
- complete truth table of image input and visual permission; unknown and unsupported roles/requirements reject;
- no vendor-name capability inference; profile model/tool conflicts and spending allowlist rejection;
- CLI/MCP parity for spawn, fork and submit; grouped invalid second lane causes zero launches;
- unknown legacy records cannot satisfy constrained work; live reuse uses bound recorded identity; revoked policy and changed requirements reject; constrained resume cannot silently upgrade a profile;
- stored assignment survives checkpoint/receipt serialization and old formats still parse;
- terminal capabilities retain their meaning; emitted tool schemas and exported pi contract contain the new fields;
- precise rejection diagnostics name profile, requirement and reason without printing unrelated configuration or credentials.

Central root gate: personal full diff review, cargo fmt --all -- --check, cargo test -p qol, cargo clippy -p qol --all-targets -- -D warnings, cargo build -p qol, and repository check through cargo run -p qol -- check (assess unrelated pre-existing failures separately). Headless admission fixtures must use fake terminal services, no host live-lane launch smoke. Desktop runtime verification, if needed, goes in a disposable guest. After independent review and acceptance, one local squash commit on main, preserve unrelated dirty GPUI changes, no push.

## Literal audit

Root fixed-string search recorded in /tmp/sessions-capability-literals.txt: SpawnArgs, LaneSpec, SpawnRecord, session_row(, tool_spawn, tool_submit, tool_fork, StoredCheckpoint and allowed_models. All 62 hits are in the implementation-owned sessions subtree. The broader CLI contract is an explicitly owned compatibility consumer. Existing terminal SessionCapabilities is read-only transport context. No literal-removal claim is made outside those owners.

## Correction round 1

Root gate failed: cargo test reports 14 compiler errors and an unused flag warning in /tmp/sessions-capabilities-test.log; cargo fmt check differences are in /tmp/sessions-capabilities-fmt.log. No acceptance. Independent review report: /home/kmrh47/.local/share/qol-tray/sessions/lanes/rounds/7913db2afd03da5fe579be5197bbb5370c9357278e4343e8dce9fe25f770e0ca/report.md.

Required corrections, same ownership:
1. Fix missing test-only enum imports, SpawnArgs Debug requirement, all old run_with test callers, unused flag, test-only helper cfg gates and widened publish clippy issue. Use the actual compiler/format output; do not run commands yourself. Do not add production unused imports to fix test compilation.
2. Group preflight must evaluate the real intended decision for ALL lanes: live reuse identity, recorded resume compatibility, launch eligibility and requirements under the same snapshot. A known policy mismatch in a later reused/resumed lane must result in zero terminals and zero pending rounds. Keep runtime rechecks for state changes, but do not describe already-known identity mismatch as an ordinary launch failure. Add both reuse and resume second-lane regressions.
3. Constrained resume currently compares only profile names. Revalidate the full prior recorded tool/model and assignment under CURRENT policy before resuming. Same profile renamed to a different model or revoked prior requirements cannot silently produce a newly trusted resume. Use resume=false to explicitly start fresh. Add same-profile changed-model and revoked-visual-permission regressions.
4. Fork profile model must supply the default when explicit --model is absent, as on spawn. Remove the new documentation exception, preserve rejection of explicit conflicts and spending limits, and make CLI/MCP schemas consistent.
5. requires=[] must remain explicitly empty across pi exporter, CLI parser, MCP and submit inheritance. Do not simply drop the flag: that would inherit an old visual requirement on submit instead of clearing it. Add roundtrip coverage for absent versus empty. No duplicated bespoke parser workaround across three commands.
6. Remove the inaccurate claim that next currently reports assignment, unless implementing that straightforward additive output consistently. Do not weaken spec claims to excuse bypasses.
7. Root also requires spending-policy revocation to apply before subsequent constrained work is submitted: submit consumes paid model work even without spawning a new process. Check allowed_models in admit_submit, with a regression for a now-disallowed recorded model. No change to the existing empty-allowlist legacy meaning.
8. Legacy reuse with a token-bound SpawnRecord but no agent_assignment still ignores that record's model, since recorded_assignment drops it. Correct the outcome to the known recorded model and reject an explicit conflicting model without silently relabeling. Unmanaged sessions without any record must not imply their current model was verified. Add a no-assignment-but-bound-record test.

No feature expansion, code comments, build/test/lint/format/git commands, host config or installed client changes. Root will run the central gate again. Report exact edits and unresolved issues.

## Correction round 2

The central R2 gate builds successfully, but tests fail before execution and strict lint fails. Fix only these concrete defects, retaining all policy semantics from round 1:

1. In spawn.rs tests near lines 6062-6097, three `super::fork` references resolve inside spawn, not sessions. Use the actual sibling sessions::fork module consistently for ForkStore and fork calls. See /tmp/sessions-capabilities-test-r2.log.
2. `config_spawn_model` has no callers and `config_spawn_model_at` only has test callers after the fresh DispatchPolicy loader migration. Remove obsolete production wrappers and adapt existing configuration tests to exercise the actual DispatchPolicy loader rather than retaining dead production code or suppressing warnings. Preserve missing/explicit/blank model behavior. See /tmp/sessions-capabilities-clippy-r2.log.
3. Apply the exact formatting changes in /tmp/sessions-capabilities-fmt-r2.log by edits. New fixes must follow surrounding Rust formatting. Do not execute formatter commands.

Owned paths remain tools/qol-cli/src/commands/sessions/ recursively, tools/qol-cli/src/cli/contract.rs for sessions-only references, and docs/notes/sessions-agent-capabilities.md. Do not edit this root-owned plan. No build, test, lint, format, or git commands, no code comments, and no em-dash characters. Report changed files and lines, plus conscious deviations only. Root reruns verification after completion.

## Correction round 3

Central R3: formatting and build pass; 1272 tests pass and 7 fail; strict lint reports 3 findings. Logs: /tmp/sessions-capabilities-test-r3.log and /tmp/sessions-capabilities-clippy-r3.log.

1. Preserve actionable policy diagnostics through group preflight and prior-resume wrapping. Five failing tests correctly expect the underlying reason, but anyhow context followed by to_string() in callers drops it. Make these wrapper messages include the full underlying cause using alternate error formatting, preserving lane key, no-launch statement, and resume=false instruction. Keep the existing reason assertions and zero-launch assertions. Ensure MCP callers also receive the useful message, rather than fixing tests to inspect a chain users never see.
2. The submit inheritance test expects an old informational provider even though each new assignment snapshots current policy. Keep current-policy snapshot semantics; update the test to explicitly show changed informational provider comes from current configuration while role and requirements inherit. Do not change immutable previous receipts.
3. Update the canonical loop-close receipt test for the required agent_status=unconstrained addition, retaining exact final_report checks. Include the managed receipt assignment case if existing coverage is absent.
4. Fix the two Box::new(default) lint findings with Box::default and the err().expect() finding with expect_err. Do not suppress these lints.
5. Correct docs/notes/sessions-agent-capabilities.md identity-binding bullet: background launch embeds admitted task in launch argv, so the new session token cannot be persisted before that launch begins. State policy checked before launch, assignment included in prompt, token binding/checkpoint after discovery; foreground submit checkpoints precede text delivery. Never claim atomic pre-delivery token persistence for the background path. No change to launch ordering in this correction.

Same owned paths and edit-only prohibitions as correction round 2. No build/test/lint/format/git commands, comments, or em-dash characters. Report changed files/lines and deviations only.

## Correction round 4

Central R4: 1279 tests pass, one fails; strict lint and build pass. Formatting has one wrapping difference in /tmp/sessions-capabilities-fmt-r4.log.

The remaining test uncovered a production defect, not a stale assertion: recorded_identity in spawn.rs loads the token-bound ledger record but only uses its model, discarding its agent_assignment. When the checkpoint is absent, valid managed sessions become incorrectly unmanaged. Resolve assignment from the checkpoint first, then fall back to the token-bound ledger record's assignment. Preserve the separate legacy model fallback, explicit model conflict detection, and no key-only trust. Do not weaken the existing grouped mismatch test.

Add focused regressions proving ledger-only managed assignment lookup/reuse succeeds, and checkpoint assignment takes precedence over older ledger metadata. The existing unmanaged ledger model regression must remain. Manually apply /tmp/sessions-capabilities-fmt-r4.log. Same owned paths and edit-only constraints as previous corrections. Report exact changes and deviations only.

## Final formatting and independent review

R5 central tests: 1283 passed, zero failures. Apply only the two wrapping changes from /tmp/sessions-capabilities-fmt-r5.log in spawn.rs. Formatting lane owns only spawn.rs and must not make semantic edits or add tests.

Independent reviewer owns no files. Read-only review scope is tools/qol-cli/src/commands/sessions/ recursively, tools/qol-cli/src/cli/contract.rs sessions entries, and docs/notes/sessions-agent-capabilities.md against this plan. Revisit policy rejection, current-policy revocation, token-bound ledger/checkpoint precedence, full group preflight, fork defaults, absent versus empty requirements, and truthful receipts. The formatter concurrently changes only whitespace in two new test fixture calls near spawn.rs:6229/6241. Do not report style preferences or re-open accepted declared-capability versus runtime-attestation scope. Report only concrete remaining correctness defects with file/line, trigger, impact, required fix and confidence; if none, say none and list reviewed paths/limits. Do not run build, test, lint, format, or git commands; no edits or code comments or em-dash characters. Root owns the final gate and acceptance.
