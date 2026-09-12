# Sessions agent capability admission

Sessions represent two different things and must never conflate them: what a terminal transport can do (screen reading, focus, text input) and what a configured agent is trusted to do.
This note documents the second one, the agent assignment policy added to the CLI sessions orchestration.
It is assignment policy, not a sandbox, and it is not evidence that a model is competent.
The existing terminal `SessionCapabilities` keep their meaning untouched.

## What the user configures

Agent profiles live in the same `sessions.toml` the spawn settings already use (`~/.config/qol-tray/sessions.toml` on Linux, `~/Library/Application Support/qol-tray/sessions.toml` on macOS).

```toml
default_agent_profile = "visual-reviewer"
enforce_agent_profiles = true

[agent_profiles.visual-reviewer]
tool = "claude"
model = "opus"
provider = "informational only"
roles = ["review", "architect"]
image_input = "native"
visual_review = "allow"
preference = 10

[agent_profiles.implementer]
tool = "pi"
model = "flash"
roles = ["implement", "debug"]
image_input = "unknown"
visual_review = "deny"
preference = 20
```

Fields:

- `tool` and `model` are required and must be non-empty. They are bound to the harness the profile declares.
- `provider` is optional and informational. It never grants a permission.
- `roles` is required and closed: `scout`, `implement`, `architect`, `review`, `debug`. Repeats are rejected.
- `image_input` defaults to `unknown` and is closed: `none`, `native`, `unknown`.
- `visual_review` defaults to `deny` and is closed: `deny`, `allow`.
- `preference` is an optional integer; lower sorts first. It only orders discovery output.
- Unknown keys inside a profile, unknown enum values, and empty identities are rejected at parse time.

`agent_profiles` is a map of named profiles.
`default_agent_profile` is optional but must resolve to a configured profile.
`enforce_agent_profiles` is optional; configuring any profile turns enforcement on unless it is explicitly set to `false`.

The opt-out is unconstrained compatibility, never checked admission.
With enforcement off, an unmanaged dispatch proceeds exactly as before, but it is still reported as `unconstrained` and is never advertised as eligible for a declared visual requirement.

## Assignment fields

`session_spawn`, `session_submit`, `session_fork`, `qol sessions spawn`, `qol sessions submit` and `qol sessions fork` accept:

- `agent_profile` (CLI `--agent-profile`): a named `agent_profiles` entry.
- `task_role` (CLI `--task-role`): one of the closed role enum.
- `requires` (CLI `--requires`, comma-separated): a list drawn from `image_input` and `visual_review`.

A lane set carries the same fields on each lane entry and accepts them once at the top level as a default for every lane.
Setting the same field both at the top level and on a lane is refused as ambiguous.

An assignment is constrained when `enforce_agent_profiles` is effectively on or when any assignment field is present.
In constrained mode a resolvable profile and an explicit `task_role` are required even when `requires` is empty.
The role must be permitted by the profile, and `requires` must be satisfiable:

| declared `image_input` | declared `visual_review` | `requires = []` | `requires = [image_input]` | `requires = [visual_review]` |
|---|---|---|---|---|
| `none` | `deny` | allowed | rejected | rejected |
| `unknown` | `deny` | allowed | rejected | rejected |
| `unknown` | `allow` | allowed | rejected | rejected |
| `native` | `deny` | allowed | allowed | rejected |
| `native` | `allow` | allowed | allowed | allowed |

A nonvisual implementer can implement a written specification with `requires = []` even on a visually oriented project, because nothing forces a visual declaration onto work that does not declare one.

An omitted `requires` field means no requirements were declared, so a submit inherits the requirements already recorded against the session.
An explicitly empty `requires` (`--requires ""`, `requires: []`) means no requirements and clears whatever the session inherited.
The CLI, the MCP schema and the exported pi client all preserve that absent versus empty distinction.

No vendor name, tool name, or model substring grants a capability.
A profile named after a vision model with `image_input = "unknown"` still fails a `visual_review` requirement.

## Tool, model, and spending

With a selected profile, its declared tool and model are the launch identity.
A conflicting explicit `--model` or tool is an error rather than a silent switch.
For `session_spawn` and `session_fork` the profile model is the default when no explicit model is supplied, and an explicit model outside the selected profile is refused.

