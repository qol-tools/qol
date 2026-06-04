# CLAUDE.md (Shared Rules)

These rules apply to everything in this repository.

## PRs are opt-in. Default is commit-direct-to-main.

Solo workspace - no async team to coordinate with via PR. **Default everything direct to `main`** (tests, refactors, fixes, features, configs, docs). Edit, commit, push. The user reviews on `main`, not via PR UI.

**Open a PR ONLY when the user explicitly asks for one** ("open a PR", "make a PR", "I want a diff-review", "kickoff TRAY-N"). Never offer "or open a PR" as a fallback option. The choice is fix-now-direct or not-now. Issues / ADRs follow the same rule: explicit ask only. See `qol-workflow:git-trees` for mechanics.

## Standards Evolution (read first)

When you discover a practice that one-ups the current standard (a better library, pattern, tool, workflow, etc.), **STOP and encode it as a skill or CLAUDE.md rule FIRST, then apply it.** Drift kills consistency: improvements applied ad-hoc are improvements the next session can't see. The rule lives where the area lives:

- testing pattern → `qol-tray:qol-apps-testing`
- Rust idiom → `qol-langs:rust-conventions`
- workflow / branch / PR / commit → `qol-workflow:*`
- cross-cutting principle (this file)

Trigger on phrases like "I noticed", "this is better than", "we should be using", "modern best practice", "10x'er pattern". The meta-skill is `qol-workflow:standards-evolution` - load it before encoding so you stay non-redundant and pick the right home.

## Code Style

- **No comments** - Code removed all comments; keep it that way
- **Conventional commits** - Use format: `feat:`, `fix:`, `refactor:`, `test:`, etc.
- **Short commit messages** - One-liners, no fluff, no co-authors
- **Atomic commits** - One logical change per commit. Split distinct changes (bug fix, refactor, tests) into separate commits. Each commit must compile and represent a working state.
- **Amend mistakes** - If a refactor fixes a mistake from the previous unpushed commit, amend or squash. Don't create separate "fix the fix" commits.
- **No dead code warnings** - Remove unused code or gate with feature flags
- **Always build and test** - Build, test, fmt, and clippy your changes and verify with real command output before reporting them done or committing. Never assume; check. (`cargo fmt` is also enforced by the `.githooks/pre-commit` gate.)
- **No pushing unless asked** - Commit locally but do not push until the end of a session or when explicitly told. Pushing triggers CI and should be batched.

## Single Responsibility Patterns

- **Describe without AND** - If you need "and" to describe a function, split it
- **Extract by abstraction level** - High-level orchestration shouldn't contain low-level details
- **Input → Transform → Output** - Functions should be one of: gather input, transform data, produce output. Don't mix I/O with business logic.
- **Command/Query separation** - Functions either change state OR return data, not both

## Type Safety Patterns

- **Newtypes for domain concepts** - Use `struct PluginId(String)` not raw `String`
- **Make invalid states unrepresentable** - Use enums to model state machines, not bool flags with optional fields
- **Parse, don't validate** - Parse into validated types at boundaries, use those types internally
- **Exhaustive matching** - Always match all enum variants explicitly (no `_ =>`), compiler catches new variants

## Complexity Thresholds (Deep Modules Philosophy)

Inspired by "A Philosophy of Software Design" by John Ousterhout:

- **Deep modules over shallow** - Hide complexity behind simple, clean APIs. A function should do meaningful work, not just delegate. Prefer fewer functions that do more over many trivial wrappers.
- **Max 50 lines per function** - Split beyond this, but only if it creates genuinely reusable abstractions
- **Nesting is acceptable** - Common idioms like `for` + `if`, `match` in loop, early returns are fine. Extract helpers only when it genuinely clarifies intent or creates reusable logic.
- **One concern per function** - Don't mix state management, navigation, and action dispatch
- **Avoid shallow extractions** - Don't create `ensure_parent_dir()` if it's only called once and the inline version is equally clear. Extract when the abstraction has a meaningful name and hides real complexity.
- **Clean interfaces** - Public APIs should be obvious and hard to misuse. Internal complexity is fine if the interface is clean.

## Test Style

- **Table-driven tests** - Consolidate similar test cases into a single test with a cases array:
  ```rust
  let cases = [("input1", expected1), ("input2", expected2)];
  for (input, expected) in cases {
      assert_eq!(func(input), expected, "input: {}", input);
  }
  ```
- **Context in assertions** - Always include identifying info in assertion messages for debugging failed iterations
- **AAA pattern** - Use Arrange/Act/Assert comments for larger, complex tests where structure aids clarity. Omit for table-driven tests and simple one-liner tests where AAA would be redundant.
- **Generic test data** - Use abstract paths like `/a/b/c/foo` not personal-looking paths like `/home/user/documents/file.txt`. Use generic names (`foo`, `bar`) not real app names (`firefox`, `discord`).
- **No tests for thin wrappers** - If a function just calls already-tested functions, don't test it separately.
- **Meaningful assertions** - Tests must verify specific behavior. Never just assert `result.is_ok()` - check the actual value.
- **Descriptive names** - Use snake_case that explains what is being tested: `version_parsing_extracts_parts` not `test_parse`

## Frontend Architecture (for repos with UI)

- **Functional and declarative** - Pure render functions, no imperative DOM manipulation
- **Data-driven** - UI derived from state, not manually synchronized
- **Single responsibility** - Split logical chunks into focused modules
- **Type safety** - Define data structures explicitly, validate API responses
- **Scalability** - Design for N items, not hardcoded assumptions
- **Keyboard-first** - All interactions MUST be accessible via keyboard. Design keyboard flow first, then add mouse/hover as secondary.
