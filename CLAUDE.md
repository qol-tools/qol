# CLAUDE.md (Shared Rules)

## PRs are opt-in. Default is commit-direct-to-main.

Default all work (tests, refactors, fixes, features, configs, docs) direct to `main`. Open a PR, issue, or ADR **only when explicitly asked**; never offer one as a fallback. Mechanics: `qol-workflow:git-trees`.

## Standards Evolution

Found a practice better than the current standard? Encode it as a skill or rule **before** applying it, so the next session sees it. Place it with `qol-workflow:standards-evolution`.

## Code Style

- **No comments** - keep the codebase comment-free
- **Conventional commits** - `feat:`, `fix:`, `refactor:`, `test:`, etc.
- **Short commit messages** - one-liners, no co-authors
- **Atomic commits** - one logical change per commit; each commit compiles and represents a working state
- **Amend mistakes** - fix a flaw in a previous unpushed commit by amending, not a "fix the fix" commit
- **No dead code warnings** - remove unused code or gate it behind a feature flag
- **Always build and test** - build, test, fmt, and clippy with real command output before reporting done or committing; never assume
- **No pushing unless asked** - commit locally; push only when explicitly told

## Single Responsibility Patterns

- **Describe without AND** - if you need "and" to describe a function, split it
- **Extract by abstraction level** - high-level orchestration shouldn't contain low-level detail
- **Input → Transform → Output** - a function gathers input, transforms data, or produces output; don't mix I/O with business logic
- **Command/Query separation** - a function changes state or returns data, not both

## Type Safety Patterns

- **Newtypes for domain concepts** - `struct PluginId(String)`, not raw `String`
- **Make invalid states unrepresentable** - model state machines with enums, not bool flags plus optional fields
- **Parse, don't validate** - parse into validated types at boundaries, use those types internally
- **Exhaustive matching** - match all enum variants explicitly (no `_ =>`), so the compiler catches new variants

## Complexity Thresholds (Deep Modules)

- **Deep modules over shallow** - hide complexity behind simple APIs; a function does meaningful work, not just delegate
- **Max 50 lines per function** - split beyond this only if it yields a genuinely reusable abstraction
- **Nesting is acceptable** - `for` + `if`, `match` in a loop, early returns are fine; extract only when it clarifies intent or creates reusable logic
- **One concern per function** - don't mix state management, navigation, and action dispatch
- **Avoid shallow extractions** - don't extract a single-use helper when the inline version is equally clear
- **Clean interfaces** - public APIs obvious and hard to misuse; internal complexity is fine behind a clean interface

## Test Style

- **Table-driven tests** - consolidate similar cases into one test with a cases array:
  ```rust
  let cases = [("input1", expected1), ("input2", expected2)];
  for (input, expected) in cases {
      assert_eq!(func(input), expected, "input: {}", input);
  }
  ```
- **Context in assertions** - include identifying info so a failed iteration is debuggable
- **AAA pattern** - Arrange/Act/Assert comments for larger, complex tests; omit for table-driven and one-liner tests
- **Generic test data** - abstract paths (`/a/b/c/foo`) and names (`foo`, `bar`), not real apps or personal paths
- **No tests for thin wrappers** - don't test a function that only calls already-tested functions
- **Meaningful assertions** - verify the actual value, never just `result.is_ok()`
- **Descriptive names** - snake_case that says what is tested: `version_parsing_extracts_parts`, not `test_parse`

## Frontend Architecture (for repos with UI)

- **Functional and declarative** - pure render functions, no imperative DOM manipulation
- **Data-driven** - UI derived from state, not manually synchronized
- **Single responsibility** - split logical chunks into focused modules
- **Type safety** - define data structures explicitly, validate API responses
- **Scalability** - design for N items, not hardcoded assumptions
- **Keyboard-first** - every interaction reachable by keyboard; design keyboard flow first, mouse/hover second