The existing `allowed_models` spending allowlist stays independent and is applied to the resolved model on every launch path, including the MCP fork path that previously skipped it, and to a constrained submit so a now-disallowed recorded model cannot keep consuming paid work.
There is no automatic model upgrade, no preference-based launch, no fallback launch, and no widening of `allowed_models`.
`qol sessions capability` lists the configured profiles sorted by preference with their declarations and a `spend_allowed` flag so a caller can choose within budget.

## Identity binding and persistence

The resolved assignment is immutable for a dispatch and is serialized as `agent_assignment` with `agent_status` (`constrained` or `unconstrained`) and `evidence_basis = "configuration_declared"`.

- Policy is checked before any launch begins, and the resolved assignment is included in the delivered prompt as a directive.
- A background launch embeds the admitted task in the launch argv, so the new session token does not exist yet and nothing can be persisted against it before the terminal starts; the spawn ledger binds the token and the pending-round checkpoint carries the assignment immediately after the session is discovered, which is still before any later round text is delivered.
- A foreground submit (the non-background spawn delivery path and `session_submit`) persists the checkpoint with the resolved assignment before the task text is sent.
- A fork writes its record with the assignment before launching and finalizes the record with the session token afterwards.
- A submit inherits the assignment recorded against the session when fields are omitted, revalidates it against current policy, persists the resolved assignment on the checkpoint, and only then dispatches the task.

Live reuse consults the recorded identity bound to the live session token, not a stale key-only record and not the caller's requested model.
Reusing a managed session reports its recorded model even on a legacy call, and a conflicting request is refused instead of reporting a model switch that did not happen.
A token-bound record without an assignment still supplies its recorded model to the reuse outcome and refuses a conflicting explicit model, while a session with no record at all reports no verified model.
A constrained reuse, submit, or resume against a session with no recorded assignment is refused; an unmanaged session is never relabelled with a trusted profile.
A recorded profile is revalidated against current policy on every reuse, submit and spawn-resume, so revoking or changing a profile blocks those paths; a constrained resume also requires the prior profile to match the fresh selection, and a resume with a recorded prior assignment revalidates the full prior tool, model, role and requirements under current policy, not just the profile name.
A constrained spawn that would resume a prior session whose recorded assignment is missing or incompatible fails with an instruction to pass `resume=false` for a fresh session.

The policy file is read fresh for each dispatch.
An MCP server that started before a configuration change does not keep the old permissions.

## Grouped preflight

A lane set resolves every lane against one policy snapshot before the first terminal is created.
The preflight evaluates the intended decision for each lane: live reuse identity against the recorded assignment, prior-record resume compatibility, launch eligibility including the model and task, and role and requirement checks.
A known policy or identity mismatch in any lane therefore creates no terminal and no pending round, including a bad second lane in a set that would reuse or resume.

Runtime failures after preflight are state changes the snapshot could not see, a competing process holding the key, or a local launch or IO failure, and the outcome reports them honestly (`lane X failed after preflight because the live session state changed; N lane(s) of this set are already live and keep running`).

## Receipts

The durable round receipt written by the watcher (`lanes/rounds/<digest>/receipt.json`) carries the checkpoint assignment, and the `session_loop_close` receipt carries `agent_assignment` plus `agent_status`.
Old receipts without the field still parse.

A receipt means the policy was checked for the assignment.
It never means the model passed its work or produced trusted visual evidence.

## Limitations

- Evidence is `configuration_declared`. A runtime model change performed inside a harness cannot be attested by this feature, and no screen parsing or guessed model detection is attempted.
- Human terminal diagnostics (`qol sessions send`, `read`, `wait`, `focus`, and `qol sessions bridge` with a task) are outside this admission boundary. The bridge command still revalidates the recorded assignment for a constrained host, but the feature does not claim to constrain arbitrary shell execution.
- Collection-only calls (`session_bridge`, `qol sessions resume`) are unchanged in admission behavior, but they report the recorded assignment so a reviewer can see what was checked at dispatch time; `qol sessions next` rows carry `agent_status` and `agent_assignment` for the same reason.

## External clients

The agent tool contract is owned by `tools/qol-cli/src/commands/sessions/contract.rs`.
Regenerate the pi client from that contract with:

```bash
qol sessions export pi
```

The cross-repository generated client refresh for other harnesses uses the same command and the same contract.
